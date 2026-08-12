use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use crate::error::{Error, Result};
use crate::model::repo::playlists as repository;
use crate::playback::playlist::{Activation, ApplyPort, ApplyRequest, ApplySharing, Definition};
use crate::playback::Mode;
use crate::wallframe::scheduler::DisplayId;
use crate::DaemonContext;

use super::resolve;

fn apply_port(app: &Arc<DaemonContext>) -> ApplyPort {
    let app = app.clone();
    ApplyPort::new(move |request: ApplyRequest| {
        let app = app.clone();
        async move {
            match request.sharing {
                ApplySharing::Independent => match request.first_frame_timeout {
                    Some(timeout) => {
                        super::apply_wallpaper_to_displays_with_first_frame_timeout(
                            &app,
                            &request.entry_id,
                            &request.display_ids,
                            timeout,
                        )
                        .await?;
                    }
                    None => {
                        super::apply_wallpaper_to_displays(
                            &app,
                            &request.entry_id,
                            &request.display_ids,
                        )
                        .await?;
                    }
                },
                ApplySharing::Shared => {
                    super::apply_wallpaper_shared_to_displays(
                        &app,
                        &request.entry_id,
                        &request.display_ids,
                        request.first_frame_timeout,
                    )
                    .await?;
                }
            }
            Ok(())
        }
    })
}

async fn definition(app: &Arc<DaemonContext>, playlist_id: i64) -> Result<Definition> {
    let playlist = repository::get(&app.db, playlist_id)
        .await?
        .ok_or_else(|| Error::PlaylistNotFound(playlist_id.to_string()))?;
    Ok(Definition {
        id: playlist.id,
        mode: playlist.mode,
        interval_secs: playlist.interval_secs,
        items: resolve::resolve(app, playlist_id).await?,
    })
}

async fn display_settings_key(app: &Arc<DaemonContext>, display_id: DisplayId) -> Option<String> {
    app.router
        .snapshot_displays()
        .await
        .into_iter()
        .find(|display| display.id == display_id)
        .map(|display| display.instance_id.unwrap_or(display.name))
}

async fn resume_ids(
    app: &Arc<DaemonContext>,
    definition: &Definition,
    display_ids: &[DisplayId],
) -> HashMap<DisplayId, String> {
    if definition.mode == Mode::Random {
        return HashMap::new();
    }
    let mut ids = HashMap::new();
    for display_id in display_ids {
        let Some(key) = display_settings_key(app, *display_id).await else {
            continue;
        };
        let Some(entry_id) = app
            .settings
            .display_prefs(&key)
            .and_then(|prefs| prefs.last_wallpaper)
            .filter(|entry_id| definition.items.contains(entry_id))
        else {
            continue;
        };
        ids.insert(*display_id, entry_id);
    }
    ids
}

async fn persist_assignments(
    app: &Arc<DaemonContext>,
    display_ids: &[DisplayId],
    playlist_id: Option<i64>,
) {
    let mut keys = Vec::new();
    for display_id in display_ids {
        if let Some(key) = display_settings_key(app, *display_id).await {
            keys.push(key);
        }
    }
    app.settings.update(|settings| {
        for key in &keys {
            settings
                .displays
                .entry(key.clone())
                .or_default()
                .active_playlist_id = playlist_id;
        }
    });
    app.settings.flush_now().await;
}

fn publish_changed(app: &Arc<DaemonContext>) {
    app.events
        .publish(crate::events::GlobalEvent::PlaylistChanged);
}

pub async fn activate(
    app: &Arc<DaemonContext>,
    display_ids: &[DisplayId],
    playlist_id: i64,
) -> Result<()> {
    activate_inner(app, display_ids, playlist_id, false, None).await
}

pub async fn activate_resuming_with_first_frame_timeout(
    app: &Arc<DaemonContext>,
    display_ids: &[DisplayId],
    playlist_id: i64,
    timeout: Duration,
) -> Result<()> {
    activate_inner(app, display_ids, playlist_id, true, Some(timeout)).await
}

async fn activate_inner(
    app: &Arc<DaemonContext>,
    display_ids: &[DisplayId],
    playlist_id: i64,
    resume: bool,
    first_frame_timeout: Option<Duration>,
) -> Result<()> {
    let definition = definition(app, playlist_id).await?;
    let targets = if display_ids.is_empty() {
        app.router
            .snapshot_displays()
            .await
            .into_iter()
            .map(|display| display.id)
            .collect()
    } else {
        display_ids.to_vec()
    };
    let resume_by_display = if resume {
        resume_ids(app, &definition, &targets).await
    } else {
        HashMap::new()
    };
    app.playlists
        .activate(
            Activation {
                definition,
                display_ids: targets.clone(),
                resume_by_display,
                first_frame_timeout,
            },
            apply_port(app),
            app.shutdown_subscribe(),
        )
        .await?;
    persist_assignments(app, &targets, Some(playlist_id)).await;
    publish_changed(app);
    Ok(())
}

pub async fn attach_shared(
    app: &Arc<DaemonContext>,
    display_id: DisplayId,
    playlist_id: i64,
) -> Result<bool> {
    let attached = app
        .playlists
        .attach_shared(
            display_id,
            playlist_id,
            crate::application::APPLY_FIRST_FRAME_TIMEOUT,
            apply_port(app),
        )
        .await?;
    if attached {
        persist_assignments(app, &[display_id], Some(playlist_id)).await;
        publish_changed(app);
    }
    Ok(attached)
}

pub async fn deactivate(app: &Arc<DaemonContext>, display_ids: &[DisplayId]) -> Result<()> {
    let targets = if display_ids.is_empty() {
        app.playlists.owned_display_ids().await
    } else {
        display_ids.to_vec()
    };
    app.playlists.deactivate(&targets).await;
    persist_assignments(app, &targets, None).await;
    publish_changed(app);
    Ok(())
}

pub async fn deactivate_for_playlist(app: &Arc<DaemonContext>, playlist_id: i64) {
    let displays = app.playlists.deactivate_playlist(playlist_id).await;
    if displays.is_empty() {
        return;
    }
    persist_assignments(app, &displays, None).await;
    publish_changed(app);
}

pub async fn jump_to(app: &Arc<DaemonContext>, playlist_id: i64, entry_id: &str) -> Result<()> {
    app.playlists
        .jump_to(playlist_id, entry_id, apply_port(app))
        .await
}

pub async fn rebuild_for_playlist(app: &Arc<DaemonContext>, playlist_id: i64) {
    let Ok(definition) = definition(app, playlist_id).await else {
        return;
    };
    match app.playlists.rebuild(definition, apply_port(app)).await {
        Ok(cleared) if !cleared.is_empty() => {
            persist_assignments(app, &cleared, None).await;
            publish_changed(app);
        }
        Ok(_) => {}
        Err(error) => log::warn!("playlist rebuild {playlist_id} failed: {error:#}"),
    }
}

pub async fn set_interval_for_playlist(
    app: &Arc<DaemonContext>,
    playlist_id: i64,
    interval_secs: u32,
) {
    app.playlists.set_interval(playlist_id, interval_secs).await;
}
