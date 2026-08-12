use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use anyhow::Result;
use futures_util::StreamExt;
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;
use zbus::fdo::{DBusProxy, PropertiesProxy};
use zbus::zvariant::OwnedValue;

use crate::tasks::TaskKind;
use crate::wallframe::renderer_manager::{
    MprisSnapshot, RendererEventKind, RendererId, RendererSubscriptionSnapshot,
};
use crate::DaemonContext;

const MPRIS_PREFIX: &str = "org.mpris.MediaPlayer2.";
const MPRIS_PATH: &str = "/org/mpris/MediaPlayer2";
const MPRIS_PLAYER_IFACE: &str = "org.mpris.MediaPlayer2.Player";

const STATE_STOPPED: u32 = 0;
const STATE_PLAYING: u32 = 1;
const STATE_PAUSED: u32 = 2;
const LOG_TEXT_MAX_CHARS: usize = 80;

enum PlayerMsg {
    Snapshot {
        name: String,
        snapshot: MprisSnapshot,
    },
    Gone(String),
}

struct PlayerTask {
    handle: JoinHandle<()>,
}

pub fn spawn(app: Arc<DaemonContext>) {
    let task_app = app.clone();
    app.tasks
        .spawn_async(TaskKind::Service, "service/mpris", async move {
            run(task_app).await
        });
}

async fn run(app: Arc<DaemonContext>) -> Result<()> {
    let conn = match zbus::Connection::session().await {
        Ok(conn) => conn,
        Err(e) => {
            log::warn!("cannot connect to D-Bus session bus: {e}");
            return Ok(());
        }
    };
    log::debug!("connected to D-Bus session bus");
    let dbus = match DBusProxy::new(&conn).await {
        Ok(proxy) => proxy,
        Err(e) => {
            log::warn!("org.freedesktop.DBus proxy unavailable: {e}");
            return Ok(());
        }
    };
    let mut name_stream = match dbus.receive_name_owner_changed().await {
        Ok(stream) => stream,
        Err(e) => {
            log::warn!("NameOwnerChanged subscription failed: {e}");
            return Ok(());
        }
    };

    let (tx, mut rx) = mpsc::channel::<PlayerMsg>(64);
    let mut shutdown = app.shutdown_subscribe();
    let mut subscriptions = app.renderer_manager.subscribe_subscriptions();
    let mut tasks: BTreeMap<String, PlayerTask> = BTreeMap::new();
    let mut players: BTreeMap<String, MprisSnapshot> = BTreeMap::new();
    let mut current = MprisSnapshot::default();
    let mut known_subscribers = BTreeMap::new();
    let mut discovered = 0usize;

    match dbus.list_names().await {
        Ok(names) => {
            for name in names {
                let name = name.as_str().to_string();
                if is_mpris_name(&name) {
                    discovered += 1;
                    spawn_player_watch(
                        &mut tasks,
                        conn.clone(),
                        name,
                        tx.clone(),
                        app.shutdown_subscribe(),
                    );
                }
            }
        }
        Err(e) => log::warn!("ListNames failed: {e}"),
    }
    log::debug!("discovered {discovered} existing MPRIS player(s)");
    let initial_subscribers = {
        let snapshot = subscriptions.borrow_and_update();
        mpris_subscribers(&snapshot)
    };
    let initial_targets = updated_subscribers(&known_subscribers, &initial_subscribers);
    known_subscribers = initial_subscribers;
    if !initial_targets.is_empty() {
        publish_to_renderers(&app, &current, &initial_targets, "subscription").await;
    }

    loop {
        tokio::select! {
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    break;
                }
            }
            msg = rx.recv() => {
                let Some(msg) = msg else { break; };
                match msg {
                    PlayerMsg::Snapshot { name, snapshot } => {
                        log::trace!("snapshot from {name}: {}", snapshot_debug(&snapshot));
                        players.insert(name, snapshot);
                    }
                    PlayerMsg::Gone(name) => {
                        log::debug!("player gone: {name}");
                        players.remove(&name);
                        if let Some(task) = tasks.remove(&name) {
                            stop_player_task(task).await;
                        }
                    }
                }
                let next = choose_snapshot(&players);
                if next != current {
                    current = next;
                    publish_current_snapshot(
                        &app,
                        &mut subscriptions,
                        &mut known_subscribers,
                        &current,
                    ).await;
                }
            }
            signal = name_stream.next() => {
                let Some(signal) = signal else { break; };
                match signal.args() {
                    Ok(args) => {
                        let name = args.name.as_str().to_string();
                        if !is_mpris_name(&name) {
                            continue;
                        }
                        let appeared = args.new_owner.as_ref().is_some();
                        log::debug!("NameOwnerChanged name={name} appeared={appeared}");
                        if appeared {
                            spawn_player_watch(
                                &mut tasks,
                                conn.clone(),
                                name,
                                tx.clone(),
                                app.shutdown_subscribe(),
                            );
                        } else {
                            players.remove(&name);
                            if let Some(task) = tasks.remove(&name) {
                                stop_player_task(task).await;
                            }
                            let next = choose_snapshot(&players);
                            if next != current {
                                current = next;
                                publish_current_snapshot(
                                    &app,
                                    &mut subscriptions,
                                    &mut known_subscribers,
                                    &current,
                                ).await;
                            }
                        }
                    }
                    Err(e) => log::warn!("bad NameOwnerChanged signal: {e}"),
                }
            }
            changed = subscriptions.changed() => {
                if changed.is_err() {
                    break;
                }
                let next_subscribers = {
                    let snapshot = subscriptions.borrow_and_update();
                    mpris_subscribers(&snapshot)
                };
                let targets = updated_subscribers(&known_subscribers, &next_subscribers);
                known_subscribers = next_subscribers;
                if !targets.is_empty() {
                    publish_to_renderers(&app, &current, &targets, "subscription").await;
                }
            }
        }
    }

    for (_, task) in tasks {
        stop_player_task(task).await;
    }
    Ok(())
}

async fn stop_player_task(task: PlayerTask) {
    task.handle.abort();
    let _ = task.handle.await;
}

fn spawn_player_watch(
    tasks: &mut BTreeMap<String, PlayerTask>,
    conn: zbus::Connection,
    name: String,
    tx: mpsc::Sender<PlayerMsg>,
    shutdown: watch::Receiver<bool>,
) {
    if tasks.contains_key(&name) {
        log::debug!("player watch already running for {name}");
        return;
    }
    log::debug!("spawning player watch for {name}");
    let task_name = name.clone();
    let handle = tokio::spawn(async move {
        watch_player(conn, task_name, tx, shutdown).await;
    });
    tasks.insert(name, PlayerTask { handle });
}

async fn watch_player(
    conn: zbus::Connection,
    name: String,
    tx: mpsc::Sender<PlayerMsg>,
    mut shutdown: watch::Receiver<bool>,
) {
    let proxy = match zbus::Proxy::new(&conn, name.as_str(), MPRIS_PATH, MPRIS_PLAYER_IFACE).await {
        Ok(proxy) => proxy,
        Err(e) => {
            log::warn!("player proxy unavailable for {name}: {e}");
            let _ = tx.send(PlayerMsg::Gone(name)).await;
            return;
        }
    };
    let props_builder = match PropertiesProxy::builder(&conn).destination(name.as_str()) {
        Ok(builder) => builder,
        Err(e) => {
            log::warn!("properties proxy destination failed for {name}: {e}");
            let _ = tx.send(PlayerMsg::Gone(name)).await;
            return;
        }
    };
    let props_builder = match props_builder.path(MPRIS_PATH) {
        Ok(builder) => builder,
        Err(e) => {
            log::warn!("properties proxy path failed for {name}: {e}");
            let _ = tx.send(PlayerMsg::Gone(name)).await;
            return;
        }
    };
    let props = match props_builder.build().await {
        Ok(proxy) => proxy,
        Err(e) => {
            log::warn!("properties proxy unavailable for {name}: {e}");
            let _ = tx.send(PlayerMsg::Gone(name)).await;
            return;
        }
    };
    let mut changes = match props.receive_properties_changed().await {
        Ok(stream) => stream,
        Err(e) => {
            log::warn!("PropertiesChanged subscription failed for {name}: {e}");
            let _ = tx.send(PlayerMsg::Gone(name)).await;
            return;
        }
    };

    log::debug!("watching player {name}");
    let mut last_art_url = String::new();
    let mut previous_art_url = String::new();
    if let Some(snapshot) =
        read_player_snapshot(&proxy, &mut last_art_url, &mut previous_art_url).await
    {
        log::trace!("initial snapshot for {name}: {}", snapshot_debug(&snapshot));
        let _ = tx
            .send(PlayerMsg::Snapshot {
                name: name.clone(),
                snapshot,
            })
            .await;
    }

    loop {
        tokio::select! {
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    break;
                }
            }
            signal = changes.next() => {
                let Some(signal) = signal else { break; };
                if signal
                    .args()
                    .map(|args| args.interface_name.as_str() != MPRIS_PLAYER_IFACE)
                    .unwrap_or(false)
                {
                    continue;
                }
                log::trace!("PropertiesChanged for {name}");
                if let Some(snapshot) =
                    read_player_snapshot(&proxy, &mut last_art_url, &mut previous_art_url).await
                {
                    log::trace!(
                        "updated snapshot for {name}: {}",
                        snapshot_debug(&snapshot)
                    );
                    let _ = tx.send(PlayerMsg::Snapshot {
                        name: name.clone(),
                        snapshot,
                    }).await;
                }
            }
        }
    }
}

async fn read_player_snapshot(
    proxy: &zbus::Proxy<'_>,
    last_art_url: &mut String,
    previous_art_url: &mut String,
) -> Option<MprisSnapshot> {
    let status = proxy
        .get_property::<String>("PlaybackStatus")
        .await
        .unwrap_or_else(|_| "Stopped".to_string());
    let metadata = proxy
        .get_property::<HashMap<String, OwnedValue>>("Metadata")
        .await
        .unwrap_or_default();
    let art_url = normalize_art_url(&metadata_string(&metadata, "mpris:artUrl"));
    if art_url != *last_art_url {
        if !last_art_url.is_empty() {
            *previous_art_url = last_art_url.clone();
        }
        *last_art_url = art_url.clone();
    }
    Some(MprisSnapshot {
        state: playback_state_from_status(&status),
        title: metadata_string(&metadata, "xesam:title"),
        artist: metadata_string_list(&metadata, "xesam:artist"),
        album: metadata_string(&metadata, "xesam:album"),
        album_artist: metadata_string_list(&metadata, "xesam:albumArtist"),
        art_url,
        previous_art_url: previous_art_url.clone(),
    })
}

fn choose_snapshot(players: &BTreeMap<String, MprisSnapshot>) -> MprisSnapshot {
    players
        .values()
        .find(|s| s.state == STATE_PLAYING)
        .or_else(|| players.values().find(|s| snapshot_has_media(s)))
        .or_else(|| players.values().next())
        .cloned()
        .unwrap_or_default()
}

fn snapshot_has_media(s: &MprisSnapshot) -> bool {
    !s.title.is_empty()
        || !s.artist.is_empty()
        || !s.album.is_empty()
        || !s.album_artist.is_empty()
        || !s.art_url.is_empty()
}

type MprisSubscribers = BTreeMap<RendererId, u64>;

fn mpris_subscribers(snapshot: &RendererSubscriptionSnapshot) -> MprisSubscribers {
    snapshot
        .subscribers(RendererEventKind::Mpris)
        .into_iter()
        .collect()
}

fn updated_subscribers(previous: &MprisSubscribers, current: &MprisSubscribers) -> Vec<RendererId> {
    current
        .iter()
        .filter_map(|(id, revision)| (previous.get(id) != Some(revision)).then(|| id.clone()))
        .collect()
}

async fn publish_current_snapshot(
    app: &DaemonContext,
    subscriptions: &mut watch::Receiver<RendererSubscriptionSnapshot>,
    known_subscribers: &mut MprisSubscribers,
    snapshot: &MprisSnapshot,
) {
    let subscribers = {
        let snapshot = subscriptions.borrow_and_update();
        mpris_subscribers(&snapshot)
    };
    let targets: Vec<_> = subscribers.keys().cloned().collect();
    *known_subscribers = subscribers;
    publish_to_renderers(app, snapshot, &targets, "state change").await;
}

async fn publish_to_renderers(
    app: &DaemonContext,
    snapshot: &MprisSnapshot,
    ids: &[RendererId],
    reason: &str,
) {
    log::debug!(
        "publishing {reason} snapshot to {} renderer(s): {}",
        ids.len(),
        snapshot_debug(snapshot)
    );
    for id in ids {
        if let Err(e) = app.renderer_manager.send_mpris(id, snapshot.clone()).await {
            log::warn!("failed to send snapshot to renderer {id}: {e:#}");
        }
    }
}

fn is_mpris_name(name: &str) -> bool {
    name.starts_with(MPRIS_PREFIX) && name.len() > MPRIS_PREFIX.len()
}

fn playback_state_from_status(status: &str) -> u32 {
    match status {
        "Playing" => STATE_PLAYING,
        "Paused" => STATE_PAUSED,
        _ => STATE_STOPPED,
    }
}

fn playback_state_label(state: u32) -> &'static str {
    match state {
        STATE_PLAYING => "Playing",
        STATE_PAUSED => "Paused",
        _ => "Stopped",
    }
}

fn snapshot_debug(snapshot: &MprisSnapshot) -> String {
    format!(
        "state={} title={:?} artist={:?} art={} previous_art={}",
        playback_state_label(snapshot.state),
        truncate_log_text(&snapshot.title),
        truncate_log_text(&snapshot.artist),
        art_url_summary(&snapshot.art_url),
        art_url_summary(&snapshot.previous_art_url),
    )
}

fn truncate_log_text(value: &str) -> String {
    let mut chars = value.chars();
    let mut truncated: String = chars.by_ref().take(LOG_TEXT_MAX_CHARS).collect();
    if chars.next().is_some() {
        truncated.push('…');
    }
    truncated
}

fn art_url_summary(value: &str) -> String {
    let kind = if value.is_empty() {
        return "none".to_string();
    } else if value.starts_with("data:") {
        "data"
    } else if value.starts_with('/') {
        "file"
    } else if value.contains("://") {
        "url"
    } else {
        "value"
    };
    format!("{kind}({} bytes)", value.len())
}

fn metadata_string(metadata: &HashMap<String, OwnedValue>, key: &str) -> String {
    metadata
        .get(key)
        .and_then(owned_value_string)
        .unwrap_or_default()
}

fn metadata_string_list(metadata: &HashMap<String, OwnedValue>, key: &str) -> String {
    metadata
        .get(key)
        .and_then(|v| {
            v.try_clone()
                .ok()
                .and_then(|v| Vec::<String>::try_from(v).ok())
                .map(|items| items.join(", "))
                .or_else(|| owned_value_string(v))
        })
        .unwrap_or_default()
}

fn owned_value_string(value: &OwnedValue) -> Option<String> {
    value
        .try_clone()
        .ok()
        .and_then(|value| String::try_from(value).ok())
}

fn normalize_art_url(raw: &str) -> String {
    let Some(rest) = raw.strip_prefix("file://") else {
        return raw.to_string();
    };
    let path = if let Some(after_localhost) = rest.strip_prefix("localhost/") {
        format!("/{after_localhost}")
    } else if rest.starts_with('/') {
        rest.to_string()
    } else {
        return raw.to_string();
    };
    percent_decode(&path).unwrap_or(path)
}

fn percent_decode(raw: &str) -> Option<String> {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            if i + 2 >= bytes.len() {
                return None;
            }
            let hi = hex_val(bytes[i + 1])?;
            let lo = hex_val(bytes[i + 2])?;
            out.push((hi << 4) | lo);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_local_file_art_url() {
        assert_eq!(
            normalize_art_url("file:///home/me/Cover%20Art.png"),
            "/home/me/Cover Art.png"
        );
        assert_eq!(
            normalize_art_url("file://localhost/tmp/a%23b.jpg"),
            "/tmp/a#b.jpg"
        );
        assert_eq!(
            normalize_art_url("file://remote/tmp/a.jpg"),
            "file://remote/tmp/a.jpg"
        );
    }

    #[test]
    fn maps_playback_status() {
        assert_eq!(playback_state_from_status("Stopped"), STATE_STOPPED);
        assert_eq!(playback_state_from_status("Playing"), STATE_PLAYING);
        assert_eq!(playback_state_from_status("Paused"), STATE_PAUSED);
        assert_eq!(playback_state_from_status("Other"), STATE_STOPPED);
    }

    #[test]
    fn chooses_playing_player_first() {
        let mut players = BTreeMap::new();
        players.insert(
            "org.mpris.MediaPlayer2.paused".to_string(),
            MprisSnapshot {
                state: STATE_PAUSED,
                title: "Paused".to_string(),
                ..MprisSnapshot::default()
            },
        );
        players.insert(
            "org.mpris.MediaPlayer2.playing".to_string(),
            MprisSnapshot {
                state: STATE_PLAYING,
                title: "Playing".to_string(),
                ..MprisSnapshot::default()
            },
        );
        assert_eq!(choose_snapshot(&players).title, "Playing");
    }

    #[test]
    fn selects_new_and_revised_subscribers_in_stable_order() {
        let previous = BTreeMap::from([
            ("keep".to_string(), 2),
            ("revised".to_string(), 3),
            ("removed".to_string(), 1),
        ]);
        let current = BTreeMap::from([
            ("added".to_string(), 1),
            ("keep".to_string(), 2),
            ("revised".to_string(), 4),
        ]);

        assert_eq!(
            updated_subscribers(&previous, &current),
            vec!["added".to_string(), "revised".to_string()]
        );
        assert!(updated_subscribers(&current, &current).is_empty());
        assert!(updated_subscribers(&current, &BTreeMap::new()).is_empty());
    }

    #[test]
    fn snapshot_debug_bounds_text_and_hides_art_payloads() {
        let long_title = "猫".repeat(LOG_TEXT_MAX_CHARS + 10);
        let payload = "A".repeat(256);
        let art_url = format!("data:image/jpeg;base64,{payload}");
        let snapshot = MprisSnapshot {
            state: STATE_PLAYING,
            title: long_title,
            artist: "Artist".to_string(),
            art_url: art_url.clone(),
            previous_art_url: "/tmp/cover.jpg".to_string(),
            ..MprisSnapshot::default()
        };

        let summary = snapshot_debug(&snapshot);
        assert!(!summary.contains(&payload));
        assert!(summary.contains(&format!("data({} bytes)", art_url.len())));
        assert!(summary.contains("file(14 bytes)"));
        assert_eq!(
            truncate_log_text(&snapshot.title).chars().count(),
            LOG_TEXT_MAX_CHARS + 1
        );
        assert!(truncate_log_text(&snapshot.title).ends_with('…'));
    }
}
