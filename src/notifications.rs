use ashpd::desktop::{
    notification::{Notification, NotificationProxy},
    Icon,
};

const APP_ICON: &str = "org.waywallen.waywallen";

pub async fn notify(id: &str, summary: &str, body: &str) -> ashpd::Result<()> {
    let proxy = NotificationProxy::new().await?;
    let notification = Notification::new(summary)
        .body(body)
        .icon(Icon::with_names([APP_ICON]));
    proxy.add_notification(id, notification).await
}
