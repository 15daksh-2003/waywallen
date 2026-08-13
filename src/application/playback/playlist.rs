use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use crate::application::ApplySource;
use crate::error::{Error, Result};
use crate::model::repo::playlists as repository;
use crate::playback::playlist::{
    Activation, ApplyPort, ApplyRequest, ApplySharing, ApplySource as PlaylistApplySource,
    Definition,
};
use crate::playback::Mode;
use crate::wallframe::scheduler::DisplayId;
use crate::DaemonContext;

use super::resolve;

#[derive(Clone, Copy)]
enum AutoAttachUpdate {
    Preserve,
    Inherit,
    Disable,
    ClearGlobal,
}

fn apply_source(source: PlaylistApplySource, activation_source: ApplySource) -> ApplySource {
    match source {
        PlaylistApplySource::Activation => activation_source,
        PlaylistApplySource::Rotation => ApplySource::PlaylistRotation,
        PlaylistApplySource::Jump => ApplySource::UserPlaylistJump,
        PlaylistApplySource::Rebuild => ApplySource::PlaylistRebuild,
        PlaylistApplySource::Attach => ApplySource::PlaylistAttach,
    }
}

fn apply_port(app: &Arc<DaemonContext>, activation_source: ApplySource) -> ApplyPort {
    let app = app.clone();
    ApplyPort::new(move |request: ApplyRequest| {
        let app = app.clone();
        async move {
            let source = apply_source(request.source, activation_source);
            match request.sharing {
                ApplySharing::Independent => match request.first_frame_timeout {
                    Some(timeout) => {
                        super::apply_wallpaper_to_displays_with_first_frame_timeout(
                            &app,
                            &request.entry_id,
                            &request.display_ids,
                            timeout,
                            source,
                        )
                        .await?;
                    }
                    None => {
                        super::apply_wallpaper_to_displays(
                            &app,
                            &request.entry_id,
                            &request.display_ids,
                            source,
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
                        source,
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
    auto_attach: AutoAttachUpdate,
) {
    let mut keys = Vec::new();
    for display_id in display_ids {
        if let Some(key) = display_settings_key(app, *display_id).await {
            keys.push(key);
        }
    }
    app.settings.update(|settings| {
        for key in &keys {
            let prefs = settings.displays.entry(key.clone()).or_default();
            prefs.active_playlist_id = playlist_id;
            match auto_attach {
                AutoAttachUpdate::Preserve => {}
                AutoAttachUpdate::Inherit => prefs.playlist_auto_attach_disabled = false,
                AutoAttachUpdate::Disable => prefs.playlist_auto_attach_disabled = true,
                AutoAttachUpdate::ClearGlobal => prefs.playlist_auto_attach_disabled = false,
            }
        }
        if matches!(auto_attach, AutoAttachUpdate::ClearGlobal) {
            settings.global.auto_attach_playlist_id = None;
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
    activate_inner(
        app,
        display_ids,
        playlist_id,
        false,
        None,
        ApplySource::UserPlaylistActivation,
    )
    .await
}

pub async fn activate_resuming_with_first_frame_timeout(
    app: &Arc<DaemonContext>,
    display_ids: &[DisplayId],
    playlist_id: i64,
    timeout: Duration,
) -> Result<()> {
    activate_inner(
        app,
        display_ids,
        playlist_id,
        true,
        Some(timeout),
        ApplySource::StartupRestore,
    )
    .await
}

async fn activate_inner(
    app: &Arc<DaemonContext>,
    display_ids: &[DisplayId],
    playlist_id: i64,
    resume: bool,
    first_frame_timeout: Option<Duration>,
    activation_source: ApplySource,
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
            apply_port(app, activation_source),
            app.shutdown_subscribe(),
        )
        .await?;
    persist_assignments(app, &targets, Some(playlist_id), AutoAttachUpdate::Inherit).await;
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
            apply_port(app, ApplySource::UserPlaylistActivation),
        )
        .await?;
    if attached {
        persist_assignments(
            app,
            &[display_id],
            Some(playlist_id),
            AutoAttachUpdate::Inherit,
        )
        .await;
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
    persist_assignments(app, &targets, None, AutoAttachUpdate::Preserve).await;
    publish_changed(app);
    Ok(())
}

pub async fn deactivate_for_playlist(app: &Arc<DaemonContext>, playlist_id: i64) {
    let displays = app.playlists.deactivate_playlist(playlist_id).await;
    if displays.is_empty() {
        return;
    }
    persist_assignments(app, &displays, None, AutoAttachUpdate::Preserve).await;
    publish_changed(app);
}

pub async fn jump_to(app: &Arc<DaemonContext>, playlist_id: i64, entry_id: &str) -> Result<()> {
    app.playlists
        .jump_to(
            playlist_id,
            entry_id,
            apply_port(app, ApplySource::UserPlaylistActivation),
        )
        .await
}

pub async fn rebuild_for_playlist(app: &Arc<DaemonContext>, playlist_id: i64) {
    let Ok(definition) = definition(app, playlist_id).await else {
        return;
    };
    match app
        .playlists
        .rebuild(
            definition,
            apply_port(app, ApplySource::UserPlaylistActivation),
        )
        .await
    {
        Ok(cleared) if !cleared.is_empty() => {
            persist_assignments(app, &cleared, None, AutoAttachUpdate::Preserve).await;
            publish_changed(app);
        }
        Ok(_) => {}
        Err(error) => log::warn!("playlist rebuild {playlist_id} failed: {error:#}"),
    }
}

pub(super) async fn stop_for_wallpaper_override(
    app: &Arc<DaemonContext>,
    display_ids: &[DisplayId],
    all_displays: bool,
) -> Result<Vec<crate::application::StoppedPlaylist>> {
    if display_ids.is_empty() {
        return Ok(Vec::new());
    }

    let targets: HashSet<DisplayId> = display_ids.iter().copied().collect();
    let mut groups: BTreeMap<i64, Vec<DisplayId>> = BTreeMap::new();
    for status in app.playlists.status().await {
        if targets.contains(&status.display_id) {
            groups
                .entry(status.active_id)
                .or_default()
                .push(status.display_id);
        }
    }
    for ids in groups.values_mut() {
        ids.sort_unstable();
        ids.dedup();
    }

    let mut stopped = Vec::with_capacity(groups.len());
    for (id, display_ids) in groups {
        let name = repository::get(&app.db, id)
            .await?
            .map(|playlist| playlist.name)
            .unwrap_or_default();
        stopped.push(crate::application::StoppedPlaylist {
            id,
            name,
            all_displays: all_displays && display_ids.len() == targets.len(),
            display_ids,
        });
    }

    app.playlists.deactivate(display_ids).await;
    persist_assignments(
        app,
        display_ids,
        None,
        if all_displays {
            AutoAttachUpdate::ClearGlobal
        } else {
            AutoAttachUpdate::Disable
        },
    )
    .await;
    if !stopped.is_empty() {
        publish_changed(app);
    }
    Ok(stopped)
}

pub async fn set_interval_for_playlist(
    app: &Arc<DaemonContext>,
    playlist_id: i64,
    interval_secs: u32,
) {
    app.playlists.set_interval(playlist_id, interval_secs).await;
}
