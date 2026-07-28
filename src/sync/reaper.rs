use std::collections::{BTreeMap, HashMap, HashSet};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use tokio::sync::mpsc;

use crate::sync::drm_syncobj::{BinarySyncobjState, DrmDevice, SyncobjHandle};

const CANCEL_POLL_INTERVAL: Duration = Duration::from_millis(250);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FrameIdentity {
    pub buffer_generation: u64,
    pub buffer_index: u32,
    pub release_point: u64,
}

pub type DisplaySessionId = u64;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct FrameConsumerIdentity {
    pub frame: FrameIdentity,
    pub renderer_id: String,
    pub display_id: u64,
    pub display_session_id: DisplaySessionId,
    pub display_name: String,
    pub frame_seq: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReleaseWaitState {
    DeliveredUnarmed,
    Armed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReleaseEvent {
    Waiting {
        consumer: FrameConsumerIdentity,
        state: ReleaseWaitState,
    },
    Resolved {
        consumer: FrameConsumerIdentity,
    },
    GenerationPoisoned {
        consumer: FrameConsumerIdentity,
        reason: String,
    },
}

pub(crate) enum FrameRecord {
    Register {
        identity: FrameIdentity,
        consumers: Vec<FrameConsumerIdentity>,
    },
    Delivered {
        consumer: FrameConsumerIdentity,
        member_index: u32,
        consumer_handle: SyncobjHandle,
        requires_arm: bool,
    },
    Armed {
        consumer: FrameConsumerIdentity,
        member_index: u32,
    },
    Skipped {
        consumer: FrameConsumerIdentity,
        member_index: u32,
    },
    SessionClosed {
        display_session_id: DisplaySessionId,
    },
}

pub struct FrameConsumerMember {
    tx: Option<mpsc::UnboundedSender<FrameRecord>>,
    consumer: FrameConsumerIdentity,
    member_index: u32,
}

pub struct FrameConsumerArm {
    tx: Option<mpsc::UnboundedSender<FrameRecord>>,
    consumer: FrameConsumerIdentity,
    member_index: u32,
}

#[derive(Clone)]
pub struct FrameConsumerSession {
    tx: mpsc::UnboundedSender<FrameRecord>,
    renderer_id: String,
    display_session_id: DisplaySessionId,
}

impl FrameConsumerArm {
    pub fn arm(mut self) {
        if let Some(tx) = self.tx.take() {
            let _ = tx.send(FrameRecord::Armed {
                consumer: self.consumer.clone(),
                member_index: self.member_index,
            });
        }
    }
}

impl FrameConsumerSession {
    pub fn renderer_id(&self) -> &str {
        &self.renderer_id
    }

    pub fn close(self) {
        let _ = self.tx.send(FrameRecord::SessionClosed {
            display_session_id: self.display_session_id,
        });
    }
}

impl FrameConsumerMember {
    fn new(
        tx: mpsc::UnboundedSender<FrameRecord>,
        consumer: FrameConsumerIdentity,
        member_index: u32,
    ) -> Self {
        Self {
            tx: Some(tx),
            consumer,
            member_index,
        }
    }

    pub fn session(&self) -> FrameConsumerSession {
        FrameConsumerSession {
            tx: self.tx.as_ref().expect("member not completed").clone(),
            renderer_id: self.consumer.renderer_id.clone(),
            display_session_id: self.consumer.display_session_id,
        }
    }

    pub fn delivered(
        mut self,
        consumer_handle: SyncobjHandle,
        requires_arm: bool,
    ) -> Option<FrameConsumerArm> {
        if let Some(tx) = self.tx.take() {
            let arm_tx = requires_arm.then(|| tx.clone());
            let _ = tx.send(FrameRecord::Delivered {
                consumer: self.consumer.clone(),
                member_index: self.member_index,
                consumer_handle,
                requires_arm,
            });
            return arm_tx.map(|tx| FrameConsumerArm {
                tx: Some(tx),
                consumer: self.consumer.clone(),
                member_index: self.member_index,
            });
        }
        None
    }

    pub fn skip(mut self) {
        self.complete_skipped();
    }

    fn complete_skipped(&mut self) {
        if let Some(tx) = self.tx.take() {
            let _ = tx.send(FrameRecord::Skipped {
                consumer: self.consumer.clone(),
                member_index: self.member_index,
            });
        }
    }
}

impl Drop for FrameConsumerMember {
    fn drop(&mut self) {
        self.complete_skipped();
    }
}

pub(crate) fn register_frame(
    tx: &mpsc::UnboundedSender<FrameRecord>,
    identity: FrameIdentity,
    consumers: Vec<FrameConsumerIdentity>,
) -> Result<Vec<FrameConsumerMember>, &'static str> {
    if consumers.iter().any(|consumer| consumer.frame != identity) {
        return Err("consumer frame identity mismatch");
    }
    tx.send(FrameRecord::Register {
        identity,
        consumers: consumers.clone(),
    })
    .map_err(|_| "reaper channel closed")?;

    Ok(consumers
        .into_iter()
        .enumerate()
        .map(|(member_index, consumer)| {
            FrameConsumerMember::new(tx.clone(), consumer, member_index as u32)
        })
        .collect())
}

enum MemberState {
    Queued,
    DeliveredUnarmed(SyncobjHandle),
    Armed { handle: SyncobjHandle, legacy: bool },
    Skipped,
}

struct BucketMember {
    consumer: FrameConsumerIdentity,
    state: MemberState,
}

struct Bucket {
    identity: FrameIdentity,
    members: Option<Vec<BucketMember>>,
}

impl Bucket {
    fn new(identity: FrameIdentity) -> Self {
        Self {
            identity,
            members: None,
        }
    }

    fn register(&mut self, consumers: Vec<FrameConsumerIdentity>) -> Result<(), &'static str> {
        if self.members.is_some() {
            return Err("duplicate registration");
        }
        self.members = Some(
            consumers
                .into_iter()
                .map(|consumer| BucketMember {
                    consumer,
                    state: MemberState::Queued,
                })
                .collect(),
        );
        Ok(())
    }

    fn member_mut(
        &mut self,
        member_index: u32,
        consumer: &FrameConsumerIdentity,
    ) -> Result<&mut BucketMember, &'static str> {
        let members = self.members.as_mut().ok_or("frame not registered")?;
        let member = members
            .get_mut(member_index as usize)
            .ok_or("member index out of range")?;
        if member.consumer != *consumer {
            return Err("consumer identity mismatch");
        }
        Ok(member)
    }

    fn ready(&self) -> bool {
        self.members.as_ref().is_some_and(|members| {
            members.iter().all(|member| {
                matches!(
                    member.state,
                    MemberState::Armed { .. } | MemberState::Skipped
                )
            })
        })
    }

    fn into_waits(self) -> Vec<ConsumerWait> {
        self.members
            .unwrap_or_default()
            .into_iter()
            .filter_map(|member| match member.state {
                MemberState::Armed { handle, legacy } => Some(ConsumerWait {
                    consumer: member.consumer,
                    handle,
                    legacy,
                }),
                MemberState::Skipped => None,
                MemberState::Queued | MemberState::DeliveredUnarmed(_) => {
                    unreachable!("bucket dispatched before every member was ready")
                }
            })
            .collect()
    }
}

struct ConsumerWait {
    consumer: FrameConsumerIdentity,
    handle: SyncobjHandle,
    legacy: bool,
}

#[derive(Debug, PartialEq, Eq)]
enum ReleaseFrontierInsertError {
    InvalidPoint,
    AlreadyPublished,
    Duplicate,
}

struct ReleaseFrontier<T> {
    next_point: u64,
    ready: BTreeMap<u64, T>,
}

impl<T> ReleaseFrontier<T> {
    fn new() -> Self {
        Self {
            next_point: 1,
            ready: BTreeMap::new(),
        }
    }

    fn insert(&mut self, point: u64, value: T) -> Result<(), ReleaseFrontierInsertError> {
        if point == 0 {
            return Err(ReleaseFrontierInsertError::InvalidPoint);
        }
        if point < self.next_point {
            return Err(ReleaseFrontierInsertError::AlreadyPublished);
        }
        if self.ready.contains_key(&point) {
            return Err(ReleaseFrontierInsertError::Duplicate);
        }
        self.ready.insert(point, value);
        Ok(())
    }

    fn next_ready(&self) -> Option<(u64, &T)> {
        self.ready
            .get(&self.next_point)
            .map(|value| (self.next_point, value))
    }

    fn commit_next(&mut self) -> Option<T> {
        let value = self.ready.remove(&self.next_point)?;
        self.next_point = self.next_point.saturating_add(1);
        Some(value)
    }

    fn pending_count(&self) -> usize {
        self.ready.len()
    }
}

enum WaitCompletion {
    Resolved {
        identity: FrameIdentity,
        handle: SyncobjHandle,
        consumers: Vec<FrameConsumerIdentity>,
    },
    Failed {
        identity: FrameIdentity,
        error: String,
    },
    Poisoned {
        consumer: FrameConsumerIdentity,
        error: String,
    },
    Cancelled,
}

pub(crate) fn spawn_reaper(
    drm: &'static DrmDevice,
    renderer_id: String,
    release_syncobj: Arc<StdMutex<Option<OwnedFd>>>,
    mut rx: mpsc::UnboundedReceiver<FrameRecord>,
    release_events: mpsc::UnboundedSender<ReleaseEvent>,
) {
    tokio::spawn(async move {
        let mut producer_handle: Option<SyncobjHandle> = None;
        let mut buckets: HashMap<u64, Bucket> = HashMap::new();
        let mut frontier = ReleaseFrontier::new();
        let (completion_tx, mut completion_rx) = mpsc::unbounded_channel();
        let cancelled = Arc::new(AtomicBool::new(false));
        let closed_sessions = Arc::new(StdMutex::new(HashSet::<DisplaySessionId>::new()));

        loop {
            tokio::select! {
                maybe_record = rx.recv() => {
                    let Some(record) = maybe_record else {
                        cancelled.store(true, Ordering::Release);
                        if !buckets.is_empty() || frontier.pending_count() != 0 {
                            log::info!(
                                "reaper {renderer_id}: channel closed with {} pending bucket(s) and {} resolved point(s); retiring generation",
                                buckets.len(),
                                frontier.pending_count(),
                            );
                        }
                        log::info!("reaper {renderer_id}: exiting");
                        return;
                    };

                    let record = match record {
                        FrameRecord::SessionClosed { display_session_id } => {
                        if let Ok(mut closed) = closed_sessions.lock() {
                            closed.insert(display_session_id);
                        }
                        let mut poison: Option<(FrameConsumerIdentity, String)> = None;
                        for bucket in buckets.values_mut() {
                            let Some(members) = bucket.members.as_mut() else { continue };
                            for member in members.iter_mut().filter(|member| {
                                member.consumer.display_session_id == display_session_id
                            }) {
                                let state = std::mem::replace(&mut member.state, MemberState::Skipped);
                                member.state = match state {
                                    MemberState::Queued | MemberState::Skipped => MemberState::Skipped,
                                    MemberState::DeliveredUnarmed(handle) => {
                                        match drm.binary_syncobj_state(&handle) {
                                            Ok(BinarySyncobjState::Unsubmitted) => {
                                                let _ = release_events.send(ReleaseEvent::Resolved {
                                                    consumer: member.consumer.clone(),
                                                });
                                                MemberState::Skipped
                                            }
                                            Ok(BinarySyncobjState::Pending | BinarySyncobjState::Signaled) => {
                                                let _ = release_events.send(ReleaseEvent::Waiting {
                                                    consumer: member.consumer.clone(),
                                                    state: ReleaseWaitState::Armed,
                                                });
                                                MemberState::Armed { handle, legacy: false }
                                            }
                                            Err(error) => {
                                                poison = Some((
                                                    member.consumer.clone(),
                                                    format!("classify delivered release on session close: {error}"),
                                                ));
                                                MemberState::DeliveredUnarmed(handle)
                                            }
                                        }
                                    }
                                    MemberState::Armed { handle, legacy } => {
                                        MemberState::Armed { handle, legacy }
                                    }
                                };
                            }
                        }
                        if let Some((consumer, reason)) = poison {
                            cancelled.store(true, Ordering::Release);
                            let _ = release_events.send(ReleaseEvent::GenerationPoisoned {
                                consumer,
                                reason,
                            });
                            return;
                        }
                        let ready_points: Vec<u64> = buckets
                            .iter()
                            .filter_map(|(point, bucket)| bucket.ready().then_some(*point))
                            .collect();
                        for point in ready_points {
                            let bucket = buckets.remove(&point).expect("ready bucket remains registered");
                            dispatch_bucket_wait(
                                drm,
                                &renderer_id,
                                bucket,
                                completion_tx.clone(),
                                Arc::clone(&cancelled),
                                Arc::clone(&closed_sessions),
                            );
                        }
                            continue;
                        }
                        record => record,
                    };

                    let identity = match &record {
                        FrameRecord::Register { identity, .. } => *identity,
                        FrameRecord::Delivered { consumer, .. }
                        | FrameRecord::Armed { consumer, .. }
                        | FrameRecord::Skipped { consumer, .. } => consumer.frame,
                        FrameRecord::SessionClosed { .. } => unreachable!(),
                    };
                    if identity.release_point == 0 {
                        log::warn!("reaper {renderer_id}: reject release point 0");
                        continue;
                    }

                    let entry = buckets
                        .entry(identity.release_point)
                        .or_insert_with(|| Bucket::new(identity));
                    if entry.identity != identity {
                        log::warn!(
                            "reaper {renderer_id}: reject point {} identity mismatch",
                            identity.release_point,
                        );
                        continue;
                    }

                    let poison_consumer = match &record {
                        FrameRecord::Armed { consumer, .. } => Some(consumer.clone()),
                        _ => None,
                    };
                    let update = (|| -> Result<(), &'static str> { match record {
                        FrameRecord::Register { consumers, .. } => {
                            entry.register(consumers)
                        }
                        FrameRecord::Delivered {
                            consumer,
                            member_index,
                            consumer_handle,
                            requires_arm,
                        } => {
                            let member = entry.member_mut(member_index, &consumer)?;
                            if !matches!(member.state, MemberState::Queued) {
                                Err("duplicate member delivery")
                            } else {
                                member.state = if requires_arm {
                                    MemberState::DeliveredUnarmed(consumer_handle)
                                } else {
                                    MemberState::Armed {
                                        handle: consumer_handle,
                                        legacy: true,
                                    }
                                };
                                let _ = release_events.send(ReleaseEvent::Waiting {
                                    consumer,
                                    state: if requires_arm {
                                        ReleaseWaitState::DeliveredUnarmed
                                    } else {
                                        ReleaseWaitState::Armed
                                    },
                                });
                                Ok(())
                            }
                        }
                        FrameRecord::Armed { consumer, member_index } => {
                            let member = entry.member_mut(member_index, &consumer)?;
                            let state = std::mem::replace(&mut member.state, MemberState::Skipped);
                            match state {
                                MemberState::DeliveredUnarmed(handle) => {
                                    match drm.binary_syncobj_state(&handle) {
                                        Ok(BinarySyncobjState::Pending | BinarySyncobjState::Signaled) => {
                                            member.state = MemberState::Armed { handle, legacy: false };
                                            let _ = release_events.send(ReleaseEvent::Waiting {
                                                consumer,
                                                state: ReleaseWaitState::Armed,
                                            });
                                            Ok(())
                                        }
                                        Ok(BinarySyncobjState::Unsubmitted) => {
                                            member.state = MemberState::DeliveredUnarmed(handle);
                                            Err("frame_armed before release fence submission")
                                        }
                                        Err(_) => {
                                            member.state = MemberState::DeliveredUnarmed(handle);
                                            Err("frame_armed release fence classification failed")
                                        }
                                    }
                                }
                                other => {
                                    member.state = other;
                                    Err("frame_armed for member not awaiting arm")
                                }
                            }
                        }
                        FrameRecord::Skipped { consumer, member_index } => {
                            let member = entry.member_mut(member_index, &consumer)?;
                            if matches!(member.state, MemberState::Queued) {
                                member.state = MemberState::Skipped;
                                Ok(())
                            } else {
                                Err("duplicate member completion")
                            }
                        }
                        FrameRecord::SessionClosed { .. } => unreachable!(),
                    } })();
                    if let Err(error) = update {
                        log::warn!(
                            "reaper {renderer_id}: reject point {} update: {error}",
                            identity.release_point,
                        );
                        if error.contains("frame_armed") {
                            cancelled.store(true, Ordering::Release);
                            let consumer = poison_consumer.expect("frame_armed error has consumer");
                            let _ = release_events.send(ReleaseEvent::GenerationPoisoned {
                                consumer,
                                reason: error.to_string(),
                            });
                            return;
                        }
                        continue;
                    }

                    if entry.ready() {
                        let bucket = buckets
                            .remove(&identity.release_point)
                            .expect("ready bucket remains registered");
                        dispatch_bucket_wait(
                            drm,
                            &renderer_id,
                            bucket,
                            completion_tx.clone(),
                            Arc::clone(&cancelled),
                            Arc::clone(&closed_sessions),
                        );
                    }
                }
                maybe_completion = completion_rx.recv() => {
                    let Some(completion) = maybe_completion else {
                        continue;
                    };
                    match completion {
                        WaitCompletion::Resolved { identity, handle, consumers } => {
                            for consumer in consumers {
                                let _ = release_events.send(ReleaseEvent::Resolved { consumer });
                            }
                            publish_resolved_release(
                                drm,
                                &renderer_id,
                                &release_syncobj,
                                &mut producer_handle,
                                &mut frontier,
                                identity.release_point,
                                handle,
                            );
                        }
                        WaitCompletion::Failed { identity, error } => {
                            log::error!(
                                "reaper {renderer_id}: wait point {} failed without releasing ownership: {error}",
                                identity.release_point,
                            );
                        }
                        WaitCompletion::Poisoned { consumer, error } => {
                            cancelled.store(true, Ordering::Release);
                            let _ = release_events.send(ReleaseEvent::GenerationPoisoned {
                                consumer,
                                reason: error,
                            });
                            return;
                        }
                        WaitCompletion::Cancelled => {}
                    }
                }
            }
        }
    });
}

fn dispatch_bucket_wait(
    drm: &'static DrmDevice,
    renderer_id: &str,
    bucket: Bucket,
    completion_tx: mpsc::UnboundedSender<WaitCompletion>,
    cancelled: Arc<AtomicBool>,
    closed_sessions: Arc<StdMutex<HashSet<DisplaySessionId>>>,
) {
    let identity = bucket.identity;
    let waits = bucket.into_waits();
    if waits.is_empty() {
        let result = drm
            .create_binary_syncobj()
            .and_then(|handle| drm.signal(&handle).map(|()| handle));
        match result {
            Ok(handle) => {
                let _ = completion_tx.send(WaitCompletion::Resolved {
                    identity,
                    handle,
                    consumers: Vec::new(),
                });
            }
            Err(error) => {
                let _ = completion_tx.send(WaitCompletion::Failed {
                    identity,
                    error: error.to_string(),
                });
            }
        }
        return;
    }

    let renderer_id = renderer_id.to_owned();
    let poison_consumer = waits
        .first()
        .map(|wait| wait.consumer.clone())
        .expect("non-empty release waits have a consumer");
    tokio::spawn(async move {
        let join = tokio::task::spawn_blocking(move || {
            wait_for_real_release(drm, waits, &cancelled, &closed_sessions)
        })
        .await;
        let completion = match join {
            Ok(Ok((handle, consumers))) => WaitCompletion::Resolved {
                identity,
                handle,
                consumers,
            },
            Ok(Err(WaitError::Cancelled)) => WaitCompletion::Cancelled,
            Ok(Err(WaitError::Io(error))) => WaitCompletion::Poisoned {
                consumer: poison_consumer.clone(),
                error: format!("wait release syncobj: {error}"),
            },
            Ok(Err(WaitError::Poisoned { consumer, error })) => {
                WaitCompletion::Poisoned { consumer, error }
            }
            Err(error) => WaitCompletion::Poisoned {
                consumer: poison_consumer,
                error: format!("wait worker for {renderer_id} panicked: {error}"),
            },
        };
        let _ = completion_tx.send(completion);
    });
}

enum WaitError {
    Cancelled,
    Io(std::io::Error),
    Poisoned {
        consumer: FrameConsumerIdentity,
        error: String,
    },
}

fn wait_for_real_release(
    drm: &'static DrmDevice,
    mut waits: Vec<ConsumerWait>,
    cancelled: &AtomicBool,
    closed_sessions: &StdMutex<HashSet<DisplaySessionId>>,
) -> Result<(SyncobjHandle, Vec<FrameConsumerIdentity>), WaitError> {
    let mut resolved_consumers = Vec::new();
    loop {
        if cancelled.load(Ordering::Acquire) {
            return Err(WaitError::Cancelled);
        }
        let closed = closed_sessions
            .lock()
            .map_err(|_| WaitError::Io(std::io::Error::other("closed session set poisoned")))?;
        let mut retained = Vec::with_capacity(waits.len());
        for mut wait in waits {
            if wait.legacy && closed.contains(&wait.consumer.display_session_id) {
                match drm.binary_syncobj_state(&wait.handle) {
                    Ok(BinarySyncobjState::Unsubmitted) => {
                        resolved_consumers.push(wait.consumer);
                        continue;
                    }
                    Ok(BinarySyncobjState::Pending | BinarySyncobjState::Signaled) => {
                        wait.legacy = false;
                    }
                    Err(error) => {
                        return Err(WaitError::Poisoned {
                            consumer: wait.consumer,
                            error: format!("classify legacy release on session close: {error}"),
                        });
                    }
                }
            }
            retained.push(wait);
        }
        drop(closed);
        waits = retained;
        if waits.is_empty() {
            let handle = drm.create_binary_syncobj().map_err(WaitError::Io)?;
            drm.signal(&handle).map_err(WaitError::Io)?;
            return Ok((handle, resolved_consumers));
        }
        let timeout = monotonic_deadline(CANCEL_POLL_INTERVAL).map_err(WaitError::Io)?;
        let refs: Vec<&SyncobjHandle> = waits.iter().map(|wait| &wait.handle).collect();
        match drm.wait_handles_signaled(&refs, timeout) {
            Ok(()) => {
                resolved_consumers.extend(waits.iter().map(|wait| wait.consumer.clone()));
                let handle = waits
                    .into_iter()
                    .next()
                    .map(|wait| wait.handle)
                    .ok_or_else(|| WaitError::Io(std::io::Error::other("empty release wait")))?;
                return Ok((handle, resolved_consumers));
            }
            Err(error) if matches!(error.raw_os_error(), Some(libc::ETIME) | Some(libc::EINTR)) => {
                continue;
            }
            Err(error) => return Err(WaitError::Io(error)),
        }
    }
}

fn monotonic_deadline(after: Duration) -> std::io::Result<i64> {
    let mut ts: libc::timespec = unsafe { std::mem::zeroed() };
    if unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    (ts.tv_sec as i64)
        .checked_mul(1_000_000_000)
        .and_then(|seconds| seconds.checked_add(ts.tv_nsec as i64))
        .and_then(|now| now.checked_add(after.as_nanos() as i64))
        .ok_or_else(|| std::io::Error::other("monotonic deadline overflow"))
}

fn dup_release_syncobj_fd(slot: &StdMutex<Option<OwnedFd>>) -> Option<OwnedFd> {
    let guard = slot.lock().ok()?;
    let fd = guard.as_ref()?;
    let dup_raw = nix::unistd::dup(fd.as_raw_fd()).ok()?;
    Some(unsafe { OwnedFd::from_raw_fd(dup_raw) })
}

fn ensure_producer_handle(
    drm: &'static DrmDevice,
    renderer_id: &str,
    release_syncobj: &StdMutex<Option<OwnedFd>>,
    producer_handle: &mut Option<SyncobjHandle>,
    release_point: u64,
) -> bool {
    if producer_handle.is_some() {
        return true;
    }
    let Some(fd) = dup_release_syncobj_fd(release_syncobj) else {
        log::warn!(
            "reaper {renderer_id}: cannot publish point {release_point}; producer has not sent ReleaseSyncobj"
        );
        return false;
    };
    match drm.fd_to_handle(&fd) {
        Ok(handle) => {
            *producer_handle = Some(handle);
            log::info!("reaper {renderer_id}: imported release_syncobj");
            true
        }
        Err(error) => {
            log::warn!("reaper {renderer_id}: DRM_IOCTL_SYNCOBJ_FD_TO_HANDLE failed: {error}");
            false
        }
    }
}

fn publish_resolved_release(
    drm: &'static DrmDevice,
    renderer_id: &str,
    release_syncobj: &StdMutex<Option<OwnedFd>>,
    producer_handle: &mut Option<SyncobjHandle>,
    frontier: &mut ReleaseFrontier<SyncobjHandle>,
    release_point: u64,
    handle: SyncobjHandle,
) {
    if let Err(error) = frontier.insert(release_point, handle) {
        log::warn!("reaper {renderer_id}: reject resolved point {release_point}: {error:?}");
        return;
    }

    if !ensure_producer_handle(
        drm,
        renderer_id,
        release_syncobj,
        producer_handle,
        release_point,
    ) {
        return;
    }
    let producer = producer_handle.as_ref().expect("producer handle imported");

    while let Some((point, release)) = frontier.next_ready() {
        if let Err(error) = drm.transfer(release, 0, producer, point) {
            log::warn!("reaper {renderer_id}: TRANSFER to release point {point} failed: {error}");
            return;
        }
        let _ = frontier
            .commit_next()
            .expect("next_ready guaranteed a matching frontier entry");
        log::trace!("reaper {renderer_id}: published release point {point}");
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::sync::atomic::AtomicBool;
    use std::sync::{Arc, Mutex as StdMutex};
    use std::time::Duration;

    use super::{
        dispatch_bucket_wait, register_frame, spawn_reaper, Bucket, FrameConsumerIdentity,
        FrameIdentity, MemberState, ReleaseEvent, ReleaseFrontier, ReleaseFrontierInsertError,
        WaitCompletion,
    };
    use crate::sync::DrmDevice;

    fn identity(point: u64) -> FrameIdentity {
        FrameIdentity {
            buffer_generation: 3,
            buffer_index: 1,
            release_point: point,
        }
    }

    fn consumer(frame: FrameIdentity, display_id: u64, session_id: u64) -> FrameConsumerIdentity {
        FrameConsumerIdentity {
            frame,
            renderer_id: "renderer-test".to_owned(),
            display_id,
            display_session_id: session_id,
            display_name: format!("display-{display_id}"),
            frame_seq: frame.release_point,
        }
    }

    fn set_member_state(bucket: &mut Bucket, member_index: u32, state: MemberState) {
        let identity = bucket.members.as_ref().expect("registered bucket")[member_index as usize]
            .consumer
            .clone();
        bucket
            .member_mut(member_index, &identity)
            .expect("registered member")
            .state = state;
    }

    #[test]
    fn release_frontier_never_publishes_across_a_gap() {
        let mut frontier = ReleaseFrontier::new();

        frontier.insert(3, "three").unwrap();
        frontier.insert(1, "one").unwrap();
        assert_eq!(frontier.next_ready(), Some((1, &"one")));
        assert_eq!(frontier.commit_next(), Some("one"));
        assert_eq!(frontier.next_ready(), None);

        frontier.insert(2, "two").unwrap();
        assert_eq!(frontier.next_ready(), Some((2, &"two")));
        assert_eq!(frontier.commit_next(), Some("two"));
        assert_eq!(frontier.next_ready(), Some((3, &"three")));
        assert_eq!(frontier.commit_next(), Some("three"));
    }

    #[test]
    fn release_frontier_rejects_invalid_duplicate_and_published_points() {
        let mut frontier = ReleaseFrontier::new();

        assert_eq!(
            frontier.insert(0, "zero"),
            Err(ReleaseFrontierInsertError::InvalidPoint)
        );
        frontier.insert(1, "one").unwrap();
        assert_eq!(
            frontier.insert(1, "replacement"),
            Err(ReleaseFrontierInsertError::Duplicate)
        );
        assert_eq!(frontier.commit_next(), Some("one"));
        assert_eq!(
            frontier.insert(1, "stale"),
            Err(ReleaseFrontierInsertError::AlreadyPublished)
        );
    }

    #[test]
    fn bucket_completes_only_after_every_unique_member() {
        let mut bucket = Bucket::new(identity(7));
        bucket
            .register(vec![
                consumer(identity(7), 1, 11),
                consumer(identity(7), 2, 12),
            ])
            .unwrap();
        set_member_state(&mut bucket, 0, MemberState::Skipped);
        assert!(!bucket.ready());
        set_member_state(&mut bucket, 1, MemberState::Skipped);
        assert!(bucket.ready());
    }

    #[tokio::test]
    async fn blocked_bucket_does_not_delay_other_completions_or_timeout() {
        let Ok(device) = DrmDevice::open_first_render_node() else {
            eprintln!("skip: no /dev/dri/renderD* available");
            return;
        };
        let device = Box::leak(Box::new(device));
        let blocked = device
            .create_binary_syncobj()
            .expect("create blocked handle");
        let blocked_fd = device
            .handle_to_fd(&blocked)
            .expect("export blocked handle");
        let blocked_signal = device
            .fd_to_handle(&blocked_fd)
            .expect("import blocked signal handle");
        let ready = device.create_binary_syncobj().expect("create ready handle");
        device.signal(&ready).expect("signal ready handle");

        let mut blocked_bucket = Bucket::new(identity(1));
        blocked_bucket
            .register(vec![consumer(identity(1), 1, 11)])
            .unwrap();
        set_member_state(
            &mut blocked_bucket,
            0,
            MemberState::Armed {
                handle: blocked,
                legacy: false,
            },
        );
        let mut ready_bucket = Bucket::new(identity(2));
        ready_bucket
            .register(vec![consumer(identity(2), 2, 12)])
            .unwrap();
        set_member_state(
            &mut ready_bucket,
            0,
            MemberState::Armed {
                handle: ready,
                legacy: false,
            },
        );

        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let cancelled = Arc::new(AtomicBool::new(false));
        let closed_sessions = Arc::new(StdMutex::new(HashSet::new()));
        dispatch_bucket_wait(
            device,
            "test",
            blocked_bucket,
            tx.clone(),
            Arc::clone(&cancelled),
            Arc::clone(&closed_sessions),
        );
        dispatch_bucket_wait(device, "test", ready_bucket, tx, cancelled, closed_sessions);

        let completion = tokio::time::timeout(Duration::from_millis(250), rx.recv())
            .await
            .expect("ready bucket completion delayed by blocked bucket")
            .expect("completion channel closed");
        assert!(matches!(
            completion,
            WaitCompletion::Resolved {
                identity: FrameIdentity {
                    release_point: 2,
                    ..
                },
                ..
            }
        ));

        assert!(
            tokio::time::timeout(Duration::from_millis(550), rx.recv())
                .await
                .is_err(),
            "blocked bucket completed at the removed ownership timeout"
        );
        device
            .signal(&blocked_signal)
            .expect("signal blocked consumer handle");
        let completion = tokio::time::timeout(Duration::from_secs(1), rx.recv())
            .await
            .expect("blocked bucket did not resume after a real release")
            .expect("completion channel closed");
        assert!(matches!(
            completion,
            WaitCompletion::Resolved {
                identity: FrameIdentity {
                    release_point: 1,
                    ..
                },
                ..
            }
        ));
    }

    #[tokio::test]
    async fn new_session_close_skips_only_unsubmitted_delivery() {
        let Ok(device) = DrmDevice::open_first_render_node() else {
            eprintln!("skip: no /dev/dri/renderD* available");
            return;
        };
        let device = Box::leak(Box::new(device));
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel();
        spawn_reaper(
            device,
            "renderer-test".to_owned(),
            Arc::new(StdMutex::new(None)),
            rx,
            event_tx,
        );

        let frame = identity(1);
        let identity = consumer(frame, 7, 70);
        let mut members = register_frame(&tx, frame, vec![identity.clone()]).unwrap();
        let member = members.pop().unwrap();
        let session = member.session();
        let release = device.create_binary_syncobj().unwrap();
        let _arm = member
            .delivered(release, true)
            .expect("new protocol arm token");
        session.close();

        let resolved = tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if let ReleaseEvent::Resolved { consumer } = event_rx.recv().await.unwrap() {
                    break consumer;
                }
            }
        })
        .await
        .expect("unsubmitted delivery was not resolved on session close");
        assert_eq!(resolved, identity);
        drop(tx);
    }

    #[tokio::test]
    async fn legacy_session_close_resolves_unsubmitted_wait() {
        let Ok(device) = DrmDevice::open_first_render_node() else {
            eprintln!("skip: no /dev/dri/renderD* available");
            return;
        };
        let device = Box::leak(Box::new(device));
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel();
        spawn_reaper(
            device,
            "renderer-test".to_owned(),
            Arc::new(StdMutex::new(None)),
            rx,
            event_tx,
        );

        let frame = identity(1);
        let identity = consumer(frame, 7, 71);
        let mut members = register_frame(&tx, frame, vec![identity.clone()]).unwrap();
        let member = members.pop().unwrap();
        let session = member.session();
        let release = device.create_binary_syncobj().unwrap();
        assert!(member.delivered(release, false).is_none());
        session.close();

        let resolved = tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if let ReleaseEvent::Resolved { consumer } = event_rx.recv().await.unwrap() {
                    break consumer;
                }
            }
        })
        .await
        .expect("legacy unsubmitted wait was not resolved on session close");
        assert_eq!(resolved, identity);
        drop(tx);
    }

    #[tokio::test]
    async fn lost_arm_ack_keeps_a_real_release_fence() {
        let Ok(device) = DrmDevice::open_first_render_node() else {
            eprintln!("skip: no /dev/dri/renderD* available");
            return;
        };
        let device = Box::leak(Box::new(device));
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel();
        spawn_reaper(
            device,
            "renderer-test".to_owned(),
            Arc::new(StdMutex::new(None)),
            rx,
            event_tx,
        );

        let frame = identity(1);
        let identity = consumer(frame, 8, 80);
        let mut members = register_frame(&tx, frame, vec![identity.clone()]).unwrap();
        let member = members.pop().unwrap();
        let session = member.session();
        let release = device.create_binary_syncobj().unwrap();
        device.signal(&release).unwrap();
        let _lost_arm = member
            .delivered(release, true)
            .expect("new protocol arm token");
        session.close();

        let mut saw_armed = false;
        let resolved = tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                match event_rx.recv().await.unwrap() {
                    ReleaseEvent::Waiting { consumer, state } if consumer == identity => {
                        if state == super::ReleaseWaitState::Armed {
                            saw_armed = true;
                        }
                    }
                    ReleaseEvent::Resolved { consumer } if consumer == identity => break consumer,
                    _ => {}
                }
            }
        })
        .await
        .expect("submitted release did not complete after lost arm ack");
        assert!(
            saw_armed,
            "session-close classification did not retain the real fence"
        );
        assert_eq!(resolved, identity);
        drop(tx);
    }

    #[tokio::test]
    async fn two_disconnected_consumers_do_not_block_two_live_consumers() {
        let Ok(device) = DrmDevice::open_first_render_node() else {
            eprintln!("skip: no /dev/dri/renderD* available");
            return;
        };
        let device = Box::leak(Box::new(device));
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel();
        spawn_reaper(
            device,
            "renderer-test".to_owned(),
            Arc::new(StdMutex::new(None)),
            rx,
            event_tx,
        );

        let frame = identity(1);
        let consumers = vec![
            consumer(frame, 1, 101),
            consumer(frame, 2, 102),
            consumer(frame, 3, 103),
            consumer(frame, 4, 104),
        ];
        let members = register_frame(&tx, frame, consumers.clone()).unwrap();
        for (index, member) in members.into_iter().enumerate() {
            let session = member.session();
            let release = device.create_binary_syncobj().unwrap();
            if index < 2 {
                let _lost_arm = member.delivered(release, true).unwrap();
                session.close();
            } else {
                device.signal(&release).unwrap();
                member.delivered(release, true).unwrap().arm();
            }
        }

        let resolved = tokio::time::timeout(Duration::from_secs(1), async {
            let mut resolved = HashSet::new();
            while resolved.len() < consumers.len() {
                if let ReleaseEvent::Resolved { consumer } = event_rx.recv().await.unwrap() {
                    resolved.insert(consumer.display_session_id);
                }
            }
            resolved
        })
        .await
        .expect("four-way fan-out did not resolve after two session exits");
        assert_eq!(resolved, HashSet::from([101, 102, 103, 104]));
        drop(tx);
    }

    #[tokio::test]
    async fn live_unarmed_delivery_has_no_ownership_timeout() {
        let Ok(device) = DrmDevice::open_first_render_node() else {
            eprintln!("skip: no /dev/dri/renderD* available");
            return;
        };
        let device = Box::leak(Box::new(device));
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel();
        spawn_reaper(
            device,
            "renderer-test".to_owned(),
            Arc::new(StdMutex::new(None)),
            rx,
            event_tx,
        );

        let frame = identity(1);
        let identity = consumer(frame, 9, 90);
        let mut members = register_frame(&tx, frame, vec![identity.clone()]).unwrap();
        let release = device.create_binary_syncobj().unwrap();
        let _arm = members.pop().unwrap().delivered(release, true).unwrap();

        let event = tokio::time::timeout(Duration::from_millis(700), async {
            loop {
                match event_rx.recv().await.unwrap() {
                    ReleaseEvent::Resolved { consumer } if consumer == identity => break true,
                    ReleaseEvent::GenerationPoisoned { consumer, .. } if consumer == identity => {
                        break true;
                    }
                    _ => {}
                }
            }
        })
        .await;
        assert!(
            event.is_err(),
            "live unarmed release was force-resolved or poisoned"
        );
        drop(tx);
    }
}
