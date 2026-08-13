use std::future::Future;
use std::sync::Arc;

use super::DaemonContext;

pub(crate) fn spawn_ui(state: &DaemonContext) -> bool {
    let ui_bin = match state.ui_path.lock().unwrap().clone() {
        Some(path) => path,
        None => return false,
    };
    log::info!("launching ui: {}", ui_bin.display());
    match std::process::Command::new(&ui_bin).spawn() {
        Ok(child) => {
            log::info!("ui pid: {}", child.id());
            true
        }
        Err(error) => {
            log::warn!("failed to launch ui {}: {error}", ui_bin.display());
            false
        }
    }
}

pub(super) async fn run_until_shutdown<F>(
    state: Arc<DaemonContext>,
    websocket: F,
    dbus: Arc<zbus::Connection>,
) -> anyhow::Result<()>
where
    F: Future<Output = anyhow::Result<()>>,
{
    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    tokio::pin!(websocket);

    let websocket_exited = tokio::select! {
        result = &mut websocket => {
            if let Err(error) = result {
                log::error!("ws server exited with error: {error}");
            }
            true
        }
        _ = tokio::signal::ctrl_c() => {
            log::info!("SIGINT received, shutting down");
            false
        }
        _ = sigterm.recv() => {
            log::info!("SIGTERM received, shutting down");
            false
        }
        _ = async {
            let mut receiver = state.shutdown_subscribe();
            let _ = receiver.wait_for(|requested| *requested).await;
        } => {
            log::info!("shutdown requested via D-Bus");
            false
        }
    };

    state.shutdown_now();
    if !websocket_exited {
        if let Err(error) = websocket.await {
            log::warn!("ws server shutdown failed: {error}");
        }
    }
    crate::system::tray::ensure_stopped(&state).await;
    if let Err(error) = state.qr_login.cancel_all_and_wait().await {
        log::warn!("QR login shutdown cleanup failed: {error:#}");
    }
    state.playlists.shutdown().await;
    state.tasks.wait_stopped().await;
    let renderer_ids = state
        .router
        .snapshot_renderers()
        .await
        .into_iter()
        .map(|renderer| renderer.id)
        .collect::<Vec<_>>();
    state
        .router
        .stop_renderers_orderly(&renderer_ids, std::time::Duration::from_secs(1))
        .await;
    state.renderer_manager.shutdown().await;
    state.settings.stop_writer().await;
    state.settings.flush_now().await;

    if let Err(error) = crate::system::dbus::emit_shutting_down(&dbus).await {
        log::warn!("DBus ShuttingDown emit failed: {error}");
    }
    Ok(())
}
