use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use base64::Engine as _;
use tokio::sync::{broadcast, watch, Mutex};
use tokio_util::sync::CancellationToken;

use crate::control_proto::QrLoginState;
use crate::events::GlobalEvent;

use super::source::{QrLoginPollState, SourceManager};

const DEFAULT_DEADLINE: Duration = Duration::from_secs(180);
const MAX_DEADLINE: Duration = Duration::from_secs(600);
const MIN_POLL_INTERVAL: Duration = Duration::from_millis(250);
const MAX_POLL_INTERVAL: Duration = Duration::from_secs(30);
const SESSION_STOP_TIMEOUT: Duration = Duration::from_secs(55);

struct SessionControl {
    cancel: CancellationToken,
    done: CancellationToken,
}

pub struct QrLoginManager {
    source_manager: Arc<SourceManager>,
    events: broadcast::Sender<GlobalEvent>,
    shutdown: watch::Receiver<bool>,
    sessions: Mutex<HashMap<String, SessionControl>>,
    active_actions: Mutex<HashSet<(String, String)>>,
}

impl QrLoginManager {
    pub fn new(
        source_manager: Arc<SourceManager>,
        events: broadcast::Sender<GlobalEvent>,
        shutdown: watch::Receiver<bool>,
    ) -> Arc<Self> {
        Arc::new(Self {
            source_manager,
            events,
            shutdown,
            sessions: Mutex::new(HashMap::new()),
            active_actions: Mutex::new(HashSet::new()),
        })
    }

    pub async fn start(self: &Arc<Self>, plugin_id: &str, action_id: &str) -> Result<String> {
        let active_key = (plugin_id.to_string(), action_id.to_string());
        let mut active_actions = self.active_actions.lock().await;
        if !active_actions.is_empty() {
            anyhow::bail!("another QR login session is already active");
        }
        active_actions.insert(active_key.clone());
        drop(active_actions);
        let session_id = uuid::Uuid::new_v4().to_string();
        let cancel = CancellationToken::new();
        let done = CancellationToken::new();
        self.sessions.lock().await.insert(
            session_id.clone(),
            SessionControl {
                cancel: cancel.clone(),
                done: done.clone(),
            },
        );
        self.publish(
            &session_id,
            plugin_id,
            action_id,
            QrLoginState::Starting,
            "",
            "",
            "",
            "",
            "",
        );

        let mut shutdown = self.shutdown.clone();
        let shutdown_state = self.shutdown.clone();
        let begin = {
            let shutdown_wait = async {
                loop {
                    if *shutdown.borrow() || shutdown.changed().await.is_err() {
                        break;
                    }
                }
            };
            tokio::pin!(shutdown_wait);
            tokio::select! {
                biased;
                _ = cancel.cancelled() => {
                    self.finish_before_run(
                        &session_id, &active_key, &done, plugin_id, action_id,
                        QrLoginState::Cancelled, "", "", "",
                    ).await;
                    anyhow::bail!("QR login was cancelled");
                }
                _ = &mut shutdown_wait => {
                    self.finish_before_run(
                        &session_id, &active_key, &done, plugin_id, action_id,
                        QrLoginState::Cancelled, "", "", "",
                    ).await;
                    anyhow::bail!("QR login was cancelled during shutdown");
                }
                result = self.source_manager.begin_qr_login(plugin_id, action_id) => {
                    match result {
                        Ok(begin) => begin,
                        Err(error) => {
                            let message = error.to_string();
                            let state = if cancel.is_cancelled() || *shutdown_state.borrow() {
                                QrLoginState::Cancelled
                            } else {
                                QrLoginState::Failed
                            };
                            self.finish_before_run(
                                &session_id, &active_key, &done, plugin_id, action_id,
                                state,
                                if state == QrLoginState::Failed { &message } else { "" },
                                "",
                                "",
                            ).await;
                            return Err(anyhow::anyhow!(error));
                        }
                    }
                }
            }
        };

        let qr_image = match qr_data_url(&begin.challenge) {
            Ok(image) => image,
            Err(error) => {
                let _ = self
                    .source_manager
                    .cancel_qr_login(plugin_id, begin.operation_id)
                    .await;
                self.finish_before_run(
                    &session_id,
                    &active_key,
                    &done,
                    plugin_id,
                    action_id,
                    QrLoginState::Failed,
                    &error.to_string(),
                    &begin.title,
                    &begin.instruction,
                )
                .await;
                return Err(error);
            }
        };
        if cancel.is_cancelled() || *shutdown.borrow() {
            let _ = self
                .source_manager
                .cancel_qr_login(plugin_id, begin.operation_id)
                .await;
            self.finish_before_run(
                &session_id,
                &active_key,
                &done,
                plugin_id,
                action_id,
                QrLoginState::Cancelled,
                "",
                &begin.title,
                &begin.instruction,
            )
            .await;
            anyhow::bail!("QR login was cancelled");
        }
        self.publish(
            &session_id,
            plugin_id,
            action_id,
            QrLoginState::AwaitingScan,
            &qr_image,
            "",
            "",
            &begin.title,
            &begin.instruction,
        );

        let deadline = Duration::from_millis(
            begin
                .expires_in_ms
                .unwrap_or(u64::try_from(DEFAULT_DEADLINE.as_millis()).unwrap_or(180_000)),
        )
        .clamp(Duration::from_secs(1), MAX_DEADLINE);
        let poll_interval = clamp_poll_interval(begin.poll_after_ms);
        let manager = self.clone();
        let task_session_id = session_id.clone();
        let task_plugin_id = plugin_id.to_string();
        let task_action_id = action_id.to_string();
        tokio::spawn(async move {
            manager
                .run(
                    task_session_id,
                    task_plugin_id,
                    task_action_id,
                    begin.operation_id,
                    poll_interval,
                    deadline,
                    cancel,
                    done,
                )
                .await;
        });
        Ok(session_id)
    }

    #[allow(clippy::too_many_arguments)]
    async fn finish_before_run(
        &self,
        session_id: &str,
        active_key: &(String, String),
        done: &CancellationToken,
        plugin_id: &str,
        action_id: &str,
        state: QrLoginState,
        error: &str,
        title: &str,
        instruction: &str,
    ) {
        self.sessions.lock().await.remove(session_id);
        self.active_actions.lock().await.remove(active_key);
        done.cancel();
        self.publish(
            session_id,
            plugin_id,
            action_id,
            state,
            "",
            "",
            error,
            title,
            instruction,
        );
    }

    pub async fn cancel(&self, session_id: &str) -> bool {
        let sessions = self.sessions.lock().await;
        let Some(session) = sessions.get(session_id) else {
            return false;
        };
        session.cancel.cancel();
        true
    }

    pub async fn cancel_all_and_wait(&self) -> Result<()> {
        let waits = {
            let sessions = self.sessions.lock().await;
            sessions
                .values()
                .map(|session| {
                    session.cancel.cancel();
                    session.done.clone()
                })
                .collect::<Vec<_>>()
        };
        for done in waits {
            tokio::time::timeout(SESSION_STOP_TIMEOUT, done.cancelled())
                .await
                .context("timed out while stopping a QR login session")?;
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn run(
        self: Arc<Self>,
        session_id: String,
        plugin_id: String,
        action_id: String,
        operation_id: u64,
        mut poll_interval: Duration,
        deadline: Duration,
        cancel: CancellationToken,
        done: CancellationToken,
    ) {
        let mut shutdown = self.shutdown.clone();
        let deadline = tokio::time::sleep(deadline);
        tokio::pin!(deadline);
        let (state, display_value, error) = loop {
            tokio::select! {
                biased;
                _ = cancel.cancelled() => {
                    let _ = self.source_manager.cancel_qr_login(&plugin_id, operation_id).await;
                    break (QrLoginState::Cancelled, String::new(), String::new());
                }
                _ = wait_for_shutdown(&mut shutdown) => {
                    let _ = self.source_manager.cancel_qr_login(&plugin_id, operation_id).await;
                    break (QrLoginState::Cancelled, String::new(), String::new());
                }
                _ = &mut deadline => {
                    let _ = self.source_manager.cancel_qr_login(&plugin_id, operation_id).await;
                    break (QrLoginState::Expired, String::new(), "QR login expired".into());
                }
                _ = tokio::time::sleep(poll_interval) => {
                    match self.source_manager.poll_qr_login(&plugin_id, operation_id).await {
                        Ok(update) => {
                            if let Some(next) = update.poll_after_ms {
                                poll_interval = clamp_poll_interval(next);
                            }
                            match update.state {
                                QrLoginPollState::AwaitingScan => {
                                    let qr_image = if update.challenge.is_empty() {
                                        String::new()
                                    } else {
                                        match qr_data_url(&update.challenge) {
                                            Ok(image) => image,
                                            Err(error) => {
                                                let _ = self.source_manager
                                                    .cancel_qr_login(&plugin_id, operation_id)
                                                    .await;
                                                break (
                                                    QrLoginState::Failed,
                                                    String::new(),
                                                    error.to_string(),
                                                );
                                            }
                                        }
                                    };
                                    self.publish(
                                        &session_id,
                                        &plugin_id,
                                        &action_id,
                                        QrLoginState::AwaitingScan,
                                        &qr_image,
                                        &update.display_value,
                                        &update.error,
                                        "",
                                        "",
                                    );
                                }
                                QrLoginPollState::AwaitingConfirmation => self.publish(
                                    &session_id,
                                    &plugin_id,
                                    &action_id,
                                    QrLoginState::AwaitingConfirmation,
                                    "",
                                    &update.display_value,
                                    &update.error,
                                    "",
                                    "",
                                ),
                                QrLoginPollState::ChallengeChanged => {
                                    if update.challenge.is_empty() {
                                        let _ = self.source_manager
                                            .cancel_qr_login(&plugin_id, operation_id)
                                            .await;
                                        break (
                                            QrLoginState::Failed,
                                            String::new(),
                                            "QR login provider returned an empty replacement challenge".into(),
                                        );
                                    }
                                    let qr_image = match qr_data_url(&update.challenge) {
                                        Ok(image) => image,
                                        Err(error) => {
                                            let _ = self.source_manager
                                                .cancel_qr_login(&plugin_id, operation_id)
                                                .await;
                                            break (
                                                QrLoginState::Failed,
                                                String::new(),
                                                error.to_string(),
                                            );
                                        }
                                    };
                                    self.publish(
                                        &session_id,
                                        &plugin_id,
                                        &action_id,
                                        QrLoginState::ChallengeChanged,
                                        &qr_image,
                                        &update.display_value,
                                        &update.error,
                                        "",
                                        "",
                                    );
                                }
                                QrLoginPollState::Succeeded => {
                                    break (QrLoginState::Succeeded, update.display_value, update.error);
                                }
                                QrLoginPollState::Expired => {
                                    break (QrLoginState::Expired, update.display_value, update.error);
                                }
                                QrLoginPollState::Failed => {
                                    break (QrLoginState::Failed, update.display_value, update.error);
                                }
                            }
                        }
                        Err(error) => {
                            let _ = self.source_manager.cancel_qr_login(&plugin_id, operation_id).await;
                            break (QrLoginState::Failed, String::new(), error.to_string());
                        }
                    }
                }
            }
        };

        self.publish(
            &session_id,
            &plugin_id,
            &action_id,
            state,
            "",
            &display_value,
            &error,
            "",
            "",
        );
        let succeeded = state == QrLoginState::Succeeded;
        self.sessions.lock().await.remove(&session_id);
        self.active_actions
            .lock()
            .await
            .remove(&(plugin_id, action_id));
        done.cancel();
        if succeeded {
            let _ = self.events.send(GlobalEvent::PluginStateChanged);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn publish(
        &self,
        session_id: &str,
        plugin_id: &str,
        action_id: &str,
        state: QrLoginState,
        qr_image: &str,
        display_value: &str,
        error: &str,
        title: &str,
        instruction: &str,
    ) {
        let _ = self.events.send(GlobalEvent::QrLoginProgress {
            session_id: session_id.to_string(),
            plugin_id: plugin_id.to_string(),
            action_id: action_id.to_string(),
            state: state as i32,
            qr_image: qr_image.to_string(),
            display_value: display_value.to_string(),
            error: error.to_string(),
            title: title.to_string(),
            instruction: instruction.to_string(),
        });
    }
}

async fn wait_for_shutdown(shutdown: &mut watch::Receiver<bool>) {
    loop {
        if *shutdown.borrow() || shutdown.changed().await.is_err() {
            return;
        }
    }
}

fn clamp_poll_interval(milliseconds: u64) -> Duration {
    Duration::from_millis(milliseconds).clamp(MIN_POLL_INTERVAL, MAX_POLL_INTERVAL)
}

pub fn qr_data_url(challenge: &str) -> Result<String> {
    use qrcode::render::svg;
    use qrcode::QrCode;

    let code = QrCode::new(challenge.as_bytes()).context("build QR code")?;
    let svg = code
        .render::<svg::Color>()
        .min_dimensions(256, 256)
        .dark_color(svg::Color("#000000"))
        .light_color(svg::Color("#ffffff"))
        .build();
    Ok(format!(
        "data:image/svg+xml;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(svg)
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::EventBus;
    use crate::plugin::source::{SourceManager, ENTRY_VERSION_V3};

    async fn terminal_state(
        events: &mut broadcast::Receiver<GlobalEvent>,
        session_id: &str,
        label: &str,
    ) -> i32 {
        loop {
            let event = tokio::time::timeout(Duration::from_secs(3), events.recv())
                .await
                .unwrap_or_else(|_| panic!("QR terminal event timed out for {label}"))
                .expect("event bus closed");
            if let GlobalEvent::QrLoginProgress {
                session_id: event_session,
                state,
                ..
            } = event
            {
                if event_session == session_id
                    && matches!(
                        state,
                        x if x == QrLoginState::Succeeded as i32
                            || x == QrLoginState::Expired as i32
                            || x == QrLoginState::Failed as i32
                            || x == QrLoginState::Cancelled as i32
                    )
                {
                    return state;
                }
            }
        }
    }

    #[test]
    fn qr_data_url_is_svg() {
        let url = qr_data_url("https://example.invalid/challenge").unwrap();
        assert!(url.starts_with("data:image/svg+xml;base64,"));
    }

    #[test]
    fn poll_interval_is_bounded() {
        assert_eq!(clamp_poll_interval(1), MIN_POLL_INTERVAL);
        assert_eq!(clamp_poll_interval(60_000), MAX_POLL_INTERVAL);
    }

    #[tokio::test]
    async fn generic_lua_flow_emits_one_terminal_event_and_can_cancel() {
        let dir = tempfile::tempdir().unwrap();
        let entry = dir.path().join("main.lua");
        std::fs::write(
            &entry,
            r#"
local M = {}
function M.info()
    return {
        name = "account_provider",
        capabilities = {},
        actions = {
            { id = "sign_in", kind = "qr_login" },
            { id = "alternate_sign_in", kind = "qr_login" },
        },
    }
end
M.actions = {}
function M.actions.status(ctx)
    return { actions = { sign_in = { visible = true, enabled = true } } }
end
M.qrlogin = {}
function M.qrlogin.begin(ctx, action_id)
    return {
        key = { polls = 0 },
        challenge = "https://example.invalid/challenge",
        poll_after_ms = 1,
        expires_in_ms = 5000,
        title = "Sign in",
        instruction = "Scan",
    }
end
function M.qrlogin.poll(ctx, key)
    key.polls = key.polls + 1
    if key.polls == 1 then
        return {
            state = "challenge_changed",
            challenge = "https://example.invalid/rotated",
        }
    end
    if key.polls == 2 then
        return { state = "awaiting_confirmation" }
    end
    return { state = "succeeded", display_value = "alice" }
end
function M.qrlogin.cancel(ctx, key)
    key.cancelled = true
end
return M
"#,
        )
        .unwrap();

        let source_manager = Arc::new(SourceManager::new().unwrap());
        source_manager
            .load_plugin(&entry, "org.test", "1", ENTRY_VERSION_V3)
            .unwrap();
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let bus = EventBus::default();
        let mut events = bus.subscribe();
        let manager = QrLoginManager::new(source_manager, bus.sender(), shutdown_rx);

        let first = manager.start("account_provider", "sign_in").await.unwrap();
        assert!(manager.start("account_provider", "sign_in").await.is_err());
        let error = manager
            .start("account_provider", "alternate_sign_in")
            .await
            .unwrap_err();
        assert!(error.to_string().contains("another QR login"));
        let mut first_terminal = 0;
        let mut state_changed = false;
        let mut challenge_rotated = false;
        while !state_changed {
            let event = tokio::time::timeout(Duration::from_secs(2), events.recv())
                .await
                .expect("QR flow timed out")
                .expect("event bus closed");
            match event {
                GlobalEvent::QrLoginProgress {
                    session_id, state, ..
                } if session_id == first
                    && matches!(
                        state,
                        x if x == QrLoginState::Succeeded as i32
                            || x == QrLoginState::Expired as i32
                            || x == QrLoginState::Failed as i32
                            || x == QrLoginState::Cancelled as i32
                    ) =>
                {
                    first_terminal += 1;
                    assert_eq!(state, QrLoginState::Succeeded as i32);
                }
                GlobalEvent::PluginStateChanged => state_changed = true,
                GlobalEvent::QrLoginProgress {
                    session_id,
                    state,
                    qr_image,
                    ..
                } if session_id == first && state == QrLoginState::ChallengeChanged as i32 => {
                    challenge_rotated = true;
                    assert!(qr_image.starts_with("data:image/svg+xml;base64,"));
                }
                _ => {}
            }
        }
        assert_eq!(first_terminal, 1);
        assert!(challenge_rotated);

        let second = manager.start("account_provider", "sign_in").await.unwrap();
        assert!(manager.cancel(&second).await);
        let mut second_terminal = 0;
        while second_terminal == 0 {
            let event = tokio::time::timeout(Duration::from_secs(2), events.recv())
                .await
                .expect("QR cancellation timed out")
                .expect("event bus closed");
            if let GlobalEvent::QrLoginProgress {
                session_id, state, ..
            } = event
            {
                if session_id == second && state == QrLoginState::Cancelled as i32 {
                    second_terminal += 1;
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert_eq!(second_terminal, 1);
        assert!(!manager.cancel(&second).await);
        shutdown_tx.send_replace(true);
    }

    #[tokio::test(start_paused = true)]
    async fn provider_terminal_deadline_reload_and_shutdown_are_generic() {
        let dir = tempfile::tempdir().unwrap();
        let entry = dir.path().join("main.lua");
        std::fs::write(
            &entry,
            r#"
local M = {}
function M.info()
    return {
        name = "terminal_provider",
        capabilities = {},
        actions = {
            { id = "expired", kind = "qr_login" },
            { id = "failed", kind = "qr_login" },
            { id = "deadline", kind = "qr_login" },
            { id = "reload", kind = "qr_login" },
            { id = "shutdown", kind = "qr_login" },
        },
    }
end
M.actions = {}
function M.actions.status(ctx) return { actions = {} } end
M.qrlogin = {}
function M.qrlogin.begin(ctx, action_id)
    return {
        key = { mode = action_id },
        challenge = "https://example.invalid/" .. action_id,
        poll_after_ms = 1,
        expires_in_ms = action_id == "deadline" and 1000 or 5000,
    }
end
function M.qrlogin.poll(ctx, key)
    if key.mode == "expired" then return { state = "expired" } end
    if key.mode == "failed" then return { state = "failed", error = "provider failure" } end
    return { state = "awaiting_scan" }
end
function M.qrlogin.cancel(ctx, key) key.cancelled = true end
return M
"#,
        )
        .unwrap();

        let source_manager = Arc::new(SourceManager::new().unwrap());
        source_manager
            .load_plugin(&entry, "org.test", "1", ENTRY_VERSION_V3)
            .unwrap();
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let bus = EventBus::default();
        let mut events = bus.subscribe();
        let manager = QrLoginManager::new(source_manager, bus.sender(), shutdown_rx);

        let expired = manager.start("terminal_provider", "expired").await.unwrap();
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_millis(251)).await;
        tokio::task::yield_now().await;
        assert_eq!(
            terminal_state(&mut events, &expired, "provider expired").await,
            QrLoginState::Expired as i32
        );

        let failed = manager.start("terminal_provider", "failed").await.unwrap();
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_millis(251)).await;
        tokio::task::yield_now().await;
        assert_eq!(
            terminal_state(&mut events, &failed, "provider failed").await,
            QrLoginState::Failed as i32
        );

        let reload = manager.start("terminal_provider", "reload").await.unwrap();
        manager.cancel_all_and_wait().await.unwrap();
        assert_eq!(
            terminal_state(&mut events, &reload, "reload cancellation").await,
            QrLoginState::Cancelled as i32
        );

        let deadline = manager
            .start("terminal_provider", "deadline")
            .await
            .unwrap();
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_millis(1001)).await;
        tokio::task::yield_now().await;
        assert_eq!(
            terminal_state(&mut events, &deadline, "manager deadline").await,
            QrLoginState::Expired as i32
        );

        let shutdown = manager
            .start("terminal_provider", "shutdown")
            .await
            .unwrap();
        shutdown_tx.send_replace(true);
        assert_eq!(
            terminal_state(&mut events, &shutdown, "shutdown cancellation").await,
            QrLoginState::Cancelled as i32
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn reload_waits_for_a_qr_begin_in_progress() {
        let dir = tempfile::tempdir().unwrap();
        let entry = dir.path().join("main.lua");
        std::fs::write(
            &entry,
            r#"
local M = {}
function M.info()
    return {
        name = "slow_begin",
        capabilities = {},
        actions = { { id = "sign_in", kind = "qr_login" } },
    }
end
M.actions = {}
function M.actions.status(ctx) return { actions = {} } end
M.qrlogin = {}
function M.qrlogin.begin(ctx, action_id) while true do end end
function M.qrlogin.poll(ctx, key) return { state = "awaiting_scan" } end
return M
"#,
        )
        .unwrap();

        let source_manager = Arc::new(SourceManager::new().unwrap());
        source_manager
            .load_plugin(&entry, "org.test", "1", ENTRY_VERSION_V3)
            .unwrap();
        source_manager
            .set_test_callback_timeout("slow_begin", Duration::from_millis(40))
            .await;

        let (_shutdown_tx, shutdown_rx) = watch::channel(false);
        let bus = EventBus::default();
        let mut events = bus.subscribe();
        let manager = QrLoginManager::new(source_manager, bus.sender(), shutdown_rx);
        let start_manager = manager.clone();
        let start = tokio::spawn(async move { start_manager.start("slow_begin", "sign_in").await });

        let session_id = loop {
            if let GlobalEvent::QrLoginProgress {
                session_id, state, ..
            } = events.recv().await.unwrap()
            {
                if state == QrLoginState::Starting as i32 {
                    break session_id;
                }
            }
        };
        manager.cancel_all_and_wait().await.unwrap();
        assert!(start.await.unwrap().is_err());
        assert_eq!(
            terminal_state(&mut events, &session_id, "begin cancellation").await,
            QrLoginState::Cancelled as i32
        );
    }
}
