use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

use anyhow::anyhow;

use crate::error::{Error, Result};
use crate::events::GlobalEvent;
use crate::model::repo;
use crate::plugin::renderer_registry::PluginPackageMeta;
use crate::wallframe::renderer_manager;
use crate::wallframe::scheduler::DisplayId;
use crate::DaemonContext;

use super::reload;

pub struct PluginInstallResult {
    pub plugin_id: String,
    pub needs_restart: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ActivePluginIdentity {
    version: String,
    system: bool,
}

fn active_plugin_identity(
    packages: &[PluginPackageMeta],
    plugin_id: &str,
) -> Option<ActivePluginIdentity> {
    packages
        .iter()
        .find(|package| package.id == plugin_id)
        .map(|package| ActivePluginIdentity {
            version: package.version.clone(),
            system: package.system,
        })
}

async fn affected_display_plan(
    app: &Arc<DaemonContext>,
    renderer_ids: &[renderer_manager::RendererId],
) -> BTreeMap<String, Vec<DisplayId>> {
    let affected: BTreeSet<_> = renderer_ids.iter().cloned().collect();
    let mut plan: BTreeMap<String, BTreeSet<DisplayId>> = BTreeMap::new();

    for display in app.router.snapshot_displays().await {
        if !display
            .links
            .iter()
            .any(|link| affected.contains(&link.renderer_id))
        {
            continue;
        }
        let key = display.instance_id.as_deref().unwrap_or(&display.name);
        let Some(wallpaper_id) = app.settings.resolved_last_wallpaper(key) else {
            log::warn!(
                "plugin install: display {} has no last wallpaper; cannot restart renderer link",
                display.name
            );
            continue;
        };
        plan.entry(wallpaper_id).or_default().insert(display.id);
    }
    plan.into_iter()
        .map(|(wallpaper_id, display_ids)| (wallpaper_id, display_ids.into_iter().collect()))
        .collect()
}

async fn restart_affected_renderers(
    app: &Arc<DaemonContext>,
    renderer_ids: Vec<renderer_manager::RendererId>,
) -> Result<()> {
    let plan = affected_display_plan(app, &renderer_ids).await;
    for (wallpaper_id, display_ids) in plan {
        if display_ids.is_empty() {
            continue;
        }
        crate::application::apply_wallpaper_to_displays_with_first_frame_timeout(
            app,
            &wallpaper_id,
            &display_ids,
            crate::application::APPLY_FIRST_FRAME_TIMEOUT,
            crate::application::ApplySource::PluginRestart,
        )
        .await?;
    }
    for renderer_id in renderer_ids {
        if app.renderer_manager.get(&renderer_id).await.is_some() {
            app.router
                .stop_renderers_orderly(&[renderer_id], Duration::from_secs(1))
                .await;
        }
    }
    Ok(())
}

fn spawn_affected_renderer_restart(
    app: &Arc<DaemonContext>,
    plugin_id: String,
    renderer_ids: Vec<renderer_manager::RendererId>,
) {
    if renderer_ids.is_empty() {
        return;
    }
    let app = app.clone();
    let tasks = app.tasks.clone();
    let task_name = format!("plugin-restart/{plugin_id}");
    tasks.spawn_async(crate::tasks::TaskKind::Generic, task_name, async move {
        if let Err(error) = restart_affected_renderers(&app, renderer_ids).await {
            let error = format!("{error:#}");
            log::warn!("plugin restart failed for {plugin_id}: {error}");
            app.events
                .publish(GlobalEvent::PluginRestartFailed { plugin_id, error });
        }
        Ok(())
    });
}

fn spawn_source_refresh(app: &Arc<DaemonContext>, plugin_id: &str) {
    let app = app.clone();
    let tasks = app.tasks.clone();
    let task_name = format!("plugin-refresh/{plugin_id}");
    tasks.spawn_async_unique(
        crate::tasks::TaskKind::Generic,
        "source/plugin-refresh",
        task_name,
        async move {
            let skip_refresh = repo::list_libraries(&app.db)
                .await
                .map(|libraries| libraries.is_empty())
                .unwrap_or(false);
            if !skip_refresh {
                crate::application::refresh_sources(&app)
                    .await
                    .map(|_| ())?;
            }
            Ok(())
        },
    );
}

pub async fn install_plugin_archive(
    app: &Arc<DaemonContext>,
    zip_path: String,
) -> Result<PluginInstallResult> {
    let _guard = app.plugin_mutation.lock().await;
    let plugin_id =
        tokio::task::spawn_blocking(move || crate::plugin::installer::install_zip(&zip_path))
            .await
            .map_err(|error| Error::Internal(anyhow!("install join: {error}")))??;
    let old_active = active_plugin_identity(&app.plugins.read().await, &plugin_id);
    let old_renderer_ids = app
        .renderer_manager
        .live_renderer_ids_by_plugin_id(&plugin_id)
        .await;
    let plugin_roots = app.plugin_roots.clone();
    let plugin_scan = tokio::task::spawn_blocking(move || {
        crate::plugin::renderer_registry::scan_plugin_roots(plugin_roots.as_slice())
    })
    .await
    .map_err(|error| Error::Internal(anyhow!("plugin scan join: {error}")))?;

    let new_packages = reload::apply_scan(app, plugin_scan, &plugin_id).await?;
    let new_active = active_plugin_identity(&new_packages, &plugin_id);
    let active_user_install = new_active.as_ref().is_some_and(|plugin| !plugin.system);
    let should_restart =
        !old_renderer_ids.is_empty() && (active_user_install || old_active != new_active);
    if should_restart {
        spawn_affected_renderer_restart(app, plugin_id.clone(), old_renderer_ids);
    }
    spawn_source_refresh(app, &plugin_id);
    Ok(PluginInstallResult {
        plugin_id,
        needs_restart: false,
    })
}
