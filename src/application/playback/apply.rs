use std::sync::Arc;
use std::time::Duration;

use anyhow::anyhow;
use ashpd::desktop::wallpaper::{SetOn, WallpaperRequest};

use crate::application::{
    should_duplicate_renderers, ApplyActivation, ApplyRequest, ApplyResult, RendererSharingPolicy,
};
use crate::catalog::entry::WallpaperEntry;
use crate::error::{Error, Result};
use crate::model::repo;
use crate::plugin::renderer_registry::RendererDef;
use crate::wallframe::renderer_manager;
use crate::wallframe::scheduler::DisplayId;
use crate::DaemonContext;

/// Apply a wallpaper by id to every registered display.
/// Supersedes any in-flight global apply task.
pub async fn apply_wallpaper_by_id(app: &Arc<DaemonContext>, id: &str) -> Result<ApplyResult> {
    let app_clone = app.clone();
    let id_owned = id.to_string();
    let (tx, rx) = tokio::sync::oneshot::channel::<Result<ApplyResult>>();
    app.tasks.spawn_async_unique(
        crate::tasks::TaskKind::Apply,
        "apply/global",
        format!("apply/{id_owned}"),
        async move {
            let res = apply_wallpaper(&app_clone, &id_owned, ApplyRequest::default()).await;
            // If the receiver is gone the caller already moved on (or
            // was itself cancelled); silently drop the result.
            let _ = tx.send(res);
            Ok(())
        },
    );
    rx.await
        .map_err(|_| Error::Internal(anyhow!("apply task superseded or cancelled")))?
}

/// Apply a wallpaper to a specific display subset.
/// Hot-plug recall uses this without cancelling global apply work.
pub async fn apply_wallpaper_to_displays(
    app: &Arc<DaemonContext>,
    id: &str,
    target: &[DisplayId],
) -> Result<ApplyResult> {
    if target.is_empty() {
        return Err(Error::Internal(anyhow!(
            "apply_wallpaper_to_displays: empty target"
        )));
    }
    apply_wallpaper(
        app,
        id,
        ApplyRequest {
            display_ids: Some(target.to_vec()),
            ..Default::default()
        },
    )
    .await
}

pub async fn apply_wallpaper_to_displays_with_first_frame_timeout(
    app: &Arc<DaemonContext>,
    id: &str,
    target: &[DisplayId],
    timeout: Duration,
) -> Result<ApplyResult> {
    if target.is_empty() {
        return Err(Error::Internal(anyhow!(
            "apply_wallpaper_to_displays: empty target"
        )));
    }
    apply_wallpaper(
        app,
        id,
        ApplyRequest {
            display_ids: Some(target.to_vec()),
            first_frame_timeout: Some(timeout),
            ..Default::default()
        },
    )
    .await
}

pub async fn apply_wallpaper_shared_to_displays(
    app: &Arc<DaemonContext>,
    id: &str,
    target: &[DisplayId],
    first_frame_timeout: Option<Duration>,
) -> Result<ApplyResult> {
    if target.is_empty() {
        return Err(Error::Internal(anyhow!(
            "apply_wallpaper_shared_to_displays: empty target"
        )));
    }
    apply_wallpaper(
        app,
        id,
        ApplyRequest {
            display_ids: Some(target.to_vec()),
            first_frame_timeout,
            sharing: RendererSharingPolicy::Shared,
            ..Default::default()
        },
    )
    .await
}

pub struct PortalApplyResult {
    pub wallpaper_id: String,
    pub uri: String,
}

/// Apply an image wallpaper through `org.freedesktop.portal.Wallpaper`.
/// The portal owns preview, prompting, and final rendering.
pub async fn apply_wallpaper_via_portal(
    app: &Arc<DaemonContext>,
    id: &str,
) -> Result<PortalApplyResult> {
    let app_clone = app.clone();
    let id_owned = id.to_string();
    let (tx, rx) = tokio::sync::oneshot::channel::<Result<PortalApplyResult>>();
    app.tasks.spawn_async_unique(
        crate::tasks::TaskKind::Apply,
        "apply/portal",
        format!("apply-portal/{id_owned}"),
        async move {
            let res = apply_via_portal_inner(&app_clone, &id_owned).await;
            let _ = tx.send(res);
            Ok(())
        },
    );
    rx.await
        .map_err(|_| Error::Internal(anyhow!("apply task superseded or cancelled")))?
}

async fn apply_via_portal_inner(app: &Arc<DaemonContext>, id: &str) -> Result<PortalApplyResult> {
    let entry = match id.parse::<i64>() {
        Ok(iid) => repo::get_entry(&app.db, iid).await?,
        Err(_) => None,
    };
    let entry = entry.ok_or_else(|| Error::WallpaperNotFound(id.to_string()))?;

    if !entry.wp_type.eq_ignore_ascii_case("image") {
        return Err(Error::WallpaperTypeNotSupported(entry.wp_type.clone()));
    }
    if !entry.resource.starts_with('/') {
        return Err(Error::InvalidArgument(format!(
            "portal apply: resource must be an absolute path, got '{}'",
            entry.resource
        )));
    }
    let uri = ashpd::url::Url::from_file_path(&entry.resource).map_err(|()| {
        Error::InvalidArgument(format!(
            "portal apply: invalid absolute path '{}'",
            entry.resource
        ))
    })?;
    let request = WallpaperRequest::default()
        .set_on(SetOn::Background)
        .show_preview(false)
        .build_uri(&uri)
        .await
        .map_err(|e| Error::PortalCallFailed(format!("SetWallpaperURI: {e}")))?;
    request
        .response()
        .map_err(|e| Error::PortalCallFailed(format!("SetWallpaperURI response: {e}")))?;

    Ok(PortalApplyResult {
        wallpaper_id: entry.item_id.to_string(),
        uri: uri.into(),
    })
}

fn resolve_renderer(
    app: &Arc<DaemonContext>,
    entry: &WallpaperEntry,
    renderer_name: Option<&str>,
) -> Result<RendererDef> {
    let registry = app.renderer_manager.registry_snapshot();
    match renderer_name {
        Some(name) if !name.is_empty() => {
            let def = registry
                .resolve_by_name(name)
                .ok_or_else(|| Error::RendererNotFound(name.to_string()))?;
            if !def.types.iter().any(|ty| ty == &entry.wp_type) {
                return Err(Error::RendererTypeMismatch {
                    renderer: name.to_string(),
                    ty: entry.wp_type.clone(),
                });
            }
            Ok(def.clone())
        }
        _ => registry
            .resolve(&entry.wp_type)
            .cloned()
            .ok_or_else(|| Error::NoRendererForType(entry.wp_type.clone())),
    }
}

async fn wait_for_apply_frame(
    app: &Arc<DaemonContext>,
    renderer_id: &str,
    timeout: Option<Duration>,
) -> Result<()> {
    let Some(timeout) = timeout else {
        return Ok(());
    };
    if let Err(e) = app
        .renderer_manager
        .wait_for_first_frame(renderer_id, timeout)
        .await
    {
        if let Err(stop_error) = app.router.kill_renderer_drop(renderer_id).await {
            log::warn!("renderer {renderer_id}: cleanup after first-frame failure: {stop_error}");
        }
        return Err(e);
    }
    Ok(())
}

async fn apply_shared_renderer(
    app: &Arc<DaemonContext>,
    spawn_req: renderer_manager::SpawnRequest,
    target_ids: &[DisplayId],
    wallpaper_layout_override: crate::catalog::properties::WallpaperLayoutOverride,
    first_frame_timeout: Option<Duration>,
) -> Result<String> {
    let renderer_id = app
        .router
        .apply_active_assignment(spawn_req, target_ids, false, wallpaper_layout_override)
        .await?;
    wait_for_apply_frame(app, &renderer_id, first_frame_timeout).await?;
    Ok(renderer_id)
}

async fn apply_duplicate_renderers(
    app: &Arc<DaemonContext>,
    spawn_req: &renderer_manager::SpawnRequest,
    target_ids: &[DisplayId],
    wallpaper_layout_override: crate::catalog::properties::WallpaperLayoutOverride,
    first_frame_timeout: Option<Duration>,
) -> Result<String> {
    let mut first_renderer_id: Option<String> = None;
    let mut first_active_renderer_id: Option<String> = None;
    for did in target_ids {
        let single = [*did];
        if app.router.displays_are_auto_stopped(&single).await {
            let renderer_id = app
                .router
                .apply_retained_assignment(
                    spawn_req.clone(),
                    spawn_req.renderer_name.clone().unwrap_or_default(),
                    &single,
                    true,
                    wallpaper_layout_override,
                )
                .await;
            first_renderer_id.get_or_insert(renderer_id);
            continue;
        }
        let renderer_id = app
            .router
            .apply_active_assignment(spawn_req.clone(), &single, true, wallpaper_layout_override)
            .await?;
        wait_for_apply_frame(app, &renderer_id, first_frame_timeout).await?;
        first_renderer_id.get_or_insert_with(|| renderer_id.clone());
        first_active_renderer_id.get_or_insert(renderer_id);
    }

    first_active_renderer_id
        .or(first_renderer_id)
        .ok_or(Error::NoDisplayRegistered)
}

/// Shared global/per-display apply core.
/// Spawns or reuses renderers, relinks displays, and persists recall state.
pub async fn apply_wallpaper(
    app: &Arc<DaemonContext>,
    id: &str,
    request: ApplyRequest,
) -> Result<ApplyResult> {
    let entry = match id.parse::<i64>() {
        Ok(iid) => repo::get_entry(&app.db, iid).await?,
        Err(_) => None,
    };
    let entry = entry.ok_or_else(|| Error::WallpaperNotFound(id.to_string()))?;

    if request.require_display && app.router.display_count().await == 0 {
        return Err(Error::NoDisplayRegistered);
    }

    let renderer = resolve_renderer(app, &entry, request.renderer_name.as_deref())?;
    let renderer_plugin_name = renderer.name.clone();
    let apply = app
        .source_manager
        .call_apply(&entry.plugin_name, &entry)
        .await?;
    let spawn_settings = app.settings.resolved_renderer_settings(&renderer);
    let (user_property_overrides, wallpaper_layout_override) =
        repo::get_wallpaper_render_properties(&app.db, entry.item_id).await?;
    let spawn_req = renderer_manager::SpawnRequest {
        wp_type: entry.wp_type.clone(),
        extras: apply.extras,
        settings: spawn_settings,
        test_pattern: false,
        renderer_name: Some(renderer_plugin_name.clone()),
        user_property_overrides,
        default_user_properties: apply.default_user_properties,
    };
    let target = request.display_ids.as_deref();
    let target_ids = app.router.registered_display_ids(target).await;
    if request.require_display && target_ids.is_empty() {
        return Err(Error::NoDisplayRegistered);
    }
    let duplicate_renderers = should_duplicate_renderers(
        app.settings.global().duplicate_renderers_for_same_wallpaper,
        !target_ids.is_empty(),
        request.sharing,
    );
    let deferred = app.router.displays_are_auto_stopped(&target_ids).await;
    let renderer_id = if deferred {
        app.router
            .apply_retained_assignment(
                spawn_req,
                renderer_plugin_name,
                &target_ids,
                duplicate_renderers,
                wallpaper_layout_override,
            )
            .await
    } else if duplicate_renderers {
        apply_duplicate_renderers(
            app,
            &spawn_req,
            &target_ids,
            wallpaper_layout_override,
            request.first_frame_timeout,
        )
        .await?
    } else {
        apply_shared_renderer(
            app,
            spawn_req,
            &target_ids,
            wallpaper_layout_override,
            request.first_frame_timeout,
        )
        .await?
    };

    {
        let mut q = app.queue.lock().await;
        q.current = Some(entry.item_id.to_string());
        // Stash the DB id so sequential / random traversal has an anchor.
        q.last_db_id = Some(entry.item_id);
    }

    let keys = app.router.display_settings_keys(&target_ids).await;
    let wp_id = entry.item_id.to_string();
    app.settings.update(|s| {
        for (_did, key) in &keys {
            let prefs = s.displays.entry(key.clone()).or_default();
            prefs.last_wallpaper = Some(wp_id.clone());
        }
        s.global.last_wallpaper = Some(wp_id);
    });
    // Flush recall state now so a crash inside the debounce window does
    // not lose the wallpaper needed by the next startup.
    app.settings.flush_now().await;
    crate::system::dbus::notify_current_wallpaper_id_changed(app).await;

    Ok(ApplyResult {
        renderer_id,
        entry,
        activation: if deferred {
            ApplyActivation::Deferred
        } else {
            ApplyActivation::Active
        },
    })
}
