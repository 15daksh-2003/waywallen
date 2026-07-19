use std::collections::{BTreeMap, HashMap, HashSet};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use tokio::sync::mpsc;

use crate::sync::drm_syncobj::{DrmDevice, SyncobjHandle};

const CANCEL_POLL_INTERVAL: Duration = Duration::from_millis(250);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FrameIdentity {
    pub buffer_generation: u64,
    pub buffer_index: u32,
    pub release_point: u64,
}

pub(crate) enum FrameRecord {
    Register {
        identity: FrameIdentity,
        expected_members: u32,
    },
    Released {
        identity: FrameIdentity,
        member_index: u32,
        consumer_handle: SyncobjHandle,
    },
    Skipped {
        identity: FrameIdentity,
        member_index: u32,
    },
}

pub struct FrameConsumerMember {
    tx: Option<mpsc::UnboundedSender<FrameRecord>>,
    identity: FrameIdentity,
    member_index: u32,
}

impl FrameConsumerMember {
    fn new(
        tx: mpsc::UnboundedSender<FrameRecord>,
        identity: FrameIdentity,
        member_index: u32,
    ) -> Self {
        Self {
            tx: Some(tx),
            identity,
            member_index,
        }
    }

    pub fn released(mut self, consumer_handle: SyncobjHandle) {
        if let Some(tx) = self.tx.take() {
            let _ = tx.send(FrameRecord::Released {
                identity: self.identity,
                member_index: self.member_index,
                consumer_handle,
            });
        }
    }

    pub fn skip(mut self) {
        self.complete_skipped();
    }

    fn complete_skipped(&mut self) {
        if let Some(tx) = self.tx.take() {
            let _ = tx.send(FrameRecord::Skipped {
                identity: self.identity,
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
    expected_members: u32,
) -> Result<Vec<FrameConsumerMember>, &'static str> {
    tx.send(FrameRecord::Register {
        identity,
        expected_members,
    })
    .map_err(|_| "reaper channel closed")?;

    Ok((0..expected_members)
        .map(|member_index| FrameConsumerMember::new(tx.clone(), identity, member_index))
        .collect())
}

struct Bucket {
    identity: FrameIdentity,
    expected_members: Option<u32>,
    completed_members: u32,
    completed_indices: HashSet<u32>,
    handles: Vec<SyncobjHandle>,
}

impl Bucket {
    fn new(identity: FrameIdentity) -> Self {
        Self {
            identity,
            expected_members: None,
            completed_members: 0,
            completed_indices: HashSet::new(),
            handles: Vec::new(),
        }
    }

    fn register(&mut self, expected_members: u32) -> Result<(), &'static str> {
        match self.expected_members {
            Some(existing) if existing != expected_members => Err("member count mismatch"),
            Some(_) => Err("duplicate registration"),
            None => {
                self.expected_members = Some(expected_members);
                Ok(())
            }
        }
    }

    fn complete(
        &mut self,
        member_index: u32,
        handle: Option<SyncobjHandle>,
    ) -> Result<(), &'static str> {
        if let Some(expected) = self.expected_members {
            if member_index >= expected {
                return Err("member index out of range");
            }
        }
        if !self.completed_indices.insert(member_index) {
            return Err("duplicate member completion");
        }
        self.completed_members = self.completed_members.saturating_add(1);
        if let Some(handle) = handle {
            self.handles.push(handle);
        }
        Ok(())
    }

    fn ready(&self) -> bool {
        self.expected_members
            .is_some_and(|expected| self.completed_members == expected)
    }
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
    },
    Failed {
        identity: FrameIdentity,
        error: String,
    },
    Cancelled,
}

pub(crate) fn spawn_reaper(
    drm: &'static DrmDevice,
    renderer_id: String,
    release_syncobj: Arc<StdMutex<Option<OwnedFd>>>,
    mut rx: mpsc::UnboundedReceiver<FrameRecord>,
) {
    tokio::spawn(async move {
        let mut producer_handle: Option<SyncobjHandle> = None;
        let mut buckets: HashMap<u64, Bucket> = HashMap::new();
        let mut frontier = ReleaseFrontier::new();
        let (completion_tx, mut completion_rx) = mpsc::unbounded_channel();
        let cancelled = Arc::new(AtomicBool::new(false));

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

                    let identity = match &record {
                        FrameRecord::Register { identity, .. }
                        | FrameRecord::Released { identity, .. }
                        | FrameRecord::Skipped { identity, .. } => *identity,
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

                    let update = match record {
                        FrameRecord::Register { expected_members, .. } => {
                            entry.register(expected_members)
                        }
                        FrameRecord::Released {
                            member_index,
                            consumer_handle,
                            ..
                        } => entry.complete(member_index, Some(consumer_handle)),
                        FrameRecord::Skipped { member_index, .. } => {
                            entry.complete(member_index, None)
                        }
                    };
                    if let Err(error) = update {
                        log::warn!(
                            "reaper {renderer_id}: reject point {} update: {error}",
                            identity.release_point,
                        );
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
                        );
                    }
                }
                maybe_completion = completion_rx.recv() => {
                    let Some(completion) = maybe_completion else {
                        continue;
                    };
                    match completion {
                        WaitCompletion::Resolved { identity, handle } => {
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
) {
    let identity = bucket.identity;
    if bucket.handles.is_empty() {
        let result = drm
            .create_binary_syncobj()
            .and_then(|handle| drm.signal(&handle).map(|()| handle));
        match result {
            Ok(handle) => {
                let _ = completion_tx.send(WaitCompletion::Resolved { identity, handle });
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
    tokio::spawn(async move {
        let join = tokio::task::spawn_blocking(move || {
            wait_for_real_release(drm, bucket.handles, &cancelled)
        })
        .await;
        let completion = match join {
            Ok(Ok(handle)) => WaitCompletion::Resolved { identity, handle },
            Ok(Err(WaitError::Cancelled)) => WaitCompletion::Cancelled,
            Ok(Err(WaitError::Io(error))) => WaitCompletion::Failed {
                identity,
                error: error.to_string(),
            },
            Err(error) => WaitCompletion::Failed {
                identity,
                error: format!("wait worker for {renderer_id} panicked: {error}"),
            },
        };
        let _ = completion_tx.send(completion);
    });
}

enum WaitError {
    Cancelled,
    Io(std::io::Error),
}

fn wait_for_real_release(
    drm: &'static DrmDevice,
    handles: Vec<SyncobjHandle>,
    cancelled: &AtomicBool,
) -> Result<SyncobjHandle, WaitError> {
    loop {
        if cancelled.load(Ordering::Acquire) {
            return Err(WaitError::Cancelled);
        }
        let timeout = monotonic_deadline(CANCEL_POLL_INTERVAL).map_err(WaitError::Io)?;
        let refs: Vec<&SyncobjHandle> = handles.iter().collect();
        match drm.wait_handles_signaled(&refs, timeout) {
            Ok(()) => {
                return handles
                    .into_iter()
                    .next()
                    .ok_or_else(|| WaitError::Io(std::io::Error::other("empty release wait")));
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
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;
    use std::time::Duration;

    use super::{
        dispatch_bucket_wait, Bucket, FrameIdentity, ReleaseFrontier, ReleaseFrontierInsertError,
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
        bucket.register(2).unwrap();
        bucket.complete(0, None).unwrap();
        assert!(!bucket.ready());
        assert_eq!(bucket.complete(0, None), Err("duplicate member completion"));
        bucket.complete(1, None).unwrap();
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
        blocked_bucket.register(1).unwrap();
        blocked_bucket.complete(0, Some(blocked)).unwrap();
        let mut ready_bucket = Bucket::new(identity(2));
        ready_bucket.register(1).unwrap();
        ready_bucket.complete(0, Some(ready)).unwrap();

        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let cancelled = Arc::new(AtomicBool::new(false));
        dispatch_bucket_wait(
            device,
            "test",
            blocked_bucket,
            tx.clone(),
            Arc::clone(&cancelled),
        );
        dispatch_bucket_wait(device, "test", ready_bucket, tx, cancelled);

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
}
