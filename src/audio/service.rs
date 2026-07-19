use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::watch;

use super::analyzer::{AudioSpectrumFrame, SpectrumAnalyzer};
use super::pulse::{AudioCaptureBackend, CaptureErrorKind, PulseCapture};
use crate::renderer_manager::{RendererEventKind, RendererManager};

const DEFAULT_IDLE_TIMEOUT: Duration = Duration::from_secs(30);
const DISPATCH_INTERVAL: Duration = Duration::from_millis(33);
const ACTIVE_POLL_INTERVAL: Duration = Duration::from_millis(5);
const INACTIVE_POLL_INTERVAL: Duration = Duration::from_millis(100);
const MAX_BACKOFF: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioServiceState {
    Closed,
    Starting,
    Active,
    WarmIdle,
    Backoff,
    Stopping,
}

enum WorkerCommand {
    Demand(bool),
    Shutdown,
}

pub struct AudioService {
    state: watch::Receiver<AudioServiceState>,
}

impl AudioService {
    pub fn start(manager: Arc<RendererManager>, mut shutdown: watch::Receiver<bool>) -> Self {
        let (command_tx, command_rx) = std::sync::mpsc::channel();
        let (frame_tx, frame_rx) = watch::channel::<Option<AudioSpectrumFrame>>(None);
        let (state_tx, state_rx) = watch::channel(AudioServiceState::Closed);
        let worker = std::thread::spawn(move || {
            run_worker(command_rx, frame_tx, state_tx, DEFAULT_IDLE_TIMEOUT, || {
                PulseCapture::open()
                    .map(|capture| Box::new(capture) as Box<dyn AudioCaptureBackend>)
            });
        });

        let mut subscriptions = manager.subscribe_subscriptions();
        tokio::spawn(async move {
            let initial_demand = !subscriptions
                .borrow()
                .subscribers(RendererEventKind::Audio)
                .is_empty();
            let _ = command_tx.send(WorkerCommand::Demand(initial_demand));
            let mut frame_rx = frame_rx;
            let mut dispatch = tokio::time::interval(DISPATCH_INTERVAL);
            dispatch.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            let mut last_sent = None;

            loop {
                tokio::select! {
                    changed = subscriptions.changed() => {
                        if changed.is_err() {
                            break;
                        }
                        let demanded = !subscriptions
                            .borrow()
                            .subscribers(RendererEventKind::Audio)
                            .is_empty();
                        if !demanded {
                            last_sent = None;
                        }
                        if command_tx.send(WorkerCommand::Demand(demanded)).is_err() {
                            break;
                        }
                    }
                    _ = dispatch.tick() => {
                        let Some(frame) = frame_rx.borrow_and_update().clone() else {
                            continue;
                        };
                        let identity = (frame.generation, frame.sequence);
                        if last_sent == Some(identity) {
                            continue;
                        }
                        last_sent = Some(identity);
                        let targets = manager
                            .subscription_snapshot()
                            .subscribers(RendererEventKind::Audio);
                        for (id, revision) in targets {
                            if let Err(error) = manager
                                .send_audio_spectrum_latest(
                                    &id,
                                    revision,
                                    frame.generation,
                                    frame.sequence,
                                    frame.captured_at_ns,
                                    frame.left.to_vec(),
                                    frame.right.to_vec(),
                                )
                                .await
                            {
                                log::debug!("renderer {id}: audio dispatch dropped: {error}");
                            }
                        }
                    }
                    result = shutdown.changed() => {
                        if result.is_err() || *shutdown.borrow() {
                            break;
                        }
                    }
                }
            }

            let _ = command_tx.send(WorkerCommand::Shutdown);
            let _ = tokio::task::spawn_blocking(move || worker.join()).await;
        });

        Self { state: state_rx }
    }

    pub fn state(&self) -> AudioServiceState {
        *self.state.borrow()
    }
}

fn run_worker<F>(
    commands: std::sync::mpsc::Receiver<WorkerCommand>,
    frames: watch::Sender<Option<AudioSpectrumFrame>>,
    states: watch::Sender<AudioServiceState>,
    idle_timeout: Duration,
    mut open_backend: F,
) where
    F: FnMut() -> Result<Box<dyn AudioCaptureBackend>, super::pulse::CaptureError>,
{
    let mut backend: Option<Box<dyn AudioCaptureBackend>> = None;
    let mut analyzer = SpectrumAnalyzer::default();
    let mut demand = false;
    let mut idle_deadline: Option<Instant> = None;
    let mut retry_deadline: Option<Instant> = None;
    let mut backoff = Duration::from_secs(1);
    let mut unavailable_latched = false;
    let mut samples = [0.0f32; 2048];

    loop {
        let timeout = if demand {
            ACTIVE_POLL_INTERVAL
        } else {
            idle_deadline
                .map(|deadline| deadline.saturating_duration_since(Instant::now()))
                .unwrap_or(INACTIVE_POLL_INTERVAL)
                .min(INACTIVE_POLL_INTERVAL)
        };
        match commands.recv_timeout(timeout) {
            Ok(WorkerCommand::Demand(next)) => {
                if next == demand {
                    continue;
                }
                demand = next;
                frames.send_replace(None);
                analyzer.clear();
                retry_deadline = None;
                backoff = Duration::from_secs(1);
                if demand {
                    idle_deadline = None;
                    unavailable_latched = false;
                    if let Some(capture) = backend.as_mut() {
                        capture.discard();
                        analyzer.reset(capture.generation());
                        states.send_replace(AudioServiceState::Active);
                    }
                } else if backend.is_some() {
                    idle_deadline = Some(Instant::now() + idle_timeout);
                    states.send_replace(AudioServiceState::WarmIdle);
                } else {
                    states.send_replace(AudioServiceState::Closed);
                }
            }
            Ok(WorkerCommand::Shutdown) | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                states.send_replace(AudioServiceState::Stopping);
                frames.send_replace(None);
                drop(backend.take());
                states.send_replace(AudioServiceState::Closed);
                return;
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
        }

        if !demand {
            if idle_deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                drop(backend.take());
                idle_deadline = None;
                states.send_replace(AudioServiceState::Closed);
            }
            continue;
        }

        if backend.is_none()
            && !unavailable_latched
            && retry_deadline.is_none_or(|deadline| Instant::now() >= deadline)
        {
            states.send_replace(AudioServiceState::Starting);
            match open_backend() {
                Ok(mut capture) => {
                    capture.discard();
                    analyzer.reset(capture.generation());
                    backend = Some(capture);
                    retry_deadline = None;
                    backoff = Duration::from_secs(1);
                    states.send_replace(AudioServiceState::Active);
                    log::info!("audio response: PulseAudio capture active");
                }
                Err(error) => {
                    let permanent = matches!(
                        error.kind,
                        CaptureErrorKind::LibraryUnavailable | CaptureErrorKind::MissingSymbol
                    );
                    log::warn!("audio response unavailable: {error}");
                    unavailable_latched = permanent;
                    if permanent {
                        states.send_replace(AudioServiceState::Closed);
                    } else {
                        retry_deadline = Some(Instant::now() + backoff);
                        backoff = (backoff * 2).min(MAX_BACKOFF);
                        states.send_replace(AudioServiceState::Backoff);
                    }
                }
            }
        }

        let Some(capture) = backend.as_mut() else {
            continue;
        };
        let generation = capture.generation();
        match capture.read(&mut samples) {
            Ok(0) => {}
            Ok(frame_count) => {
                let captured_at_ns = monotonic_now_ns();
                if let Some(frame) = analyzer.ingest_interleaved(
                    generation,
                    captured_at_ns,
                    &samples[..frame_count * 2],
                ) {
                    frames.send_replace(Some(frame));
                }
            }
            Err(error) => {
                log::warn!("audio response capture failed: {error}");
                frames.send_replace(None);
                drop(backend.take());
                analyzer.clear();
                retry_deadline = Some(Instant::now() + backoff);
                backoff = (backoff * 2).min(MAX_BACKOFF);
                states.send_replace(AudioServiceState::Backoff);
            }
        }
    }
}

fn monotonic_now_ns() -> u64 {
    let mut time = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    if unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut time) } != 0 {
        return 0;
    }
    time.tv_sec as u64 * 1_000_000_000 + time.tv_nsec as u64
}

#[cfg(test)]
mod tests {
    use super::super::pulse::{CaptureError, CaptureErrorKind};
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct FakeCapture {
        generation: u64,
        stops: Arc<AtomicUsize>,
    }

    impl AudioCaptureBackend for FakeCapture {
        fn generation(&self) -> u64 {
            self.generation
        }

        fn read(&mut self, _samples: &mut [f32]) -> Result<usize, CaptureError> {
            Ok(0)
        }

        fn discard(&mut self) {}
    }

    impl Drop for FakeCapture {
        fn drop(&mut self) {
            self.stops.fetch_add(1, Ordering::Relaxed);
        }
    }

    #[test]
    fn warm_idle_reuses_then_closes_capture() {
        let (commands_tx, commands_rx) = std::sync::mpsc::channel();
        let (frames_tx, _) = watch::channel(None);
        let (states_tx, states_rx) = watch::channel(AudioServiceState::Closed);
        let opens = Arc::new(AtomicUsize::new(0));
        let stops = Arc::new(AtomicUsize::new(0));
        let test_opens = Arc::clone(&opens);
        let test_stops = Arc::clone(&stops);
        let worker = std::thread::spawn(move || {
            run_worker(
                commands_rx,
                frames_tx,
                states_tx,
                Duration::from_millis(40),
                move || {
                    let generation = test_opens.fetch_add(1, Ordering::Relaxed) as u64 + 1;
                    Ok(Box::new(FakeCapture {
                        generation,
                        stops: Arc::clone(&test_stops),
                    }) as Box<dyn AudioCaptureBackend>)
                },
            )
        });

        commands_tx.send(WorkerCommand::Demand(true)).unwrap();
        std::thread::sleep(Duration::from_millis(15));
        assert_eq!(*states_rx.borrow(), AudioServiceState::Active);
        commands_tx.send(WorkerCommand::Demand(false)).unwrap();
        std::thread::sleep(Duration::from_millis(10));
        commands_tx.send(WorkerCommand::Demand(true)).unwrap();
        std::thread::sleep(Duration::from_millis(10));
        assert_eq!(opens.load(Ordering::Relaxed), 1);

        commands_tx.send(WorkerCommand::Demand(false)).unwrap();
        std::thread::sleep(Duration::from_millis(60));
        assert_eq!(*states_rx.borrow(), AudioServiceState::Closed);
        assert_eq!(stops.load(Ordering::Relaxed), 1);
        commands_tx.send(WorkerCommand::Shutdown).unwrap();
        worker.join().unwrap();
    }

    #[test]
    fn missing_library_is_latched_until_demand_cycles() {
        let (commands_tx, commands_rx) = std::sync::mpsc::channel();
        let (frames_tx, _) = watch::channel(None);
        let (states_tx, _) = watch::channel(AudioServiceState::Closed);
        let attempts = Arc::new(AtomicUsize::new(0));
        let test_attempts = Arc::clone(&attempts);
        let worker = std::thread::spawn(move || {
            run_worker(
                commands_rx,
                frames_tx,
                states_tx,
                Duration::from_millis(10),
                move || {
                    test_attempts.fetch_add(1, Ordering::Relaxed);
                    Err(CaptureError {
                        kind: CaptureErrorKind::LibraryUnavailable,
                        message: "missing".to_string(),
                    })
                },
            )
        });
        commands_tx.send(WorkerCommand::Demand(true)).unwrap();
        std::thread::sleep(Duration::from_millis(25));
        assert_eq!(attempts.load(Ordering::Relaxed), 1);
        commands_tx.send(WorkerCommand::Demand(false)).unwrap();
        commands_tx.send(WorkerCommand::Demand(true)).unwrap();
        std::thread::sleep(Duration::from_millis(15));
        assert_eq!(attempts.load(Ordering::Relaxed), 2);
        commands_tx.send(WorkerCommand::Shutdown).unwrap();
        worker.join().unwrap();
    }
}
