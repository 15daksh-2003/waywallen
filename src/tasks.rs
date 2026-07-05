use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::Result;
use tokio::sync::{broadcast, mpsc, watch};
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

/// How long the supervisor waits after `abort_all` for in-flight tasks.
/// After this deadline shutdown continues anyway.
const SHUTDOWN_DEADLINE: Duration = Duration::from_secs(3);

/// Capacity of the broadcast channel used for `TaskEvent`.
/// Slow subscribers observe `RecvError::Lagged` when they fall behind.
const EVENT_CHANNEL_CAP: usize = 256;

// ---------------------------------------------------------------------------
// Types

/// Unique per-process task identifier. Monotonically increasing; the
/// first task submitted gets 1.
pub type TaskId = u64;

/// Coarse categorization of a task's purpose. Lets UIs group tasks
/// (e.g. "scanning" vs "applying wallpaper") without parsing names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskKind {
    /// One-shot startup work (source scan + DB sync + playlist seed).
    Startup,
    /// User-initiated wallpaper apply (renderer spawn + router relink).
    Apply,
    /// Long-running infrastructure loop (UDS endpoint, layer-shell
    /// supervisor). One entry per long-lived service; stays `Running`
    Service,
    /// Fallback bucket for everything not otherwise classified.
    Generic,
}

impl TaskKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            TaskKind::Startup => "startup",
            TaskKind::Apply => "apply",
            TaskKind::Service => "service",
            TaskKind::Generic => "generic",
        }
    }
}

/// Lifecycle state of a task record.
/// `Failed` carries the `{:#}` formatted error message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskState {
    Running,
    Completed,
    Failed(String),
    Cancelled,
}

impl TaskState {
    /// Short wire-friendly label. Used by DBus `ListTasks`.
    pub fn as_str(&self) -> &'static str {
        match self {
            TaskState::Running => "running",
            TaskState::Completed => "completed",
            TaskState::Failed(_) => "failed",
            TaskState::Cancelled => "cancelled",
        }
    }
}

#[derive(Debug, Clone)]
pub struct TaskRecord {
    pub id: TaskId,
    pub kind: TaskKind,
    pub name: String,
    /// Milliseconds since UNIX epoch when the task was submitted.
    pub started_at_ms: i64,
    pub state: TaskState,
}

/// Lifecycle events broadcast to every subscriber.
/// `Started` carries a full record for local state reconstruction.
#[derive(Debug, Clone)]
pub enum TaskEvent {
    Started(TaskRecord),
    Completed(TaskId),
    Failed(TaskId, String),
    Cancelled(TaskId),
}

#[derive(Debug, Clone, PartialEq)]
pub struct TaskProgress {
    pub query_id: String,
    pub progress: f32,
    pub progressing: bool,
    pub ended: bool,
    pub error: bool,
    pub message: String,
}

#[derive(Clone)]
pub struct ProgressReporter {
    query_id: String,
    sink: ProgressSink,
}

impl ProgressReporter {
    pub fn query_id(&self) -> &str {
        &self.query_id
    }

    pub fn report(&self, progress: f32, message: impl Into<String>) {
        (self.sink)(TaskProgress {
            query_id: self.query_id.clone(),
            progress: progress.clamp(0.0, 1.0),
            progressing: true,
            ended: false,
            error: false,
            message: message.into(),
        });
    }

    fn finish(&self, progress: f32, error: bool, message: impl Into<String>) {
        (self.sink)(TaskProgress {
            query_id: self.query_id.clone(),
            progress: progress.clamp(0.0, 1.0),
            progressing: false,
            ended: true,
            error,
            message: message.into(),
        });
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProgressTaskSubmission {
    pub query_id: String,
    pub task_id: TaskId,
    pub spawned: bool,
}

// ---------------------------------------------------------------------------
// TaskManager — public handle

type BoxedResultFut = Pin<Box<dyn Future<Output = Result<()>> + Send + 'static>>;
type BoxedResultFn = Box<dyn FnOnce() -> Result<()> + Send + 'static>;
pub type ProgressSink = Arc<dyn Fn(TaskProgress) + Send + Sync + 'static>;

enum TaskMsg {
    Async {
        id: TaskId,
        name: String,
        fut: BoxedResultFut,
    },
    Blocking {
        id: TaskId,
        name: String,
        func: BoxedResultFn,
    },
}

pub struct TaskManager {
    tx: mpsc::UnboundedSender<TaskMsg>,
    next_id: AtomicU64,
    records: Arc<RwLock<HashMap<TaskId, TaskRecord>>>,
    events: broadcast::Sender<TaskEvent>,
    /// Per-task cooperative cancellation handles.
    /// Entries live only while the task is running.
    cancel_tokens: Arc<RwLock<HashMap<TaskId, CancellationToken>>>,
    /// Optional dedup key to currently-running TaskId.
    /// `spawn_async_unique` cancels the prior task for the same key.
    unique_keys: Arc<RwLock<HashMap<String, TaskId>>>,
}

impl TaskManager {
    /// Start a supervisor bound to the daemon shutdown watch.
    /// Returned handles are shareable and feed the same supervisor.
    pub fn spawn(shutdown_rx: watch::Receiver<bool>) -> Arc<Self> {
        let (tx, rx) = mpsc::unbounded_channel();
        let (events_tx, _) = broadcast::channel(EVENT_CHANNEL_CAP);
        let records: Arc<RwLock<HashMap<TaskId, TaskRecord>>> =
            Arc::new(RwLock::new(HashMap::new()));

        let cancel_tokens: Arc<RwLock<HashMap<TaskId, CancellationToken>>> =
            Arc::new(RwLock::new(HashMap::new()));
        let unique_keys: Arc<RwLock<HashMap<String, TaskId>>> =
            Arc::new(RwLock::new(HashMap::new()));

        tokio::spawn(supervisor(
            rx,
            shutdown_rx,
            records.clone(),
            events_tx.clone(),
            cancel_tokens.clone(),
            unique_keys.clone(),
        ));

        Arc::new(Self {
            tx,
            next_id: AtomicU64::new(1),
            records,
            events: events_tx,
            cancel_tokens,
            unique_keys,
        })
    }

    /// Submit an async task. Returns the freshly-assigned `TaskId` so
    /// callers can correlate their submission with later events / logs.
    pub fn spawn_async<F>(&self, kind: TaskKind, name: impl Into<String>, fut: F) -> TaskId
    where
        F: Future<Output = Result<()>> + Send + 'static,
    {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let name = name.into();
        let token = CancellationToken::new();
        self.cancel_tokens
            .write()
            .unwrap()
            .insert(id, token.clone());
        self.record_started(id, kind, name.clone());
        let wrapped = async move {
            tokio::select! {
                _ = token.cancelled() => Err(anyhow::anyhow!("cancelled")),
                r = fut => r,
            }
        };
        if let Err(e) = self.tx.send(TaskMsg::Async {
            id,
            name: name.clone(),
            fut: Box::pin(wrapped),
        }) {
            log::warn!("task '{name}' (id {id}) dropped: supervisor is gone ({e})");
            self.cancel_tokens.write().unwrap().remove(&id);
            self.finalize(id, TaskState::Failed("supervisor gone".into()));
        }
        id
    }

    /// Like [`spawn_async`] but de-duplicates by `key`.
    /// A running task under the same key is cancelled first.
    pub fn spawn_async_unique<F>(
        &self,
        kind: TaskKind,
        key: impl Into<String>,
        name: impl Into<String>,
        fut: F,
    ) -> TaskId
    where
        F: Future<Output = Result<()>> + Send + 'static,
    {
        let key = key.into();
        let prev = self.unique_keys.read().unwrap().get(&key).copied();
        if let Some(prev_id) = prev {
            // Best-effort: cancel returns false if the task already
            // finished. The unique key will be replaced below.
            self.cancel(prev_id);
        }
        let id = self.spawn_async(kind, name, fut);
        self.unique_keys.write().unwrap().insert(key, id);
        id
    }

    pub fn spawn_progress_async_once<F, Fut>(
        &self,
        kind: TaskKind,
        query_id: impl Into<String>,
        name: impl Into<String>,
        sink: ProgressSink,
        fut: F,
    ) -> ProgressTaskSubmission
    where
        F: FnOnce(ProgressReporter) -> Fut + Send + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        let query_id = query_id.into();
        if let Some(task_id) = self.running_unique_task_id(&query_id) {
            return ProgressTaskSubmission {
                query_id,
                task_id,
                spawned: false,
            };
        }

        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let name = name.into();
        let token = CancellationToken::new();
        self.cancel_tokens
            .write()
            .unwrap()
            .insert(id, token.clone());
        self.record_started(id, kind, name.clone());
        self.unique_keys
            .write()
            .unwrap()
            .insert(query_id.clone(), id);

        let reporter = ProgressReporter {
            query_id: query_id.clone(),
            sink,
        };
        let task_reporter = reporter.clone();
        let wrapped = async move {
            task_reporter.report(0.0, "");
            let result = tokio::select! {
                _ = token.cancelled() => Err(anyhow::anyhow!("cancelled")),
                r = fut(task_reporter.clone()) => r,
            };
            match &result {
                Ok(()) => task_reporter.finish(1.0, false, ""),
                Err(e) => task_reporter.finish(1.0, true, format!("{e:#}")),
            }
            result
        };
        if let Err(e) = self.tx.send(TaskMsg::Async {
            id,
            name: name.clone(),
            fut: Box::pin(wrapped),
        }) {
            log::warn!("task '{name}' (id {id}) dropped: supervisor is gone ({e})");
            self.cancel_tokens.write().unwrap().remove(&id);
            self.unique_keys.write().unwrap().retain(|_, v| *v != id);
            reporter.finish(1.0, true, "supervisor gone");
            self.finalize(id, TaskState::Failed("supervisor gone".into()));
        }
        ProgressTaskSubmission {
            query_id,
            task_id: id,
            spawned: true,
        }
    }

    /// Cooperatively cancel a running task.
    /// Returns `true` when a live cancellation token existed.
    pub fn cancel(&self, id: TaskId) -> bool {
        let token = self.cancel_tokens.read().unwrap().get(&id).cloned();
        let Some(token) = token else { return false };
        token.cancel();
        // Pre-mark the record as Cancelled so a later cancelled error
        // does not get promoted to Failed.
        let mut prev_state_was_running = false;
        if let Some(rec) = self.records.write().unwrap().get_mut(&id) {
            if matches!(rec.state, TaskState::Running) {
                rec.state = TaskState::Cancelled;
                prev_state_was_running = true;
            }
        }
        if prev_state_was_running {
            let _ = self.events.send(TaskEvent::Cancelled(id));
        }
        true
    }

    /// Submit a blocking task. Runs on the Tokio blocking pool.
    pub fn spawn_blocking<F>(&self, kind: TaskKind, name: impl Into<String>, func: F) -> TaskId
    where
        F: FnOnce() -> Result<()> + Send + 'static,
    {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let name = name.into();
        self.record_started(id, kind, name.clone());
        if let Err(e) = self.tx.send(TaskMsg::Blocking {
            id,
            name: name.clone(),
            func: Box::new(func),
        }) {
            log::warn!("task '{name}' (id {id}) dropped: supervisor is gone ({e})");
            self.finalize(id, TaskState::Failed("supervisor gone".into()));
        }
        id
    }

    /// Snapshot of all currently tracked tasks.
    /// Finished entries are capped by `TRIM_FINISHED_ABOVE`.
    pub fn list(&self) -> Vec<TaskRecord> {
        self.records.read().unwrap().values().cloned().collect()
    }

    /// Subscribe to lifecycle events. Late subscribers miss historical
    /// events and should re-snapshot via [`list`](Self::list) on start.
    pub fn subscribe(&self) -> broadcast::Receiver<TaskEvent> {
        self.events.subscribe()
    }

    fn running_unique_task_id(&self, key: &str) -> Option<TaskId> {
        let task_id = self.unique_keys.read().unwrap().get(key).copied()?;
        let running = self
            .records
            .read()
            .unwrap()
            .get(&task_id)
            .is_some_and(|r| matches!(r.state, TaskState::Running));
        if running {
            return Some(task_id);
        }
        self.unique_keys.write().unwrap().remove(key);
        None
    }

    fn record_started(&self, id: TaskId, kind: TaskKind, name: String) {
        let record = TaskRecord {
            id,
            kind,
            name,
            started_at_ms: now_ms(),
            state: TaskState::Running,
        };
        {
            let mut recs = self.records.write().unwrap();
            recs.insert(id, record.clone());
            trim_finished(&mut recs);
        }
        let _ = self.events.send(TaskEvent::Started(record));
    }

    fn finalize(&self, id: TaskId, state: TaskState) {
        let event = match &state {
            TaskState::Completed => Some(TaskEvent::Completed(id)),
            TaskState::Failed(msg) => Some(TaskEvent::Failed(id, msg.clone())),
            TaskState::Cancelled => Some(TaskEvent::Cancelled(id)),
            TaskState::Running => None,
        };
        if let Some(rec) = self.records.write().unwrap().get_mut(&id) {
            rec.state = state;
        }
        if let Some(e) = event {
            let _ = self.events.send(e);
        }
    }
}

/// Cap record history so long-running daemons do not accumulate
/// unbounded finished entries.
const TRIM_FINISHED_ABOVE: usize = 512;

fn trim_finished(recs: &mut HashMap<TaskId, TaskRecord>) {
    if recs.len() <= TRIM_FINISHED_ABOVE {
        return;
    }
    let mut finished: Vec<TaskId> = recs
        .iter()
        .filter_map(|(id, r)| (!matches!(r.state, TaskState::Running)).then_some(*id))
        .collect();
    // Drop oldest (smallest ids) first until we're back under cap.
    finished.sort_unstable();
    let to_drop = recs.len().saturating_sub(TRIM_FINISHED_ABOVE);
    for id in finished.into_iter().take(to_drop) {
        recs.remove(&id);
    }
}

pub(crate) fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Supervisor

async fn supervisor(
    mut rx: mpsc::UnboundedReceiver<TaskMsg>,
    mut shutdown_rx: watch::Receiver<bool>,
    records: Arc<RwLock<HashMap<TaskId, TaskRecord>>>,
    events: broadcast::Sender<TaskEvent>,
    cancel_tokens: Arc<RwLock<HashMap<TaskId, CancellationToken>>>,
    unique_keys: Arc<RwLock<HashMap<String, TaskId>>>,
) {
    // The supervisor's JoinSet tasks resolve to (TaskId, Result) so the
    // joiner can look up records and emit the right TaskEvent.
    let mut set: JoinSet<(TaskId, Result<()>)> = JoinSet::new();
    log::info!("TaskManager supervisor started");

    loop {
        tokio::select! {
            biased;

            changed = shutdown_rx.changed() => {
                if changed.is_err() || *shutdown_rx.borrow() {
                    break;
                }
            }

            msg = rx.recv() => match msg {
                Some(TaskMsg::Async { id, name: _, fut }) => {
                    set.spawn(async move { (id, fut.await) });
                }
                Some(TaskMsg::Blocking { id, name: _, func }) => {
                    set.spawn_blocking(move || (id, func()));
                }
                None => break,
            },

            Some(joined) = set.join_next() => {
                handle_join(joined, &records, &events, &cancel_tokens, &unique_keys);
            }
        }
    }

    log::info!(
        "TaskManager supervisor draining ({} tasks in flight)",
        set.len()
    );
    set.abort_all();
    let deadline = tokio::time::sleep(SHUTDOWN_DEADLINE);
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            biased;
            _ = &mut deadline => {
                log::warn!(
                    "TaskManager shutdown timeout: {} task(s) did not finish in {:?}",
                    set.len(),
                    SHUTDOWN_DEADLINE
                );
                break;
            }
            opt = set.join_next() => match opt {
                Some(joined) => handle_join(joined, &records, &events, &cancel_tokens, &unique_keys),
                None => break,
            },
        }
    }
    log::info!("TaskManager supervisor exited");
}

fn handle_join(
    joined: Result<(TaskId, Result<()>), tokio::task::JoinError>,
    records: &Arc<RwLock<HashMap<TaskId, TaskRecord>>>,
    events: &broadcast::Sender<TaskEvent>,
    cancel_tokens: &Arc<RwLock<HashMap<TaskId, CancellationToken>>>,
    unique_keys: &Arc<RwLock<HashMap<String, TaskId>>>,
) {
    let (id, name, observed_state) = match joined {
        Ok((id, Ok(()))) => {
            let name = lookup_name(records, id);
            (id, name, TaskState::Completed)
        }
        Ok((id, Err(e))) => {
            let name = lookup_name(records, id);
            let msg = format!("{e:#}");
            (id, name, TaskState::Failed(msg))
        }
        Err(e) if e.is_cancelled() => {
            // JoinError::Cancelled is the JoinSet::abort_all path.
            // The task did not get to report its own final state.
            log::info!("task aborted during shutdown");
            return;
        }
        Err(e) => {
            log::warn!("task join error: {e}");
            return;
        }
    };

    // GC the per-task cancel token regardless of outcome.
    cancel_tokens.write().unwrap().remove(&id);
    // GC any unique_keys mapping that pointed at us.
    unique_keys.write().unwrap().retain(|_, v| *v != id);

    // If `cancel(id)` already moved the record to Cancelled, keep that
    // state even if the future later returns Err("cancelled").
    let already_cancelled = matches!(
        records.read().unwrap().get(&id).map(|r| r.state.clone()),
        Some(TaskState::Cancelled)
    );
    if already_cancelled {
        log::info!("task '{name}' (id {id}) cancelled");
        return;
    }

    {
        let mut recs = records.write().unwrap();
        if let Some(rec) = recs.get_mut(&id) {
            rec.state = observed_state.clone();
        }
    }
    match &observed_state {
        TaskState::Completed => {
            log::info!("task '{name}' (id {id}) completed");
            let _ = events.send(TaskEvent::Completed(id));
        }
        TaskState::Failed(msg) => {
            log::warn!("task '{name}' (id {id}) failed: {msg}");
            let _ = events.send(TaskEvent::Failed(id, msg.clone()));
        }
        _ => {}
    }
}

fn lookup_name(records: &Arc<RwLock<HashMap<TaskId, TaskRecord>>>, id: TaskId) -> String {
    records
        .read()
        .unwrap()
        .get(&id)
        .map(|r| r.name.clone())
        .unwrap_or_else(|| format!("id={id}"))
}

// ---------------------------------------------------------------------------
// Tests

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU32;

    async fn wait_for<F: Fn() -> bool>(pred: F, timeout: Duration) -> bool {
        let start = std::time::Instant::now();
        while start.elapsed() < timeout {
            if pred() {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        false
    }

    #[tokio::test]
    async fn async_task_runs_to_completion() {
        let (tx, rx) = watch::channel(false);
        let tm = TaskManager::spawn(rx);
        let hit = Arc::new(AtomicU32::new(0));
        let h = hit.clone();
        let id = tm.spawn_async(TaskKind::Generic, "unit/async-ok", async move {
            h.fetch_add(1, Ordering::SeqCst);
            Ok(())
        });
        assert!(id >= 1);
        assert!(
            wait_for(|| hit.load(Ordering::SeqCst) == 1, Duration::from_secs(1)).await,
            "task never ran"
        );
        let _ = tx.send(true);
    }

    #[tokio::test]
    async fn blocking_task_runs_to_completion() {
        let (tx, rx) = watch::channel(false);
        let tm = TaskManager::spawn(rx);
        let hit = Arc::new(AtomicU32::new(0));
        let h = hit.clone();
        tm.spawn_blocking(TaskKind::Generic, "unit/blocking-ok", move || {
            h.fetch_add(1, Ordering::SeqCst);
            Ok(())
        });
        assert!(
            wait_for(|| hit.load(Ordering::SeqCst) == 1, Duration::from_secs(1)).await,
            "blocking task never ran"
        );
        let _ = tx.send(true);
    }

    #[tokio::test]
    async fn shutdown_aborts_long_async_task() {
        let (tx, rx) = watch::channel(false);
        let tm = TaskManager::spawn(rx);
        let finished = Arc::new(AtomicU32::new(0));
        let f = finished.clone();
        tm.spawn_async(TaskKind::Generic, "unit/long-sleeper", async move {
            tokio::time::sleep(Duration::from_secs(60)).await;
            f.fetch_add(1, Ordering::SeqCst);
            Ok(())
        });
        tokio::time::sleep(Duration::from_millis(20)).await;
        let _ = tx.send(true);
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(finished.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn list_and_events_reflect_lifecycle() {
        let (tx, rx) = watch::channel(false);
        let tm = TaskManager::spawn(rx);
        let mut events = tm.subscribe();

        let id = tm.spawn_async(TaskKind::Generic, "unit/list", async move { Ok(()) });

        // Immediately after submit, the task should appear in list() as Running.
        let snap = tm.list();
        assert!(snap
            .iter()
            .any(|r| r.id == id && matches!(r.state, TaskState::Running)));

        // Started event fires synchronously during spawn_async.
        match tokio::time::timeout(Duration::from_millis(100), events.recv())
            .await
            .expect("no Started event")
            .unwrap()
        {
            TaskEvent::Started(r) => assert_eq!(r.id, id),
            other => panic!("expected Started, got {other:?}"),
        }

        // Completed event follows once the future resolves.
        let done = tokio::time::timeout(Duration::from_secs(1), events.recv()).await;
        match done.expect("no completion event").unwrap() {
            TaskEvent::Completed(i) => assert_eq!(i, id),
            other => panic!("expected Completed, got {other:?}"),
        }

        assert!(
            wait_for(
                || tm
                    .list()
                    .iter()
                    .any(|r| r.id == id && matches!(r.state, TaskState::Completed)),
                Duration::from_secs(1)
            )
            .await
        );

        let _ = tx.send(true);
    }

    #[tokio::test]
    async fn cancel_marks_running_task_cancelled() {
        let (tx, rx) = watch::channel(false);
        let tm = TaskManager::spawn(rx);
        let mut events = tm.subscribe();

        let id = tm.spawn_async(TaskKind::Generic, "unit/cancel-me", async move {
            tokio::time::sleep(Duration::from_secs(60)).await;
            Ok(())
        });
        // Drain Started.
        let _ = tokio::time::timeout(Duration::from_millis(100), events.recv()).await;

        // Wait until supervisor has picked it up so the wrapper future
        // is actually awaiting on the cancel token.
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(tm.cancel(id), "cancel returned false on a running task");

        match tokio::time::timeout(Duration::from_millis(500), events.recv())
            .await
            .expect("no Cancelled event")
            .unwrap()
        {
            TaskEvent::Cancelled(i) => assert_eq!(i, id),
            other => panic!("expected Cancelled, got {other:?}"),
        }

        // Final state observed by list() must stay Cancelled, not flip
        // to Failed when the wrapper future returns Err("cancelled").
        assert!(
            wait_for(
                || tm
                    .list()
                    .iter()
                    .any(|r| r.id == id && matches!(r.state, TaskState::Cancelled)),
                Duration::from_secs(1)
            )
            .await
        );

        // Cancel-token entry should be GC'd after handle_join runs.
        assert!(
            wait_for(|| !tm.cancel(id), Duration::from_secs(1)).await,
            "cancel token leaked"
        );
        let _ = tx.send(true);
    }

    #[tokio::test]
    async fn unique_key_supersedes_prior_running_task() {
        let (tx, rx) = watch::channel(false);
        let tm = TaskManager::spawn(rx);

        let first = tm.spawn_async_unique(
            TaskKind::Generic,
            "apply/output-1",
            "unit/first",
            async move {
                tokio::time::sleep(Duration::from_secs(60)).await;
                Ok(())
            },
        );
        // Let supervisor pick it up.
        tokio::time::sleep(Duration::from_millis(20)).await;

        let second = tm.spawn_async_unique(
            TaskKind::Generic,
            "apply/output-1",
            "unit/second",
            async move { Ok(()) },
        );
        assert_ne!(first, second);

        // First should end up Cancelled, second Completed.
        assert!(
            wait_for(
                || {
                    let snap = tm.list();
                    let f = snap.iter().find(|r| r.id == first);
                    let s = snap.iter().find(|r| r.id == second);
                    matches!(f.map(|r| &r.state), Some(TaskState::Cancelled))
                        && matches!(s.map(|r| &r.state), Some(TaskState::Completed))
                },
                Duration::from_secs(1)
            )
            .await
        );
        let _ = tx.send(true);
    }

    #[tokio::test]
    async fn progress_task_reuses_running_submission() {
        let (tx, rx) = watch::channel(false);
        let tm = TaskManager::spawn(rx);
        let (progress_tx, mut progress_rx) = mpsc::unbounded_channel::<TaskProgress>();
        let sink: ProgressSink = Arc::new(move |p| {
            let _ = progress_tx.send(p);
        });
        let gate = Arc::new(tokio::sync::Notify::new());
        let task_gate = gate.clone();

        let first = tm.spawn_progress_async_once(
            TaskKind::Generic,
            "unit/progress-reuse",
            "unit/progress-reuse",
            sink.clone(),
            move |reporter| {
                let task_gate = task_gate.clone();
                async move {
                    reporter.report(0.5, "half");
                    task_gate.notified().await;
                    Ok(())
                }
            },
        );
        let second = tm.spawn_progress_async_once(
            TaskKind::Generic,
            "unit/progress-reuse",
            "unit/progress-reuse",
            sink,
            move |_| async move { anyhow::bail!("duplicate task should not run") },
        );

        assert!(first.spawned);
        assert!(!second.spawned);
        assert_eq!(first.query_id, second.query_id);
        assert_eq!(first.task_id, second.task_id);

        let seen_half = tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let p = progress_rx.recv().await.expect("progress sender closed");
                if p.query_id == "unit/progress-reuse" && (p.progress - 0.5).abs() < f32::EPSILON {
                    break;
                }
            }
        })
        .await;
        assert!(seen_half.is_ok(), "progress update was not sent");

        gate.notify_one();
        let _ = tx.send(true);
    }

    #[tokio::test]
    async fn progress_task_sends_end_on_completion() {
        let (tx, rx) = watch::channel(false);
        let tm = TaskManager::spawn(rx);
        let (progress_tx, mut progress_rx) = mpsc::unbounded_channel::<TaskProgress>();
        let sink: ProgressSink = Arc::new(move |p| {
            let _ = progress_tx.send(p);
        });

        tm.spawn_progress_async_once(
            TaskKind::Generic,
            "unit/progress-end",
            "unit/progress-end",
            sink,
            move |_| async move { Ok(()) },
        );

        let ended = tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let p = progress_rx.recv().await.expect("progress sender closed");
                if p.query_id == "unit/progress-end" && p.ended {
                    break p;
                }
            }
        })
        .await
        .expect("end progress was not sent");
        assert!(!ended.progressing);
        assert!(!ended.error);
        assert_eq!(ended.progress, 1.0);
        let _ = tx.send(true);
    }

    #[tokio::test]
    async fn failed_task_surfaces_error_string() {
        let (tx, rx) = watch::channel(false);
        let tm = TaskManager::spawn(rx);
        let mut events = tm.subscribe();

        let id = tm.spawn_async(TaskKind::Generic, "unit/failing", async move {
            anyhow::bail!("nope")
        });

        // Drain Started.
        let _ = tokio::time::timeout(Duration::from_millis(100), events.recv()).await;

        let failed = tokio::time::timeout(Duration::from_secs(1), events.recv()).await;
        match failed.expect("no event").unwrap() {
            TaskEvent::Failed(i, msg) => {
                assert_eq!(i, id);
                assert!(msg.contains("nope"), "msg was {msg:?}");
            }
            other => panic!("expected Failed, got {other:?}"),
        }
        let _ = tx.send(true);
    }
}
