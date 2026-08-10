use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::Mutex;
use tokio::task::JoinHandle;

use crate::error::Result;
use crate::playlist::cursor::PlaylistCursor;
use crate::playlist::{repo as plrepo, resolve};
use crate::queue::rotator::{make_handle, RotationConfig, RotationHandle};
use crate::queue::Mode;
use crate::scheduler::DisplayId;
use crate::AppState;

struct DisplayRotation {
    playlist_id: i64,
    cursor: Arc<Mutex<PlaylistCursor>>,
    handle: RotationHandle,
    deadline: Arc<std::sync::Mutex<Option<std::time::Instant>>>,
    task: JoinHandle<()>,
}

impl Drop for DisplayRotation {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[derive(Debug, Clone)]
pub struct DisplayStatus {
    pub display_id: DisplayId,
    pub active_id: i64,
    pub mode: Mode,
    pub interval_secs: u32,
    pub current_id: Option<String>,
    pub position: u32,
    pub count: u32,
    pub remaining_secs: u32,
}

#[derive(Default)]
pub struct Engine {
    inner: Mutex<HashMap<DisplayId, DisplayRotation>>,
}

impl Engine {
    pub fn new() -> Self {
        Engine::default()
    }

    pub async fn owned_display_ids(&self) -> Vec<DisplayId> {
        self.inner.lock().await.keys().copied().collect()
    }

    pub async fn is_owned(&self, display_id: DisplayId) -> bool {
        self.inner.lock().await.contains_key(&display_id)
    }

    pub async fn activate(
        app: &Arc<AppState>,
        display_ids: &[DisplayId],
        playlist_id: i64,
    ) -> Result<()> {
        Self::activate_inner(app, display_ids, playlist_id, false, None).await
    }

    pub async fn activate_resuming(
        app: &Arc<AppState>,
        display_ids: &[DisplayId],
        playlist_id: i64,
    ) -> Result<()> {
        Self::activate_inner(app, display_ids, playlist_id, true, None).await
    }

    pub async fn activate_resuming_with_first_frame_timeout(
        app: &Arc<AppState>,
        display_ids: &[DisplayId],
        playlist_id: i64,
        timeout: std::time::Duration,
    ) -> Result<()> {
        Self::activate_inner(app, display_ids, playlist_id, true, Some(timeout)).await
    }

    async fn activate_inner(
        app: &Arc<AppState>,
        display_ids: &[DisplayId],
        playlist_id: i64,
        resume: bool,
        first_frame_timeout: Option<std::time::Duration>,
    ) -> Result<()> {
        let pl = plrepo::get(&app.db, playlist_id)
            .await?
            .ok_or_else(|| crate::error::Error::PlaylistNotFound(playlist_id.to_string()))?;
        let mode: Mode = pl.mode.into();
        let interval = pl.interval_secs as u32;
        let items = resolve::resolve(app, playlist_id).await?;
        if items.is_empty() {
            return Err(crate::error::Error::PlaylistInvalid(
                "playlist has no wallpapers".into(),
            ));
        }

        let targets = if display_ids.is_empty() {
            app.router
                .snapshot_displays()
                .await
                .into_iter()
                .map(|d| d.id)
                .collect::<Vec<_>>()
        } else {
            display_ids.to_vec()
        };

        // Shared across all targets so displays activated together shuffle in sync.
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(1)
            .max(1);

        for did in targets {
            Self::activate_one(
                app,
                did,
                playlist_id,
                mode,
                interval,
                items.clone(),
                resume,
                first_frame_timeout,
                seed,
            )
            .await?;
            persist_assignment(app, did, Some(playlist_id)).await;
        }
        app.events
            .publish(crate::events::GlobalEvent::PlaylistChanged);
        Ok(())
    }

    async fn activate_one(
        app: &Arc<AppState>,
        display_id: DisplayId,
        playlist_id: i64,
        mode: Mode,
        interval: u32,
        items: Vec<String>,
        resume: bool,
        first_frame_timeout: Option<std::time::Duration>,
        seed: u64,
    ) -> Result<()> {
        {
            app.playlists.inner.lock().await.remove(&display_id);
        }
        let resume_id = if resume && mode != Mode::Random {
            match display_settings_key(app, display_id).await {
                Some(key) => app
                    .settings
                    .display_prefs(&key)
                    .and_then(|p| p.last_wallpaper)
                    .filter(|id| items.iter().any(|x| x == id)),
                None => None,
            }
        } else {
            None
        };
        let cursor = Arc::new(Mutex::new(PlaylistCursor::new(items, mode, seed)));
        let (handle, rx) = make_handle();
        handle.set_interval(interval);
        let deadline = Arc::new(std::sync::Mutex::new(None));

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
            match first_frame_timeout {
                Some(timeout) => {
                    crate::control::apply_wallpaper_to_displays_with_first_frame_timeout(
                        app,
                        &id,
                        &[display_id],
                        timeout,
                    )
                    .await?;
                }
                None => {
                    let _ =
                        crate::control::apply_wallpaper_to_displays(app, &id, &[display_id]).await;
                }
            }
        }

        let task = tokio::spawn(run_display_rotator(
            app.clone(),
            display_id,
            cursor.clone(),
            deadline.clone(),
            rx,
            app.shutdown_subscribe(),
        ));

        app.playlists.inner.lock().await.insert(
            display_id,
            DisplayRotation {
                playlist_id,
                cursor,
                handle,
                deadline,
                task,
            },
        );
        Ok(())
    }

    pub async fn deactivate(app: &Arc<AppState>, display_ids: &[DisplayId]) -> Result<()> {
        let targets = if display_ids.is_empty() {
            app.playlists.owned_display_ids().await
        } else {
            display_ids.to_vec()
        };
        for did in targets {
            app.playlists.inner.lock().await.remove(&did);
            persist_assignment(app, did, None).await;
        }
        app.events
            .publish(crate::events::GlobalEvent::PlaylistChanged);
        Ok(())
    }

    pub async fn step(app: &Arc<AppState>, display_id: DisplayId, delta: i32) -> Result<()> {
        let cursor = {
            let map = app.playlists.inner.lock().await;
            map.get(&display_id).map(|r| r.cursor.clone())
        };
        let Some(cursor) = cursor else {
            return Ok(());
        };
        let next = cursor.lock().await.next(delta);
        if let Some(id) = next {
            crate::control::apply_wallpaper_to_displays(app, &id, &[display_id]).await?;
            let map = app.playlists.inner.lock().await;
            if let Some(r) = map.get(&display_id) {
                r.handle.kick();
            }
        }
        Ok(())
    }

    pub async fn jump_to(app: &Arc<AppState>, playlist_id: i64, entry_id: &str) -> Result<()> {
        let displays: Vec<(DisplayId, Arc<Mutex<PlaylistCursor>>)> = {
            let map = app.playlists.inner.lock().await;
            map.iter()
                .filter(|(_, r)| r.playlist_id == playlist_id)
                .map(|(d, r)| (*d, r.cursor.clone()))
                .collect()
        };
        for (did, cursor) in displays {
            let ok = cursor.lock().await.set_current(entry_id);
            if !ok {
                continue;
            }
            crate::control::apply_wallpaper_to_displays(app, entry_id, &[did]).await?;
            let map = app.playlists.inner.lock().await;
            if let Some(r) = map.get(&did) {
                r.handle.kick();
            }
        }
        Ok(())
    }

    pub async fn status(&self) -> Vec<DisplayStatus> {
        type Snap = (
            DisplayId,
            i64,
            Arc<Mutex<PlaylistCursor>>,
            u32,
            Arc<std::sync::Mutex<Option<std::time::Instant>>>,
        );
        let snapshot: Vec<Snap> = {
            let map = self.inner.lock().await;
            map.iter()
                .map(|(did, rot)| {
                    (
                        *did,
                        rot.playlist_id,
                        rot.cursor.clone(),
                        rot.handle.interval(),
                        rot.deadline.clone(),
                    )
                })
                .collect()
        };
        let now = std::time::Instant::now();
        let mut out = Vec::with_capacity(snapshot.len());
        for (did, playlist_id, cursor, interval_secs, deadline) in snapshot {
            let remaining_secs = match *deadline.lock().unwrap() {
                Some(t) => t.saturating_duration_since(now).as_secs() as u32,
                None => 0,
            };
            let c = cursor.lock().await;
            out.push(DisplayStatus {
                display_id: did,
                active_id: playlist_id,
                mode: c.mode,
                interval_secs,
                current_id: c.current.clone(),
                position: c.pos as u32,
                count: c.len() as u32,
                remaining_secs,
            });
        }
        out
    }

    pub async fn drop_display(&self, display_id: DisplayId) {
        self.inner.lock().await.remove(&display_id);
    }

    pub async fn deactivate_for_playlist(app: &Arc<AppState>, playlist_id: i64) {
        let owned: Vec<DisplayId> = {
            let map = app.playlists.inner.lock().await;
            map.iter()
                .filter(|(_, r)| r.playlist_id == playlist_id)
                .map(|(d, _)| *d)
                .collect()
        };
        if owned.is_empty() {
            return;
        }
        let _ = Self::deactivate(app, &owned).await;
    }

    pub async fn rebuild_for_playlist(app: &Arc<AppState>, playlist_id: i64) {
        type Bound = (DisplayId, Arc<Mutex<PlaylistCursor>>, RotationHandle);
        let affected: Vec<Bound> = {
            let map = app.playlists.inner.lock().await;
            map.iter()
                .filter(|(_, r)| r.playlist_id == playlist_id)
                .map(|(d, r)| (*d, r.cursor.clone(), r.handle.clone()))
                .collect()
        };
        if affected.is_empty() {
            return;
        }

        let pl = match plrepo::get(&app.db, playlist_id).await {
            Ok(Some(p)) => p,
            _ => return,
        };
        let mode: Mode = pl.mode.into();
        let interval = pl.interval_secs as u32;
        let items = resolve::resolve(app, playlist_id).await.unwrap_or_default();

        if items.is_empty() {
            let ids: Vec<DisplayId> = affected.iter().map(|(d, _, _)| *d).collect();
            let _ = Self::deactivate(app, &ids).await;
            return;
        }

        for (did, cursor, handle) in affected {
            let (apply_id, need_apply) = {
                let mut c = cursor.lock().await;
                let cur = c.current.clone();
                c.items = items.clone();
                c.mode = mode;
                match cur {
                    Some(id) if items.iter().any(|x| x == &id) => {
                        c.set_current(&id);
                        (id, false)
                    }
                    _ => (c.first().unwrap_or_default(), true),
                }
            };
            handle.set_interval(interval);
            if need_apply && !apply_id.is_empty() {
                let _ = crate::control::apply_wallpaper_to_displays(app, &apply_id, &[did]).await;
                handle.kick();
            }
        }
    }

    pub async fn set_interval_for_playlist(app: &Arc<AppState>, playlist_id: i64, secs: u32) {
        let map = app.playlists.inner.lock().await;
        for (_, r) in map.iter() {
            if r.playlist_id == playlist_id {
                r.handle.set_interval(secs);
            }
        }
    }
}

async fn run_display_rotator(
    app: Arc<AppState>,
    display_id: DisplayId,
    cursor: Arc<Mutex<PlaylistCursor>>,
    deadline: Arc<std::sync::Mutex<Option<std::time::Instant>>>,
    mut rx: tokio::sync::watch::Receiver<RotationConfig>,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) {
    loop {
        let cfg: RotationConfig = *rx.borrow();
        if cfg.interval_secs == 0 {
            *deadline.lock().unwrap() = None;
            tokio::select! {
                _ = rx.changed() => continue,
                changed = shutdown.changed() => {
                    if changed.is_err() || *shutdown.borrow() { break; }
                }
            }
        } else {
            let dur = std::time::Duration::from_secs(cfg.interval_secs as u64);
            *deadline.lock().unwrap() = Some(std::time::Instant::now() + dur);
            tokio::select! {
                _ = tokio::time::sleep(dur) => {
                    if rx.borrow().interval_secs == 0 { continue; }
                    let next = cursor.lock().await.next(1);
                    if let Some(id) = next {
                        if let Err(e) =
                            crate::control::apply_wallpaper_to_displays(&app, &id, &[display_id]).await
                        {
                            log::warn!("playlist rotator display={display_id} apply failed: {e:#}");
                        }
                    }
                }
                _ = rx.changed() => continue,
                changed = shutdown.changed() => {
                    if changed.is_err() || *shutdown.borrow() { break; }
                }
            }
        }
    }
}

async fn persist_assignment(app: &Arc<AppState>, display_id: DisplayId, playlist_id: Option<i64>) {
    let key = match display_settings_key(app, display_id).await {
        Some(k) => k,
        None => return,
    };
    app.settings.update(|s| {
        let prefs = s.displays.entry(key.clone()).or_default();
        prefs.active_playlist_id = playlist_id;
    });
    app.settings.flush_now().await;
}

async fn display_settings_key(app: &Arc<AppState>, display_id: DisplayId) -> Option<String> {
    app.router
        .snapshot_displays()
        .await
        .into_iter()
        .find(|d| d.id == display_id)
        .map(|d| d.instance_id.unwrap_or(d.name))
}
