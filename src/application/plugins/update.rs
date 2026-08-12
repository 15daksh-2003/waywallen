use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use anyhow::anyhow;

use crate::error::{Error, Result};
use crate::events::GlobalEvent;
use crate::plugin::update::{PluginUpdateInfo, PluginUpdateState};
use crate::DaemonContext;

use super::install::install_plugin_archive;
use crate::application::PLUGIN_UPDATE_NOTIFICATION_ID;

pub async fn check_plugin_updates(
    app: &Arc<DaemonContext>,
    plugin_id: Option<&str>,
) -> Vec<crate::plugin::update::PluginUpdateInfo> {
    let _guard = app.plugin_update_check.lock().await;
    let mut packages = app.plugins.read().await.clone();
    let plugin_id = plugin_id.filter(|id| !id.is_empty());
    if let Some(plugin_id) = plugin_id {
        packages.retain(|pkg| pkg.id == plugin_id);
    }
    let updates =
        crate::plugin::update::check_packages(&app.plugin_updates, packages, plugin_id.is_none())
            .await;
    if !updates.is_empty() {
        app.events.publish(GlobalEvent::PluginUpdateChanged);
    }
    updates
}

async fn check_plugin_updates_with_progress<F>(
    app: &Arc<DaemonContext>,
    plugin_id: Option<&str>,
    on_progress: F,
) -> Vec<crate::plugin::update::PluginUpdateInfo>
where
    F: FnMut(f32) + Send,
{
    let _guard = app.plugin_update_check.lock().await;
    let mut packages = app.plugins.read().await.clone();
    let plugin_id = plugin_id.filter(|id| !id.is_empty());
    if let Some(plugin_id) = plugin_id {
        packages.retain(|pkg| pkg.id == plugin_id);
    }
    let updates = crate::plugin::update::check_packages_with_progress(
        &app.plugin_updates,
        packages,
        plugin_id.is_none(),
        on_progress,
    )
    .await;
    if !updates.is_empty() {
        app.events.publish(GlobalEvent::PluginUpdateChanged);
    }
    updates
}

pub async fn plugin_update_snapshots(
    app: &Arc<DaemonContext>,
    plugin_id: Option<&str>,
) -> Vec<crate::plugin::update::PluginUpdateInfo> {
    let mut packages = app.plugins.read().await.clone();
    let plugin_id = plugin_id.filter(|id| !id.is_empty());
    if let Some(plugin_id) = plugin_id {
        packages.retain(|pkg| pkg.id == plugin_id);
    }
    let mut out = Vec::with_capacity(packages.len());
    for pkg in packages {
        out.push(crate::plugin::update::snapshot_for_package(&app.plugin_updates, &pkg).await);
    }
    out
}

async fn notify_new_plugin_updates(
    app: &Arc<DaemonContext>,
    previous: &HashMap<String, PluginUpdateInfo>,
    updates: &[PluginUpdateInfo],
) {
    if !app.settings.global().plugin_update_notifications {
        return;
    }

    let available = updates
        .iter()
        .filter(|info| crate::plugin::update::became_available(previous.get(&info.plugin_id), info))
        .collect::<Vec<_>>();
    if available.is_empty() {
        return;
    }

    let plugin_names = app
        .plugins
        .read()
        .await
        .iter()
        .map(|pkg| (pkg.id.clone(), pkg.name.clone()))
        .collect::<HashMap<_, _>>();
    let (summary, body) = plugin_update_notification_text(&available, &plugin_names);
    if let Err(e) =
        crate::system::notifications::notify(PLUGIN_UPDATE_NOTIFICATION_ID, &summary, &body).await
    {
        log::warn!("plugin update notification failed: {e}");
    }
}

fn plugin_update_notification_text(
    updates: &[&PluginUpdateInfo],
    plugin_names: &HashMap<String, String>,
) -> (String, String) {
    if let [info] = updates {
        return (
            "Plugin update available".into(),
            format!("{} is available.", plugin_update_label(info, plugin_names)),
        );
    }

    let mut labels = updates
        .iter()
        .take(3)
        .map(|info| plugin_update_label(info, plugin_names))
        .collect::<Vec<_>>();
    if updates.len() > labels.len() {
        let remaining = updates.len() - labels.len();
        labels.push(format!("{remaining} more"));
    }
    (
        format!("{} plugin updates available", updates.len()),
        format!("Available: {}.", labels.join(", ")),
    )
}

fn plugin_update_label(info: &PluginUpdateInfo, plugin_names: &HashMap<String, String>) -> String {
    let name = plugin_names
        .get(&info.plugin_id)
        .filter(|name| !name.is_empty())
        .unwrap_or(&info.plugin_id);
    if info.latest_version.is_empty() {
        name.clone()
    } else {
        format!("{name} {}", info.latest_version)
    }
}

pub fn plugin_update_check_query_id(plugin_id: Option<&str>) -> String {
    match plugin_id.filter(|id| !id.is_empty()) {
        Some(plugin_id) => format!("plugin/update-check/{plugin_id}"),
        None => "plugin/update-check/all".into(),
    }
}

pub fn spawn_plugin_update_check(
    app: &Arc<DaemonContext>,
    plugin_id: Option<String>,
) -> crate::tasks::ProgressTaskSubmission {
    let query_id = plugin_update_check_query_id(plugin_id.as_deref());
    let event_sender = app.events.sender();
    let sink: crate::tasks::ProgressSink = Arc::new(move |progress| {
        let _ = event_sender.send(GlobalEvent::TaskProgress(progress));
    });
    let task_app = app.clone();
    let task_plugin_id = plugin_id.clone();
    app.tasks.spawn_progress_async_once(
        crate::tasks::TaskKind::Generic,
        query_id.clone(),
        query_id,
        sink,
        move |reporter| async move {
            let progress_reporter = reporter.clone();
            let _ = check_plugin_updates_with_progress(
                &task_app,
                task_plugin_id.as_deref(),
                move |progress| progress_reporter.report(progress, ""),
            )
            .await;
            Ok(())
        },
    )
}

pub fn plugin_update_install_query_id(plugin_id: &str) -> String {
    format!("plugin/update-install/{plugin_id}")
}

pub fn spawn_plugin_update_install(
    app: &Arc<DaemonContext>,
    plugin_id: String,
) -> Result<crate::tasks::ProgressTaskSubmission> {
    let plugin_id = plugin_id.trim().to_string();
    if plugin_id.is_empty() {
        return Err(Error::InvalidArgument("plugin id is empty".into()));
    }

    let query_id = plugin_update_install_query_id(&plugin_id);
    let event_sender = app.events.sender();
    let sink: crate::tasks::ProgressSink = Arc::new(move |progress| {
        let _ = event_sender.send(GlobalEvent::TaskProgress(progress));
    });
    let task_app = app.clone();
    let task_plugin_id = plugin_id.clone();
    Ok(app.tasks.spawn_progress_async_once(
        crate::tasks::TaskKind::Generic,
        query_id.clone(),
        query_id,
        sink,
        move |reporter| async move {
            let info = plugin_update_info_for_install(&task_app, &task_plugin_id)
                .await
                .map_err(anyhow::Error::from)?;
            reporter.report(0.05, "");
            let archive = super::download::download_archive(&info, reporter.clone())
                .await
                .map_err(anyhow::Error::from)?;
            let result =
                install_downloaded_plugin_update(&task_app, &task_plugin_id, &archive, reporter)
                    .await
                    .map_err(anyhow::Error::from);
            let _ = tokio::fs::remove_file(&archive).await;
            result
        },
    ))
}

async fn plugin_update_info_for_install(
    app: &Arc<DaemonContext>,
    plugin_id: &str,
) -> Result<PluginUpdateInfo> {
    let active = app
        .plugins
        .read()
        .await
        .iter()
        .any(|pkg| pkg.id == plugin_id);
    if !active {
        return Err(Error::InvalidArgument(format!(
            "plugin '{plugin_id}' is not active"
        )));
    }

    let Some(info) = app.plugin_updates.read().await.get(plugin_id).cloned() else {
        return Err(Error::FailedPrecondition(format!(
            "plugin '{plugin_id}' has no checked update"
        )));
    };
    if info.state != PluginUpdateState::Available {
        return Err(Error::FailedPrecondition(format!(
            "plugin '{plugin_id}' has no available update"
        )));
    }
    if info.zip_url.trim().is_empty() {
        return Err(Error::PluginInstallFailed(format!(
            "plugin '{plugin_id}' update has no zip url"
        )));
    }
    if info.sha256.trim().is_empty() {
        return Err(Error::PluginInstallFailed(format!(
            "plugin '{plugin_id}' update has no sha256"
        )));
    }
    Ok(info)
}

async fn install_downloaded_plugin_update(
    app: &Arc<DaemonContext>,
    plugin_id: &str,
    archive: &Path,
    reporter: crate::tasks::ProgressReporter,
) -> Result<()> {
    reporter.report(0.82, "");
    let inspect_path = archive.to_string_lossy().to_string();
    let info =
        tokio::task::spawn_blocking(move || crate::plugin::installer::inspect_zip(&inspect_path))
            .await
            .map_err(|e| Error::Internal(anyhow!("plugin update inspect join: {e}")))??;
    if info.id != plugin_id {
        return Err(Error::PluginInstallFailed(format!(
            "update archive id '{}' does not match '{}'",
            info.id, plugin_id
        )));
    }

    reporter.report(0.90, "");
    let install_path = archive.to_string_lossy().to_string();
    let result = install_plugin_archive(app, install_path).await?;
    if result.plugin_id != plugin_id {
        return Err(Error::PluginInstallFailed(format!(
            "installed plugin '{}' does not match '{}'",
            result.plugin_id, plugin_id
        )));
    }
    app.events.publish(GlobalEvent::PluginUpdateChanged);
    Ok(())
}

pub async fn run_plugin_update_checker(
    app: Arc<DaemonContext>,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) -> anyhow::Result<()> {
    if *shutdown.borrow() {
        return Ok(());
    }

    let initial_delay = tokio::time::sleep(Duration::from_secs(10));
    tokio::pin!(initial_delay);
    tokio::select! {
        _ = shutdown.changed() => return Ok(()),
        _ = &mut initial_delay => {}
    }

    loop {
        let previous = app.plugin_updates.read().await.clone();
        let updates = check_plugin_updates(&app, None).await;
        notify_new_plugin_updates(&app, &previous, &updates).await;

        let wait = tokio::time::sleep(Duration::from_secs(30 * 60));
        tokio::pin!(wait);
        tokio::select! {
            _ = shutdown.changed() => return Ok(()),
            _ = &mut wait => {}
        }
    }
}
