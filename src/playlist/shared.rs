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
        let session = match self.attach_start(display_id, playlist_id).await {
            AttachStart::AlreadyOwned => return Ok(true),
            AttachStart::NoSession => return Ok(false),
            AttachStart::Session(session) => session,
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
            return Ok(Some(self.deactivate_playlist(playlist_id).await));
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

    async fn attach_start(&self, display_id: DisplayId, playlist_id: i64) -> AttachStart {
        if self.is_owned(display_id).await {
            return AttachStart::AlreadyOwned;
        }
        let session = {
            let by_playlist = self.by_playlist.lock().await;
            by_playlist.get(&playlist_id).cloned()
        };
        match session {
            Some(session) => AttachStart::Session(session),
            None => AttachStart::NoSession,
        }
    }
}

enum AttachStart {
    AlreadyOwned,
    NoSession,
    Session(Arc<SharedSession>),
}

#[cfg(test)]
impl Sessions {
    pub(crate) async fn seed_for_test(
        &self,
        playlist_id: i64,
        members: &[DisplayId],
        items: Vec<String>,
        current: Option<String>,
        interval: u32,
    ) {
        let mut cursor = PlaylistCursor::new(items, Mode::Sequential, 1);
        if let Some(id) = current.as_ref() {
            cursor.set_current(id);
        } else {
            cursor.first();
        }
        let (handle, rx) = make_handle();
        handle.set_interval(interval);
        let task = tokio::spawn(async move {
            let _rx = rx;
            std::future::pending::<()>().await
        });
        let session = Arc::new(SharedSession {
            playlist_id,
            cursor: Arc::new(Mutex::new(cursor)),
            handle,
            deadline: Arc::new(std::sync::Mutex::new(None)),
            members: Arc::new(Mutex::new(members.to_vec())),
            task,
        });
        self.by_playlist
            .lock()
            .await
            .insert(playlist_id, Arc::clone(&session));
        let mut by_display = self.by_display.lock().await;
        for &did in members {
            by_display.insert(did, playlist_id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn is_owned_and_release_last_member_drops_session() {
        let sessions = Sessions::default();
        sessions
            .seed_for_test(1, &[10, 20], vec!["a".into()], Some("a".into()), 30)
            .await;
        assert!(sessions.is_owned(10).await);
        assert!(sessions.is_owned(20).await);
        sessions.release(&[10]).await;
        assert!(!sessions.is_owned(10).await);
        assert!(sessions.is_owned(20).await);
        assert!(sessions.by_playlist.lock().await.contains_key(&1));
        sessions.release(&[20]).await;
        assert!(!sessions.is_owned(20).await);
        assert!(sessions.by_playlist.lock().await.is_empty());
    }

    #[tokio::test]
    async fn attach_returns_false_without_session() {
        let sessions = Sessions::default();
        assert!(matches!(
            sessions.attach_start(5, 99).await,
            AttachStart::NoSession
        ));
    }

    #[tokio::test]
    async fn attach_noops_when_already_owned() {
        let sessions = Sessions::default();
        sessions
            .seed_for_test(1, &[5], vec!["a".into()], Some("a".into()), 30)
            .await;
        assert!(matches!(
            sessions.attach_start(5, 1).await,
            AttachStart::AlreadyOwned
        ));
    }

    #[tokio::test]
    async fn deactivate_playlist_returns_members() {
        let sessions = Sessions::default();
        sessions
            .seed_for_test(3, &[1, 2], vec!["a".into()], Some("a".into()), 10)
            .await;
        let members = sessions.deactivate_playlist(3).await;
        assert_eq!(members, vec![1, 2]);
        assert!(!sessions.is_owned(1).await);
        assert!(!sessions.is_owned(2).await);
        assert!(sessions.deactivate_playlist(3).await.is_empty());
    }

    #[tokio::test]
    async fn status_fans_out_same_cursor_to_all_members() {
        let sessions = Sessions::default();
        sessions
            .seed_for_test(
                8,
                &[100, 200],
                vec!["a".into(), "b".into()],
                Some("b".into()),
                45,
            )
            .await;
        let status = sessions.status().await;
        assert_eq!(status.len(), 2);
        for s in &status {
            assert_eq!(s.active_id, 8);
            assert_eq!(s.current_id.as_deref(), Some("b"));
            assert_eq!(s.position, 1);
            assert_eq!(s.count, 2);
            assert_eq!(s.interval_secs, 45);
        }
        let ids: Vec<_> = status.iter().map(|s| s.display_id).collect();
        assert!(ids.contains(&100));
        assert!(ids.contains(&200));
    }

    #[tokio::test]
    async fn rebuild_empty_items_releases_session() {
        let sessions = Sessions::default();
        sessions
            .seed_for_test(4, &[7, 8], vec!["a".into()], Some("a".into()), 30)
            .await;
        let exists = sessions.by_playlist.lock().await.contains_key(&4);
        assert!(exists);
        let members = sessions.deactivate_playlist(4).await;
        assert_eq!(members, vec![7, 8]);
        assert!(sessions.by_playlist.lock().await.is_empty());
        assert!(matches!(
            sessions.attach_start(7, 4).await,
            AttachStart::NoSession
        ));
    }

    #[tokio::test]
    async fn set_interval_updates_existing_session() {
        let sessions = Sessions::default();
        assert!(!sessions.set_interval(1, 60).await);
        sessions
            .seed_for_test(1, &[9], vec!["a".into()], Some("a".into()), 30)
            .await;
        assert!(sessions.set_interval(1, 60).await);
        let session = sessions.by_playlist.lock().await.get(&1).cloned().unwrap();
        assert_eq!(session.handle.interval(), 60);
    }
}
