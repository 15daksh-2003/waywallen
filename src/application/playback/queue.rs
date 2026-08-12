use std::sync::Arc;

use crate::error::{Error, Result};
use crate::model::repo;
use crate::playback::{Mode, RotationConfig};
use crate::DaemonContext;

use super::{apply_wallpaper_by_id, apply_wallpaper_to_displays};

pub async fn step_pick(app: &Arc<DaemonContext>, delta: i32) -> Result<String> {
    use crate::model::repo::QueueRow;
    use crate::playback::Mode;

    let (filters, logics) = app.settings.global().wallpaper_queue_filter();
    let sorts = crate::settings::WallpaperSortRuleState::vec_to_catalog(
        &app.settings.global().wallpaper_sorts,
    );
    let mode = app.queue.lock().await.mode;

    let entry_id: String = match mode {
        Mode::Sequential => step_sequential(app, delta, &filters, &logics, &sorts).await?,
        Mode::Random => {
            let exclude = app.queue.lock().await.last_db_id;
            let row: QueueRow = repo::random_item_by_filter(&app.db, &filters, &logics, exclude)
                .await?
                .ok_or_else(|| Error::FailedPrecondition("queue is empty".into()))?;
            bridge_to_entry_id(&row)
        }
        Mode::Shuffle => {
            let row = step_shuffle(app, &filters, &logics, delta).await?;
            bridge_to_entry_id(&row)
        }
    };
    Ok(entry_id)
}

pub async fn step(app: &Arc<DaemonContext>, delta: i32) -> Result<String> {
    let entry_id = step_pick(app, delta).await?;
    apply_wallpaper_by_id(app, &entry_id).await?;
    app.rotation.kick();
    Ok(entry_id)
}

/// Walk the sorted+filtered entry list by `delta`, wrapping with `rem_euclid`.
/// If the current entry is absent, start at the first or last item.
async fn step_sequential(
    app: &Arc<DaemonContext>,
    delta: i32,
    filters: &[crate::catalog::FilterRule],
    logics: &[crate::catalog::FilterLogic],
    sorts: &[crate::catalog::SortRule],
) -> Result<String> {
    let ordered =
        crate::application::catalog::ordered_entry_ids(app, filters, logics, sorts).await?;
    if ordered.is_empty() {
        return Err(Error::FailedPrecondition("queue is empty".into()));
    }
    let len = ordered.len() as i64;
    let current = app.queue.lock().await.current.clone();
    let cur_idx = current
        .as_deref()
        .and_then(|c| ordered.iter().position(|id| id == c));
    let next_idx = match cur_idx {
        Some(i) => ((i as i64) + delta as i64).rem_euclid(len) as usize,
        None => {
            if delta >= 0 {
                0
            } else {
                (len - 1) as usize
            }
        }
    };
    Ok(ordered[next_idx].clone())
}

/// Bridge a DB queue row to the `WallpaperApply` argument. Identity is
/// the DB `item.id`, which the row already carries.
fn bridge_to_entry_id(row: &repo::QueueRow) -> String {
    row.item_id.to_string()
}

async fn step_shuffle(
    app: &Arc<DaemonContext>,
    filters: &[crate::catalog::FilterRule],
    logics: &[crate::catalog::FilterLogic],
    delta: i32,
) -> Result<repo::QueueRow> {
    // Lock-free preflight: snapshot whether the round is empty so we
    // can fetch ids without holding the queue mutex through the DB call.
    let need_round = {
        let q = app.queue.lock().await;
        q.shuffle_round.is_empty()
    };
    if need_round {
        let ids = repo::list_item_ids_by_filter(&app.db, filters, logics).await?;
        if ids.is_empty() {
            return Err(Error::FailedPrecondition("queue is empty".into()));
        }
        let mut q = app.queue.lock().await;
        let avoid = q.last_db_id;
        q.build_shuffle_round(ids, avoid, 0);
        let pick = q.shuffle_round[0];
        q.shuffle_pos = 0;
        drop(q);
        return repo::get_item_with_library(&app.db, pick)
            .await?
            .ok_or_else(|| Error::FailedPrecondition("queue is empty".into()));
    }

    let pick = {
        let mut q = app.queue.lock().await;
        let len = q.shuffle_round.len() as i64;
        let raw = q.shuffle_pos as i64 + delta as i64;
        if raw >= len || raw < 0 {
            // Wrap: rebuild the round.
            let avoid = q.last_db_id;
            let target = if raw >= len {
                0usize
            } else {
                q.shuffle_round.len().saturating_sub(1)
            };
            let candidates = q.shuffle_round.clone();
            q.build_shuffle_round(candidates, avoid, target);
            q.shuffle_pos = target;
        } else {
            q.shuffle_pos = raw as usize;
        }
        q.shuffle_round[q.shuffle_pos]
    };

    repo::get_item_with_library(&app.db, pick)
        .await?
        .ok_or_else(|| Error::FailedPrecondition("queue is empty".into()))
}

/// Set the rotation mode on the active playlist and persist it to settings.
pub async fn set_mode(app: &Arc<DaemonContext>, mode: Mode) {
    app.queue.lock().await.set_mode(mode);
    app.settings.update(|s| {
        s.global.queue_mode = mode.as_str().to_owned();
    });
    crate::system::dbus::notify_queue_mode_changed(app).await;
    crate::system::tray::dbusmenu::notify_menu_changed(app).await;
}

/// Set the auto-rotation interval in seconds; `0` disables rotation.
/// Updates the live rotator and persists the cadence to settings.
pub async fn set_rotation_interval(app: &Arc<DaemonContext>, secs: u32) {
    app.rotation.set_interval(secs);
    app.settings.update(|s| {
        s.global.rotation_secs = secs;
    });
    crate::system::dbus::notify_rotation_secs_changed(app).await;
    crate::system::tray::dbusmenu::notify_menu_changed(app).await;
}

/// Convenience: flip shuffle on/off without exposing the [`Mode`]
/// enum to D-Bus / WS callers. `true` → Shuffle, `false` → Sequential.
pub async fn set_shuffle(app: &Arc<DaemonContext>, on: bool) {
    let mode = if on { Mode::Shuffle } else { Mode::Sequential };
    set_mode(app, mode).await;
}

/// Snapshot of the live playlist state for status reporting.
#[derive(Debug, Clone)]
pub struct QueueStatus {
    pub active_id: Option<i64>,
    pub mode: String,
    pub interval_secs: u32,
    pub current: Option<String>,
    pub position: Option<u32>,
    pub count: u32,
    pub is_smart: bool,
}

pub async fn queue_status(app: &Arc<DaemonContext>) -> QueueStatus {
    let (filters, logics) = app.settings.global().wallpaper_queue_filter();
    let count = repo::count_items_by_filter(&app.db, &filters, &logics)
        .await
        .unwrap_or(0) as u32;
    // "smart" reflects user-authored filter rules only; the quick
    // skip-type toggles narrow the queue but don't make it a playlist.
    let is_smart = !app.settings.global().wallpaper_filter.filters.is_empty();
    let g = app.queue.lock().await;
    QueueStatus {
        active_id: None,
        mode: g.mode.as_str().to_owned(),
        interval_secs: app.rotation.interval(),
        current: g.current.clone(),
        position: None,
        count,
        is_smart,
    }
}

/// Restore queue mode, rotation cadence, and manual audio state from disk. Idempotent.
pub async fn run_restore(app: &Arc<DaemonContext>) -> Result<()> {
    use crate::events::GlobalEvent;

    let g = app.settings.global();
    if let Some(mode) = crate::playback::Mode::from_str(&g.queue_mode) {
        app.queue.lock().await.set_mode(mode);
    }
    if g.rotation_secs > 0 {
        app.rotation.set_interval(g.rotation_secs);
    }
    app.router.set_manual_mute(g.manual_muted).await;

    app.events.publish(GlobalEvent::RestoreApplied(None));
    Ok(())
}

/// Auto-rotation task body.
/// Reads live cadence from a watch channel and applies the next wallpaper.
pub async fn run_rotator(
    app: Arc<DaemonContext>,
    mut rx: tokio::sync::watch::Receiver<RotationConfig>,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) {
    log::info!("playlist rotator started");
    loop {
        let cfg = *rx.borrow();
        if cfg.interval_secs == 0 {
            tokio::select! {
                _ = rx.changed() => continue,
                changed = shutdown.changed() => {
                    if changed.is_err() || *shutdown.borrow() { break; }
                }
            }
        } else {
            let dur = std::time::Duration::from_secs(cfg.interval_secs as u64);
            tokio::select! {
                _ = tokio::time::sleep(dur) => {
                    if rx.borrow().interval_secs == 0 {
                        continue;
                    }
                    let owned = app.playlists.owned_display_ids().await;
                    let all: Vec<crate::wallframe::scheduler::DisplayId> = app
                        .router
                        .snapshot_displays()
                        .await
                        .into_iter()
                        .map(|d| d.id)
                        .collect();
                    let unowned: Vec<_> =
                        all.into_iter().filter(|d| !owned.contains(d)).collect();
                    if unowned.is_empty() {
                        continue;
                    }
                    match step_pick(&app, 1).await {
                        Ok(id) => {
                            if let Err(e) =
                                apply_wallpaper_to_displays(&app, &id, &unowned).await
                            {
                                log::warn!("rotator apply failed: {e:#}");
                            }
                        }
                        Err(e) => log::warn!("rotator tick step failed: {e:#}"),
                    }
                }
                _ = rx.changed() => continue,
                changed = shutdown.changed() => {
                    if changed.is_err() || *shutdown.borrow() { break; }
                }
            }
        }
    }
    log::info!("playlist rotator exited");
}
