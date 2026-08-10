use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::Mutex;
use tokio::task::JoinHandle;

use crate::error::Result;
use crate::playlist::cursor::PlaylistCursor;
use crate::playlist::engine::{self, DisplayStatus};
use crate::queue::rotator::{make_handle, RotationHandle};
use crate::queue::Mode;
use crate::scheduler::DisplayId;
use crate::AppState;

struct SharedSession {
    playlist_id: i64,
    cursor: Arc<Mutex<PlaylistCursor>>,
    handle: RotationHandle,
    deadline: Arc<std::sync::Mutex<Option<std::time::Instant>>>,
    members: Arc<Mutex<Vec<DisplayId>>>,
    task: JoinHandle<()>,
}

impl Drop for SharedSession {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[derive(Default)]
pub struct Sessions {
    by_playlist: Mutex<HashMap<i64, Arc<SharedSession>>>,
    by_display: Mutex<HashMap<DisplayId, i64>>,
}

impl Sessions {
    pub async fn owned_display_ids(&self) -> Vec<DisplayId> {
        self.by_display.lock().await.keys().copied().collect()
    }

    pub async fn is_owned(&self, display_id: DisplayId) -> bool {
        self.by_display.lock().await.contains_key(&display_id)
    }

    pub async fn activate(
        &self,
        app: &Arc<AppState>,
        targets: &[DisplayId],
        playlist_id: i64,
        mode: Mode,
        interval: u32,
        items: Vec<String>,
        resume: bool,
        first_frame_timeout: Option<std::time::Duration>,
    ) -> Result<()> {
        if targets.is_empty() {
            return Ok(());
        }

        self.release(targets).await;

        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(1)
            .max(1);

        let resume_id = if resume && mode != Mode::Random {
            let mut found = None;
            for &did in targets {
                let Some(key) = engine::display_settings_key(app, did).await else {
                    continue;
                };
                if let Some(id) = app
                    .settings
                    .display_prefs(&key)
                    .and_then(|p| p.last_wallpaper)
                    .filter(|id| items.iter().any(|x| x == id))
                {
                    found = Some(id);
                    break;
                }
            }
            found
        } else {
            None
        };

        let cursor = Arc::new(Mutex::new(PlaylistCursor::new(items, mode, seed)));
        let (handle, rx) = make_handle();
        handle.set_interval(interval);
        let deadline = Arc::new(std::sync::Mutex::new(None));
        let members = Arc::new(Mutex::new(targets.to_vec()));

        let first = {
            let mut c = cursor.lock().await;
            match resume_id {
                Some(id) => {
                    c.set_current(&id);
                    Some(id)
                }
                None => c.first(),
            }
        };
        if let Some(id) = first {
            crate::control::apply_wallpaper_shared_to_displays(
                app,
                &id,
                targets,
                first_frame_timeout,
            )
            .await?;
        }

        let task = tokio::spawn(engine::run_playlist_rotator(
            app.clone(),
            cursor.clone(),
            deadline.clone(),
            members.clone(),
            true,
            rx,
            app.shutdown_subscribe(),
        ));

        let session = Arc::new(SharedSession {
            playlist_id,
            cursor,
            handle,
            deadline,
            members,
            task,
        });

        {
            let mut by_playlist = self.by_playlist.lock().await;
            by_playlist.insert(playlist_id, Arc::clone(&session));
        }
        {
            let mut by_display = self.by_display.lock().await;
            for &did in targets {
                by_display.insert(did, playlist_id);
            }
        }
        Ok(())
    }

    pub async fn attach(
        &self,
        app: &Arc<AppState>,
        display_id: DisplayId,
        playlist_id: i64,
    ) -> Result<bool> {
        if self.is_owned(display_id).await {
            return Ok(true);
        }
        let session = {
            let by_playlist = self.by_playlist.lock().await;
            by_playlist.get(&playlist_id).cloned()
        };
        let Some(session) = session else {
            return Ok(false);
        };

        let current = session.cursor.lock().await.current.clone();
        if let Some(id) = current {
            crate::control::apply_wallpaper_shared_to_displays(
                app,
                &id,
                &[display_id],
                Some(crate::control::APPLY_FIRST_FRAME_TIMEOUT),
            )
            .await?;
        }

        session.members.lock().await.push(display_id);
        self.by_display
            .lock()
            .await
            .insert(display_id, playlist_id);
        Ok(true)
    }

    pub async fn release(&self, display_ids: &[DisplayId]) {
        let mut by_display = self.by_display.lock().await;
        let mut touched = Vec::new();
        for &did in display_ids {
            if let Some(pid) = by_display.remove(&did) {
                touched.push((did, pid));
            }
        }
        drop(by_display);

        let mut by_playlist = self.by_playlist.lock().await;
        for (did, pid) in touched {
            let Some(session) = by_playlist.get(&pid).cloned() else {
                continue;
            };
            session.members.lock().await.retain(|d| *d != did);
            if session.members.lock().await.is_empty() {
                by_playlist.remove(&pid);
            }
        }
    }

    pub async fn step(
        &self,
        app: &Arc<AppState>,
        display_id: DisplayId,
        delta: i32,
    ) -> Result<bool> {
        let session = self.session_for_display(display_id).await;
        let Some(session) = session else {
            return Ok(false);
        };
        let next = session.cursor.lock().await.next(delta);
        if let Some(id) = next {
            let targets = session.members.lock().await.clone();
            if !targets.is_empty() {
                crate::control::apply_wallpaper_shared_to_displays(app, &id, &targets, None)
                    .await?;
            }
            session.handle.kick();
        }
        Ok(true)
    }

    pub async fn jump_to(
        &self,
        app: &Arc<AppState>,
        playlist_id: i64,
        entry_id: &str,
    ) -> Result<bool> {
        let session = {
            let by_playlist = self.by_playlist.lock().await;
            by_playlist.get(&playlist_id).cloned()
        };
        let Some(session) = session else {
            return Ok(false);
        };
        if !session.cursor.lock().await.set_current(entry_id) {
            return Ok(true);
        }
        let targets = session.members.lock().await.clone();
        if !targets.is_empty() {
            crate::control::apply_wallpaper_shared_to_displays(app, entry_id, &targets, None)
                .await?;
        }
        session.handle.kick();
        Ok(true)
    }

    pub async fn status(&self) -> Vec<DisplayStatus> {
        let sessions: Vec<Arc<SharedSession>> = {
            let by_playlist = self.by_playlist.lock().await;
            by_playlist.values().cloned().collect()
        };
        let now = std::time::Instant::now();
        let mut out = Vec::new();
        for session in sessions {
            let remaining_secs = match *session.deadline.lock().unwrap() {
                Some(t) => t.saturating_duration_since(now).as_secs() as u32,
                None => 0,
            };
            let interval_secs = session.handle.interval();
            let (mode, current_id, position, count) = {
                let c = session.cursor.lock().await;
                (c.mode, c.current.clone(), c.pos as u32, c.len() as u32)
            };
            let members = session.members.lock().await.clone();
            for did in members {
                out.push(DisplayStatus {
                    display_id: did,
                    active_id: session.playlist_id,
                    mode,
                    interval_secs,
                    current_id: current_id.clone(),
                    position,
                    count,
                    remaining_secs,
                });
            }
        }
        out
    }

    pub async fn drop_display(&self, display_id: DisplayId) {
        self.release(&[display_id]).await;
    }

    pub async fn deactivate_playlist(&self, playlist_id: i64) -> Vec<DisplayId> {
        let members = {
            let by_playlist = self.by_playlist.lock().await;
            match by_playlist.get(&playlist_id) {
                Some(s) => s.members.lock().await.clone(),
                None => return Vec::new(),
            }
        };
        self.release(&members).await;
        members
    }

    pub async fn rebuild(
        &self,
        app: &Arc<AppState>,
        playlist_id: i64,
        mode: Mode,
        interval: u32,
        items: Vec<String>,
    ) -> Result<Option<Vec<DisplayId>>> {
        let session = {
            let by_playlist = self.by_playlist.lock().await;
            by_playlist.get(&playlist_id).cloned()
        };
        let Some(session) = session else {
            return Ok(None);
        };
        if items.is_empty() {
            let members = session.members.lock().await.clone();
            self.release(&members).await;
            return Ok(Some(members));
        }

        let (apply_id, need_apply) = {
            let mut c = session.cursor.lock().await;
            let cur = c.current.clone();
            c.items = items;
            c.mode = mode;
            match cur {
                Some(id) if c.items.iter().any(|x| x == &id) => {
                    c.set_current(&id);
                    (id, false)
                }
                _ => (c.first().unwrap_or_default(), true),
            }
        };
        session.handle.set_interval(interval);
        if need_apply && !apply_id.is_empty() {
            let targets = session.members.lock().await.clone();
            if !targets.is_empty() {
                crate::control::apply_wallpaper_shared_to_displays(app, &apply_id, &targets, None)
                    .await?;
            }
            session.handle.kick();
        }
        Ok(Some(Vec::new()))
    }

    pub async fn set_interval(&self, playlist_id: i64, secs: u32) -> bool {
        let by_playlist = self.by_playlist.lock().await;
        let Some(session) = by_playlist.get(&playlist_id) else {
            return false;
        };
        session.handle.set_interval(secs);
        true
    }

    async fn session_for_display(&self, display_id: DisplayId) -> Option<Arc<SharedSession>> {
        let pid = self.by_display.lock().await.get(&display_id).copied()?;
        self.by_playlist.lock().await.get(&pid).cloned()
    }
}
