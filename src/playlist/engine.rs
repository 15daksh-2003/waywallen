use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::Mutex;
use tokio::task::JoinHandle;

use crate::error::Result;
use crate::playlist::cursor::PlaylistCursor;
use crate::playlist::shared;
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
    shared: shared::Sessions,
}

impl Engine {
    pub fn new() -> Self {
        Engine::default()
    }

    pub async fn owned_display_ids(&self) -> Vec<DisplayId> {
        let mut ids = self.inner.lock().await.keys().copied().collect::<Vec<_>>();
        ids.extend(self.shared.owned_display_ids().await);
        ids
    }

    pub async fn is_owned(&self, display_id: DisplayId) -> bool {
        self.inner.lock().await.contains_key(&display_id)
            || self.shared.is_owned(display_id).await
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
        if targets.is_empty() {
            return Ok(());
        }

        if targets.len() >= 2 {
            for &did in &targets {
                app.playlists.inner.lock().await.remove(&did);
            }
            app.playlists
                .shared
                .activate(
                    app,
                    &targets,
                    playlist_id,
                    mode,
                    interval,
                    items,
                    resume,
                    first_frame_timeout,
                )
                .await?;
            for &did in &targets {
                persist_assignment(app, did, Some(playlist_id)).await;
            }
            app.events
                .publish(crate::events::GlobalEvent::PlaylistChanged);
            return Ok(());
        }

        // Shared across all targets so displays activated together shuffle in sync.
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(1)
            .max(1);

        app.playlists.shared.release(&targets).await;
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

    pub async fn attach_shared(
        app: &Arc<AppState>,
        display_id: DisplayId,
        playlist_id: i64,
    ) -> Result<bool> {
        if app.playlists.is_owned(display_id).await {
            return Ok(true);
        }
        let attached = app
            .playlists
            .shared
            .attach(app, display_id, playlist_id)
            .await?;
        if attached {
            persist_assignment(app, display_id, Some(playlist_id)).await;
            app.events
                .publish(crate::events::GlobalEvent::PlaylistChanged);
        }
        Ok(attached)
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

        let members = Arc::new(Mutex::new(vec![display_id]));
        let task = tokio::spawn(run_playlist_rotator(
            app.clone(),
            cursor.clone(),
            deadline.clone(),
            members,
            false,
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
        app.playlists.shared.release(&targets).await;
        for did in &targets {
            app.playlists.inner.lock().await.remove(did);
            persist_assignment(app, *did, None).await;
        }
        app.events
            .publish(crate::events::GlobalEvent::PlaylistChanged);
        Ok(())
    }

    pub async fn step(app: &Arc<AppState>, display_id: DisplayId, delta: i32) -> Result<()> {
        if app.playlists.shared.step(app, display_id, delta).await? {
            return Ok(());
        }
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
        if app
            .playlists
            .shared
            .jump_to(app, playlist_id, entry_id)
            .await?
        {
            return Ok(());
        }
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
        out.extend(self.shared.status().await);
        out
    }

    pub async fn drop_display(&self, display_id: DisplayId) {
        self.shared.drop_display(display_id).await;
        self.inner.lock().await.remove(&display_id);
    }

    pub async fn deactivate_for_playlist(app: &Arc<AppState>, playlist_id: i64) {
        let shared_members = app.playlists.shared.deactivate_playlist(playlist_id).await;
        for did in &shared_members {
            persist_assignment(app, *did, None).await;
        }
        let owned: Vec<DisplayId> = {
            let map = app.playlists.inner.lock().await;
            map.iter()
                .filter(|(_, r)| r.playlist_id == playlist_id)
                .map(|(d, _)| *d)
                .collect()
        };
        if owned.is_empty() && shared_members.is_empty() {
            return;
        }
        if !owned.is_empty() {
            let _ = Self::deactivate(app, &owned).await;
        } else if !shared_members.is_empty() {
            app.events
                .publish(crate::events::GlobalEvent::PlaylistChanged);
        }
    }

    pub async fn rebuild_for_playlist(app: &Arc<AppState>, playlist_id: i64) {
        let pl = match plrepo::get(&app.db, playlist_id).await {
            Ok(Some(p)) => p,
            _ => return,
        };
        let mode: Mode = pl.mode.into();
        let interval = pl.interval_secs as u32;
        let items = resolve::resolve(app, playlist_id).await.unwrap_or_default();

        match app
            .playlists
            .shared
            .rebuild(app, playlist_id, mode, interval, items.clone())
            .await
        {
            Ok(None) => {}
            Ok(Some(cleared)) if !cleared.is_empty() => {
                for did in cleared {
                    persist_assignment(app, did, None).await;
                }
                app.events
                    .publish(crate::events::GlobalEvent::PlaylistChanged);
                return;
            }
            Ok(Some(_)) => return,
            Err(e) => {
                log::warn!("shared playlist rebuild {playlist_id} failed: {e:#}");
                return;
            }
        }

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
        if app.playlists.shared.set_interval(playlist_id, secs).await {
            return;
        }
        let map = app.playlists.inner.lock().await;
        for (_, r) in map.iter() {
            if r.playlist_id == playlist_id {
                r.handle.set_interval(secs);
            }
        }
    }
}

pub(crate) async fn run_playlist_rotator(
    app: Arc<AppState>,
    cursor: Arc<Mutex<PlaylistCursor>>,
    deadline: Arc<std::sync::Mutex<Option<std::time::Instant>>>,
    targets: Arc<Mutex<Vec<DisplayId>>>,
    shared_renderer: bool,
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
                    let members = targets.lock().await.clone();
                    if members.is_empty() {
                        continue;
                    }
                    let next = cursor.lock().await.next(1);
                    if let Some(id) = next {
                        let result = if shared_renderer {
                            crate::control::apply_wallpaper_shared_to_displays(
                                &app, &id, &members, None,
                            )
                            .await
                        } else {
                            crate::control::apply_wallpaper_to_displays(&app, &id, &members).await
                        };
                        if let Err(e) = result {
                            log::warn!(
                                "playlist rotator displays={members:?} apply failed: {e:#}"
                            );
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

pub(crate) async fn display_settings_key(
    app: &Arc<AppState>,
    display_id: DisplayId,
) -> Option<String> {
    app.router
        .snapshot_displays()
        .await
        .into_iter()
        .find(|d| d.id == display_id)
        .map(|d| d.instance_id.unwrap_or(d.name))
}
