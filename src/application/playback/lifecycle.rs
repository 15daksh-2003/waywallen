use std::sync::Arc;

use crate::error::Result;
use crate::wallframe::scheduler::DisplayId;
use crate::DaemonContext;

use super::apply_wallpaper_to_displays;

pub async fn run_auto_stop_restore(
    app: Arc<DaemonContext>,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) {
    let mut rx = app.router.subscribe_auto_stop();
    log::info!("auto-stop restore service started");
    loop {
        tokio::select! {
            evt = rx.recv() => {
                match evt {
                    Ok(evt) if !evt.stopped => {
                        restore_auto_stopped_display(&app, evt.display_id).await;
                    }
                    Ok(_) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        log::warn!("auto-stop restore lagged {n} events");
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    break;
                }
            }
        }
    }
    log::info!("auto-stop restore service exited");
}

async fn restore_auto_stopped_display(app: &Arc<DaemonContext>, display_id: DisplayId) {
    let Some(display) = app.router.snapshot_display(display_id).await else {
        return;
    };
    if !display.links.is_empty() {
        return;
    }
    let key = display.instance_id.as_deref().unwrap_or(&display.name);
    let Some(wallpaper_id) = app.settings.resolved_last_wallpaper(key) else {
        log::debug!("auto-stop restore: display {display_id} has no saved wallpaper");
        return;
    };
    if let Err(e) = apply_wallpaper_to_displays(app, &wallpaper_id, &[display_id]).await {
        log::warn!("auto-stop restore: apply {wallpaper_id} to display {display_id}: {e:#}");
    }
}

pub async fn pause_all(app: &Arc<DaemonContext>) -> Result<()> {
    set_pause_all(app, true).await?;
    Ok(())
}

pub async fn resume_all(app: &Arc<DaemonContext>) -> Result<()> {
    set_pause_all(app, false).await?;
    Ok(())
}

pub async fn set_pause_all(app: &Arc<DaemonContext>, paused: bool) -> Result<bool> {
    app.router.set_manual_pause(paused).await;
    notify_lifecycle_changed(app).await;
    Ok(paused)
}

pub async fn toggle_pause_all(app: &Arc<DaemonContext>) -> Result<bool> {
    let paused = app.router.toggle_manual_pause().await;
    notify_lifecycle_changed(app).await;
    Ok(paused)
}

pub async fn mute_all(app: &Arc<DaemonContext>) -> Result<()> {
    set_mute_all(app, true).await?;
    Ok(())
}

pub async fn unmute_all(app: &Arc<DaemonContext>) -> Result<()> {
    set_mute_all(app, false).await?;
    Ok(())
}

pub async fn set_mute_all(app: &Arc<DaemonContext>, muted: bool) -> Result<bool> {
    app.router.set_manual_mute(muted).await;
    app.settings.update(|settings| {
        settings.global.manual_muted = muted;
    });
    notify_lifecycle_changed(app).await;
    Ok(muted)
}

pub async fn toggle_mute_all(app: &Arc<DaemonContext>) -> Result<bool> {
    let muted = app.router.toggle_manual_mute().await;
    app.settings.update(|settings| {
        settings.global.manual_muted = muted;
    });
    notify_lifecycle_changed(app).await;
    Ok(muted)
}

async fn notify_lifecycle_changed(app: &Arc<DaemonContext>) {
    crate::system::tray::dbusmenu::notify_menu_changed(app).await;
    app.events
        .publish(crate::events::GlobalEvent::StatusChanged);
}
