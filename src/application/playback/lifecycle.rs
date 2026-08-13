use std::sync::Arc;

use crate::error::Result;
use crate::DaemonContext;

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

pub async fn set_stop_all(app: &Arc<DaemonContext>, stopped: bool) -> Result<bool> {
    app.router.set_manual_stop(stopped).await;
    notify_lifecycle_changed(app).await;
    Ok(stopped)
}

async fn notify_lifecycle_changed(app: &Arc<DaemonContext>) {
    crate::system::tray::dbusmenu::notify_menu_changed(app).await;
    app.events
        .publish(crate::events::GlobalEvent::StatusChanged);
}
