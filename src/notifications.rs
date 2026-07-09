use std::collections::HashMap;

use zbus::zvariant::Value;

const SERVICE: &str = "org.freedesktop.Notifications";
const PATH: &str = "/org/freedesktop/Notifications";
const INTERFACE: &str = "org.freedesktop.Notifications";
const APP_NAME: &str = "Waywallen";
const APP_ICON: &str = "org.waywallen.waywallen";

pub async fn notify(summary: &str, body: &str) -> zbus::Result<u32> {
    let conn = zbus::Connection::session().await?;
    let actions: Vec<&str> = Vec::new();
    let hints: HashMap<&str, Value<'_>> = HashMap::new();
    let reply = conn
        .call_method(
            Some(SERVICE),
            PATH,
            Some(INTERFACE),
            "Notify",
            &(
                APP_NAME, 0u32, APP_ICON, summary, body, actions, hints, -1i32,
            ),
        )
        .await?;
    reply.body().deserialize()
}
