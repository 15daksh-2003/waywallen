use std::collections::{BTreeMap, HashMap};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use tokio::sync::mpsc;
use tokio::time::Instant;

use crate::sync::drm_syncobj::{DrmDevice, SyncobjHandle};

/// Maximum bucket age before force-flush when consumers lag.
/// Sized so a 60 fps producer has room for several frames.
const BUCKET_TIMEOUT: Duration = Duration::from_millis(500);

/// Per-handle wait deadline inside the bucket-flush ioctl.
/// Late consumers are force-signaled after this timeout.
const WAIT_TIMEOUT: Duration = Duration::from_millis(500);

/// Per-frame work item produced by `display::endpoint::forward_frame_ready`
/// and consumed by [`spawn_reaper`].
pub struct FrameRecord {
    pub buffer_generation: u64,
    pub buffer_index: u32,
    pub release_point: u64,
    /// `None` with a zero expected count means there are no recipients.
    /// With a non-zero expected count it registers the release point
    /// before endpoints start delivering consumer handles.
    pub consumer_handle: Option<SyncobjHandle>,
    /// Total fan-out width for this `release_point`.
    /// `0` means the release point can complete without consumers.
    pub expected_count: u32,
}

struct Bucket {
    buffer_generation: u64,
    buffer_index: u32,
    handles: Vec<SyncobjHandle>,
    expected: u32,
    deadline: Instant,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum ReleaseResolutionOutcome {
    ConsumerReleased = 0,
    NoConsumers = 1,
    Forced = 2,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReleaseResolution {
    pub buffer_generation: u64,
    pub buffer_index: u32,
    pub release_point: u64,
    pub outcome: ReleaseResolutionOutcome,
}

pub trait ReleaseResolutionPublisher: Send + Sync {
    fn publish(&self, resolution: ReleaseResolution) -> Result<(), String>;
}

struct ResolvedRelease {
    handle: SyncobjHandle,
    resolution: ReleaseResolution,
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

pub fn spawn_reaper(
    drm: &'static DrmDevice,
    renderer_id: String,
    release_syncobj: Arc<StdMutex<Option<OwnedFd>>>,
    resolution_publisher: Arc<dyn ReleaseResolutionPublisher>,
    mut rx: mpsc::UnboundedReceiver<FrameRecord>,
) {
    tokio::spawn(async move {
        let mut producer_handle: Option<SyncobjHandle> = None;
        let mut buckets: HashMap<u64, Bucket> = HashMap::new();
        let mut frontier = ReleaseFrontier::new();

        loop {
            // Earliest bucket deadline. None when there are no pending
            // buckets, in which case we just wait on the channel.
            let next_deadline = buckets.values().map(|b| b.deadline).min();

            tokio::select! {
                maybe_record = rx.recv() => {
                    let Some(record) = maybe_record else {
                        // Channel closed: every Sender clone is gone,
                        // so the renderer handle has been dropped.
                        if !buckets.is_empty() || frontier.pending_count() != 0 {
                            log::info!(
                                "reaper {renderer_id}: channel closed with {} pending bucket(s) and {} resolved point(s); dropping",
                                buckets.len(),
                                frontier.pending_count(),
                            );
                        }
                        drop(buckets);
                        log::info!("reaper {renderer_id}: exiting");
                        return;
                    };
                    let Some(consumer_handle) = record.consumer_handle else {
                        if record.expected_count == 0 {
                            if let Some(release) = resolve_empty_release(drm, &renderer_id, &record) {
                                publish_resolved_release(
                                    drm, &renderer_id, &release_syncobj, &mut producer_handle,
                                    &resolution_publisher, &mut frontier, record.release_point, release,
                                ).await;
                            }
                        } else {
                            let entry = buckets.entry(record.release_point).or_insert_with(|| Bucket {
                                buffer_generation: record.buffer_generation,
                                buffer_index: record.buffer_index,
                                handles: Vec::new(),
                                expected: record.expected_count,
                                deadline: Instant::now() + BUCKET_TIMEOUT,
                            });
                            if entry.buffer_generation != record.buffer_generation ||
                                entry.buffer_index != record.buffer_index
                            {
                                log::warn!(
                                    "reaper {renderer_id}: reject point {} registration identity mismatch",
                                    record.release_point,
                                );
                            } else {
                                entry.expected = entry.expected.max(record.expected_count);
                            }
                        }
                        continue;
                    };
                    let entry = buckets.entry(record.release_point).or_insert_with(|| {
                        Bucket {
                            buffer_generation: record.buffer_generation,
                            buffer_index: record.buffer_index,
                            handles: Vec::new(),
                            expected: record.expected_count,
                            deadline: Instant::now() + BUCKET_TIMEOUT,
                        }
                    });
                    if entry.buffer_generation != record.buffer_generation ||
                        entry.buffer_index != record.buffer_index
                    {
                        log::warn!(
                            "reaper {renderer_id}: reject point {} identity mismatch: \
                             existing generation={} index={}, received generation={} index={}",
                            record.release_point,
                            entry.buffer_generation,
                            entry.buffer_index,
                            record.buffer_generation,
                            record.buffer_index,
                        );
                        continue;
                    }
                    // Defensive: if a later record reports a different
                    // expected_count, use the wider fan-out.
                    entry.expected = entry.expected.max(record.expected_count);
                    entry.handles.push(consumer_handle);
                    if entry.handles.len() as u32 >= entry.expected {
                        let bucket = buckets.remove(&record.release_point).unwrap();
                        if let Some(release) = resolve_bucket(
                            drm, &renderer_id, record.release_point, bucket,
                        ).await {
                            publish_resolved_release(
                                drm, &renderer_id, &release_syncobj, &mut producer_handle,
                                &resolution_publisher, &mut frontier, record.release_point, release,
                            ).await;
                        }
                    }
                }
                _ = sleep_until_or_pending(next_deadline) => {
                    // Snapshot expired keys first so the map can be
                    // mutated during flushing.
                    let now = Instant::now();
                    let expired: Vec<u64> = buckets
                        .iter()
                        .filter(|(_, b)| b.deadline <= now)
                        .map(|(p, _)| *p)
                        .collect();
                    for point in expired {
                        let bucket = buckets.remove(&point).unwrap();
                        log::warn!(
                            "reaper {renderer_id}: bucket point {point} timed out \
                             with {}/{} consumer signals — force-flushing",
                            bucket.handles.len(),
                            bucket.expected,
                        );
                        if let Some(release) = resolve_bucket(
                            drm, &renderer_id, point, bucket,
                        ).await {
                            publish_resolved_release(
                                drm, &renderer_id, &release_syncobj, &mut producer_handle,
                                &resolution_publisher, &mut frontier, point, release,
                            ).await;
                        }
                    }
                }
            }
        }
    });
}

/// Sleep until `deadline`. If `deadline` is `None`, never resolve —
/// the surrounding `tokio::select!` falls through to the recv arm.
async fn sleep_until_or_pending(deadline: Option<Instant>) {
    match deadline {
        Some(d) => tokio::time::sleep_until(d).await,
        None => std::future::pending::<()>().await,
    }
}

/// Duplicate the producer release_syncobj fd out of the shared slot.
/// The returned fd is owned by the caller.
fn dup_release_syncobj_fd(slot: &StdMutex<Option<OwnedFd>>) -> Option<OwnedFd> {
    let guard = slot.lock().ok()?;
    let fd = guard.as_ref()?;
    let dup_raw = nix::unistd::dup(fd.as_raw_fd()).ok()?;
    // SAFETY: nix::unistd::dup returned a fresh fd we now own.
    Some(unsafe { OwnedFd::from_raw_fd(dup_raw) })
}

/// Lazy-import the producer's release_syncobj into our handle cache.
/// Returns true if `producer_handle` is `Some` after this call.
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
            "reaper {renderer_id}: dropping point {release_point} — \
             producer hasn't sent ReleaseSyncobj yet"
        );
        return false;
    };
    match drm.fd_to_handle(&fd) {
        Ok(h) => {
            *producer_handle = Some(h);
            log::info!("reaper {renderer_id}: imported release_syncobj");
            true
        }
        Err(e) => {
            log::warn!("reaper {renderer_id}: DRM_IOCTL_SYNCOBJ_FD_TO_HANDLE failed: {e}");
            false
        }
    }
}

async fn publish_resolved_release(
    drm: &'static DrmDevice,
    renderer_id: &str,
    release_syncobj: &StdMutex<Option<OwnedFd>>,
    producer_handle: &mut Option<SyncobjHandle>,
    resolution_publisher: &Arc<dyn ReleaseResolutionPublisher>,
    frontier: &mut ReleaseFrontier<ResolvedRelease>,
    release_point: u64,
    release: ResolvedRelease,
) {
    if let Err(error) = frontier.insert(release_point, release) {
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
    let producer = producer_handle.as_ref().expect("set above");

    while let Some((point, resolution)) = frontier
        .next_ready()
        .map(|(point, release)| (point, release.resolution))
    {
        if resolution.outcome == ReleaseResolutionOutcome::Forced {
            log::warn!("reaper {renderer_id}: publishing forced release point {point}");
        }
        let publisher = Arc::clone(resolution_publisher);
        let publish_result =
            tokio::task::spawn_blocking(move || publisher.publish(resolution)).await;
        match publish_result {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                log::warn!(
                    "reaper {renderer_id}: release outcome notification for point {point} \
                     failed: {error}"
                );
                return;
            }
            Err(error) => {
                log::warn!(
                    "reaper {renderer_id}: release outcome notification task for point {point} \
                     failed: {error}"
                );
                return;
            }
        }
        let release = frontier
            .next_ready()
            .map(|(_, release)| release)
            .expect("frontier cannot change while publishing a release outcome");
        if let Err(error) = drm.transfer(&release.handle, 0, producer, point) {
            log::warn!("reaper {renderer_id}: TRANSFER to release point {point} failed: {error}");
            return;
        }
        let _published = frontier
            .commit_next()
            .expect("next_ready guaranteed a matching frontier entry");
        log::trace!("reaper {renderer_id}: published release point {point}");
    }
}

/// Used when a frame has zero recipients. The returned signaled fence is
/// published only when every preceding release point has also resolved.
fn resolve_empty_release(
    drm: &'static DrmDevice,
    renderer_id: &str,
    record: &FrameRecord,
) -> Option<ResolvedRelease> {
    let release_point = record.release_point;
    let placeholder = match drm.create_binary_syncobj() {
        Ok(h) => h,
        Err(e) => {
            log::warn!(
                "reaper {renderer_id}: advance point {release_point}: create_binary_syncobj: {e}"
            );
            return None;
        }
    };
    if let Err(e) = drm.signal(&placeholder) {
        log::warn!("reaper {renderer_id}: advance point {release_point}: SIGNAL: {e}");
        return None;
    }
    Some(ResolvedRelease {
        handle: placeholder,
        resolution: ReleaseResolution {
            buffer_generation: record.buffer_generation,
            buffer_index: record.buffer_index,
            release_point,
            outcome: ReleaseResolutionOutcome::NoConsumers,
        },
    })
}

/// Wait for every handle in `bucket`, force-signaling stragglers.
/// The returned fence is not transferred to the producer timeline until
/// every preceding release point has also resolved.
async fn resolve_bucket(
    drm: &'static DrmDevice,
    renderer_id: &str,
    release_point: u64,
    mut bucket: Bucket,
) -> Option<ResolvedRelease> {
    if bucket.handles.is_empty() {
        let placeholder = match drm.create_binary_syncobj() {
            Ok(handle) => handle,
            Err(error) => {
                log::warn!(
                    "reaper {renderer_id}: create forced placeholder for point \
                     {release_point} failed: {error}"
                );
                return None;
            }
        };
        if let Err(error) = drm.signal(&placeholder) {
            log::warn!(
                "reaper {renderer_id}: signal forced placeholder for point \
                 {release_point} failed: {error}"
            );
            return None;
        }
        return Some(ResolvedRelease {
            handle: placeholder,
            resolution: ReleaseResolution {
                buffer_generation: bucket.buffer_generation,
                buffer_index: bucket.buffer_index,
                release_point,
                outcome: ReleaseResolutionOutcome::Forced,
            },
        });
    }

    // 1+2. Wait for all consumer signals; force-signal stragglers.
    // wait_handles_signaled wants ABSOLUTE CLOCK_MONOTONIC.
    let timeout_nsec = {
        let mut ts: libc::timespec = unsafe { std::mem::zeroed() };
        let ok = unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) } == 0;
        if !ok {
            i64::MAX
        } else {
            (ts.tv_sec as i64)
                .checked_mul(1_000_000_000)
                .and_then(|s| s.checked_add(ts.tv_nsec as i64))
                .and_then(|now| now.checked_add(WAIT_TIMEOUT.as_nanos() as i64))
                .unwrap_or(i64::MAX)
        }
    };

    // Move ownership across spawn_blocking boundary to keep handles
    // alive on the blocking thread; ioctl needs &SyncobjHandle so we
    let actual_count = bucket.handles.len() as u32;
    let incomplete = actual_count < bucket.expected;
    let handles_for_blocking = std::mem::take(&mut bucket.handles);
    let join = tokio::task::spawn_blocking(move || {
        let refs: Vec<&SyncobjHandle> = handles_for_blocking.iter().collect();
        let res = drm.wait_handles_signaled(&refs, timeout_nsec);
        (res, handles_for_blocking)
    })
    .await;
    let (wait_result, handles) = match join {
        Ok(pair) => pair,
        Err(e) => {
            log::warn!("reaper {renderer_id}: wait task panicked: {e}");
            return None;
        }
    };

    let resolution_outcome =
        classify_release_outcome(bucket.expected, actual_count, wait_result.is_ok());
    if incomplete {
        log::warn!(
            "reaper {renderer_id}: point {release_point} missing consumer records \
             ({actual_count}/{}); marking release forced",
            bucket.expected,
        );
    }
    if let Err(e) = wait_result {
        log::warn!(
            "reaper {renderer_id}: wait point {release_point} timed out / errored ({e}); \
             force-signaling stragglers"
        );
        for h in &handles {
            // SIGNAL is a CPU-side mark; cheap and cannot fail in any
            // meaningful way for our handles.
            if let Err(se) = drm.signal(h) {
                log::warn!("reaper {renderer_id}: force SIGNAL failed: {se}");
            }
        }
    }

    // Single-consumer fast path: keep the consumer fence directly.
    let n = handles.len();
    if n == 1 {
        return handles.into_iter().next().map(|handle| ResolvedRelease {
            handle,
            resolution: ReleaseResolution {
                buffer_generation: bucket.buffer_generation,
                buffer_index: bucket.buffer_index,
                release_point,
                outcome: resolution_outcome,
            },
        });
    }

    // Fan-out merge:
    //   3a. EXPORT_SYNC_FILE on each consumer handle.
    let mut sync_files: Vec<std::os::fd::OwnedFd> = Vec::with_capacity(n);
    let mut export_failed = false;
    for h in &handles {
        match drm.export_sync_file(h) {
            Ok(fd) => sync_files.push(fd),
            Err(e) => {
                log::warn!(
                    "reaper {renderer_id}: EXPORT_SYNC_FILE on point {release_point} failed: {e}"
                );
                export_failed = true;
                break;
            }
        }
    }
    if export_failed || sync_files.is_empty() {
        // Keep the existing compatibility fallback, but still route it
        // through the ordered publication frontier.
        return handles.into_iter().next().map(|handle| ResolvedRelease {
            handle,
            resolution: ReleaseResolution {
                buffer_generation: bucket.buffer_generation,
                buffer_index: bucket.buffer_index,
                release_point,
                outcome: ReleaseResolutionOutcome::Forced,
            },
        });
    }

    let merged = sync_files
        .into_iter()
        .reduce(|a, b| match crate::sync::merge_sync_files(&a, &b) {
            Ok(m) => m,
            Err(e) => {
                log::warn!(
                    "reaper {renderer_id}: SYNC_IOC_MERGE on point {release_point} failed: {e}; \
                     dropping later fences"
                );
                a
            }
        })
        .expect("non-empty after empty-check above");

    let temp_handle = match drm.create_binary_syncobj() {
        Ok(h) => h,
        Err(e) => {
            log::warn!(
                "reaper {renderer_id}: create temp syncobj for point {release_point} failed: {e}"
            );
            return None;
        }
    };
    if let Err(e) = drm.import_sync_file(&temp_handle, &merged) {
        log::warn!("reaper {renderer_id}: IMPORT_SYNC_FILE for point {release_point} failed: {e}");
        return None;
    }
    log::trace!(
        "reaper {renderer_id}: resolved point {release_point} ({n} consumer fences merged)"
    );
    drop(handles);
    Some(ResolvedRelease {
        handle: temp_handle,
        resolution: ReleaseResolution {
            buffer_generation: bucket.buffer_generation,
            buffer_index: bucket.buffer_index,
            release_point,
            outcome: resolution_outcome,
        },
    })
}

fn classify_release_outcome(
    expected_count: u32,
    actual_count: u32,
    all_waits_succeeded: bool,
) -> ReleaseResolutionOutcome {
    if actual_count < expected_count || !all_waits_succeeded {
        ReleaseResolutionOutcome::Forced
    } else {
        ReleaseResolutionOutcome::ConsumerReleased
    }
}

#[cfg(test)]
mod tests {
    use super::{
        classify_release_outcome, ReleaseFrontier, ReleaseFrontierInsertError,
        ReleaseResolutionOutcome,
    };

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
        assert_eq!(frontier.next_ready(), None);
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
    fn release_outcome_requires_every_consumer_record_and_signal() {
        assert_eq!(
            classify_release_outcome(2, 2, true),
            ReleaseResolutionOutcome::ConsumerReleased
        );
        assert_eq!(
            classify_release_outcome(2, 1, true),
            ReleaseResolutionOutcome::Forced
        );
        assert_eq!(
            classify_release_outcome(2, 0, true),
            ReleaseResolutionOutcome::Forced
        );
        assert_eq!(
            classify_release_outcome(2, 2, false),
            ReleaseResolutionOutcome::Forced
        );
    }
}
