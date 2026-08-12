use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use tokio::sync::{watch, Mutex};
use tokio::task::JoinHandle;

use super::cursor::PlaylistCursor;
use super::port::{ApplyPort, ApplyRequest, ApplySharing};
use super::session;
use crate::error::{Error, Result};
use crate::playback::rotation::{make_handle, RotationConfig, RotationHandle};
use crate::playback::Mode;
use crate::wallframe::scheduler::DisplayId;

#[derive(Debug, Clone)]
pub struct Definition {
    pub id: i64,
    pub mode: Mode,
    pub interval_secs: u32,
    pub items: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Activation {
    pub definition: Definition,
    pub display_ids: Vec<DisplayId>,
    pub resume_by_display: HashMap<DisplayId, String>,
    pub first_frame_timeout: Option<Duration>,
}

struct DisplayRotation {
    playlist_id: i64,
    cursor: Arc<Mutex<PlaylistCursor>>,
    handle: RotationHandle,
    deadline: Arc<std::sync::Mutex<Option<Instant>>>,
    task: JoinHandle<()>,
}

impl Drop for DisplayRotation {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[derive(Debug, Clone)]
pub struct DisplayStatus {
    pub display_id: DisplayId,
    pub active_id: i64,
    pub mode: Mode,
    pub interval_secs: u32,
    pub current_id: Option<String>,
    pub position: u32,
    pub count: u32,
    pub remaining_secs: u32,
}

#[derive(Default)]
pub struct Engine {
    inner: Mutex<HashMap<DisplayId, DisplayRotation>>,
    shared: session::Sessions,
}

impl Engine {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn owned_display_ids(&self) -> Vec<DisplayId> {
        let mut ids = self.inner.lock().await.keys().copied().collect::<Vec<_>>();
        ids.extend(self.shared.owned_display_ids().await);
        ids
    }

    pub async fn is_owned(&self, display_id: DisplayId) -> bool {
        self.inner.lock().await.contains_key(&display_id) || self.shared.is_owned(display_id).await
    }

    pub async fn activate(
        &self,
        activation: Activation,
        apply: ApplyPort,
        shutdown: watch::Receiver<bool>,
    ) -> Result<()> {
        let Activation {
            definition,
            display_ids,
            resume_by_display,
            first_frame_timeout,
        } = activation;
        if definition.items.is_empty() {
            return Err(Error::PlaylistInvalid("playlist has no wallpapers".into()));
        }
        if display_ids.is_empty() {
            return Ok(());
        }

        if display_ids.len() >= 2 {
            let resume_id = display_ids
                .iter()
                .find_map(|id| resume_by_display.get(id).cloned());
            for display_id in &display_ids {
                self.inner.lock().await.remove(display_id);
            }
            return self
                .shared
                .activate(
                    &display_ids,
                    definition,
                    resume_id,
                    first_frame_timeout,
                    apply,
                    shutdown,
                )
                .await;
        }

        let seed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos() as u64)
            .unwrap_or(1)
            .max(1);
        for display_id in display_ids {
            self.activate_one(
                display_id,
                definition.clone(),
                resume_by_display.get(&display_id).cloned(),
                first_frame_timeout,
                seed,
                apply.clone(),
                shutdown.clone(),
            )
            .await?;
        }
        Ok(())
    }

    async fn activate_one(
        &self,
        display_id: DisplayId,
        definition: Definition,
        resume_id: Option<String>,
        first_frame_timeout: Option<Duration>,
        seed: u64,
        apply: ApplyPort,
        shutdown: watch::Receiver<bool>,
    ) -> Result<()> {
        let cursor = Arc::new(Mutex::new(PlaylistCursor::new(
            definition.items,
            definition.mode,
            seed,
        )));
        let (handle, rx) = make_handle();
        handle.set_interval(definition.interval_secs);
        let deadline = Arc::new(std::sync::Mutex::new(None));
        let first = {
            let mut cursor = cursor.lock().await;
            match resume_id {
                Some(id) if cursor.set_current(&id) => Some(id),
                _ => cursor.first(),
            }
        };
        let members = Arc::new(Mutex::new(vec![display_id]));
        let task = tokio::spawn(run_playlist_rotator(
            cursor.clone(),
            deadline.clone(),
            members,
            ApplySharing::Independent,
            apply.clone(),
            rx,
            shutdown,
        ));
        {
            let mut inner = self.inner.lock().await;
            inner.remove(&display_id);
            inner.insert(
                display_id,
                DisplayRotation {
                    playlist_id: definition.id,
                    cursor,
                    handle,
                    deadline,
                    task,
                },
            );
        }
        self.shared.release(&[display_id]).await;

        if let Some(entry_id) = first {
            let result = apply
                .apply(ApplyRequest {
                    entry_id,
                    display_ids: vec![display_id],
                    sharing: ApplySharing::Independent,
                    first_frame_timeout,
                })
                .await;
            if first_frame_timeout.is_some() {
                if let Err(error) = result {
                    self.inner.lock().await.remove(&display_id);
                    return Err(error);
                }
            }
        }
        Ok(())
    }

    pub async fn attach_shared(
        &self,
        display_id: DisplayId,
        playlist_id: i64,
        first_frame_timeout: Duration,
        apply: ApplyPort,
    ) -> Result<bool> {
        if self.is_owned(display_id).await {
            return Ok(true);
        }
        self.shared
            .attach(display_id, playlist_id, first_frame_timeout, apply)
            .await
    }

    pub async fn deactivate(&self, display_ids: &[DisplayId]) {
        self.shared.release(display_ids).await;
        let mut inner = self.inner.lock().await;
        for display_id in display_ids {
            inner.remove(display_id);
        }
    }

    pub async fn jump_to(&self, playlist_id: i64, entry_id: &str, apply: ApplyPort) -> Result<()> {
        if self
            .shared
            .jump_to(playlist_id, entry_id, apply.clone())
            .await?
        {
            return Ok(());
        }
        let displays: Vec<_> = {
            let inner = self.inner.lock().await;
            inner
                .iter()
                .filter(|(_, rotation)| rotation.playlist_id == playlist_id)
                .map(|(display_id, rotation)| (*display_id, rotation.cursor.clone()))
                .collect()
        };
        for (display_id, cursor) in displays {
            if !cursor.lock().await.set_current(entry_id) {
                continue;
            }
            apply
                .apply(ApplyRequest {
                    entry_id: entry_id.to_owned(),
                    display_ids: vec![display_id],
                    sharing: ApplySharing::Independent,
                    first_frame_timeout: None,
                })
                .await?;
            if let Some(rotation) = self.inner.lock().await.get(&display_id) {
                rotation.handle.kick();
            }
        }
        Ok(())
    }

    pub async fn status(&self) -> Vec<DisplayStatus> {
        type Snapshot = (
            DisplayId,
            i64,
            Arc<Mutex<PlaylistCursor>>,
            u32,
            Arc<std::sync::Mutex<Option<Instant>>>,
        );
        let snapshot: Vec<Snapshot> = {
            let inner = self.inner.lock().await;
            inner
                .iter()
                .map(|(display_id, rotation)| {
                    (
                        *display_id,
                        rotation.playlist_id,
                        rotation.cursor.clone(),
                        rotation.handle.interval(),
                        rotation.deadline.clone(),
                    )
                })
                .collect()
        };
        let now = Instant::now();
        let mut statuses = Vec::with_capacity(snapshot.len());
        for (display_id, playlist_id, cursor, interval_secs, deadline) in snapshot {
            let remaining_secs = deadline
                .lock()
                .unwrap()
                .map(|deadline| deadline.saturating_duration_since(now).as_secs() as u32)
                .unwrap_or(0);
            let cursor = cursor.lock().await;
            statuses.push(DisplayStatus {
                display_id,
                active_id: playlist_id,
                mode: cursor.mode,
                interval_secs,
                current_id: cursor.current.clone(),
                position: cursor.pos as u32,
                count: cursor.len() as u32,
                remaining_secs,
            });
        }
        statuses.extend(self.shared.status().await);
        statuses
    }

    pub async fn drop_display(&self, display_id: DisplayId) {
        self.shared.drop_display(display_id).await;
        self.inner.lock().await.remove(&display_id);
    }

    pub async fn shutdown(&self) {
        self.shared.shutdown().await;
        self.inner.lock().await.clear();
    }

    pub async fn deactivate_playlist(&self, playlist_id: i64) -> Vec<DisplayId> {
        let mut displays = self.shared.deactivate_playlist(playlist_id).await;
        let independent: Vec<_> = {
            let inner = self.inner.lock().await;
            inner
                .iter()
                .filter(|(_, rotation)| rotation.playlist_id == playlist_id)
                .map(|(display_id, _)| *display_id)
                .collect()
        };
        self.deactivate(&independent).await;
        displays.extend(independent);
        displays
    }

    pub async fn rebuild(
        &self,
        definition: Definition,
        apply: ApplyPort,
    ) -> Result<Vec<DisplayId>> {
        if let Some(cleared) = self
            .shared
            .rebuild(definition.clone(), apply.clone())
            .await?
        {
            return Ok(cleared);
        }
        type Bound = (DisplayId, Arc<Mutex<PlaylistCursor>>, RotationHandle);
        let affected: Vec<Bound> = {
            let inner = self.inner.lock().await;
            inner
                .iter()
                .filter(|(_, rotation)| rotation.playlist_id == definition.id)
                .map(|(display_id, rotation)| {
                    (
                        *display_id,
                        rotation.cursor.clone(),
                        rotation.handle.clone(),
                    )
                })
                .collect()
        };
        if definition.items.is_empty() {
            let displays = affected
                .iter()
                .map(|(display_id, _, _)| *display_id)
                .collect::<Vec<_>>();
            self.deactivate(&displays).await;
            return Ok(displays);
        }
        for (display_id, cursor, handle) in affected {
            let (entry_id, needs_apply) = {
                let mut cursor = cursor.lock().await;
                let current = cursor.current.clone();
                cursor.items = definition.items.clone();
                cursor.mode = definition.mode;
                match current {
                    Some(id) if cursor.items.iter().any(|item| item == &id) => {
                        cursor.set_current(&id);
                        (id, false)
                    }
                    _ => (cursor.first().unwrap_or_default(), true),
                }
            };
            handle.set_interval(definition.interval_secs);
            if needs_apply && !entry_id.is_empty() {
                apply
                    .apply(ApplyRequest {
                        entry_id,
                        display_ids: vec![display_id],
                        sharing: ApplySharing::Independent,
                        first_frame_timeout: None,
                    })
                    .await?;
                handle.kick();
            }
        }
        Ok(Vec::new())
    }

    pub async fn set_interval(&self, playlist_id: i64, interval_secs: u32) {
        if self.shared.set_interval(playlist_id, interval_secs).await {
            return;
        }
        let inner = self.inner.lock().await;
        for rotation in inner.values() {
            if rotation.playlist_id == playlist_id {
                rotation.handle.set_interval(interval_secs);
            }
        }
    }
}

pub(super) async fn run_playlist_rotator(
    cursor: Arc<Mutex<PlaylistCursor>>,
    deadline: Arc<std::sync::Mutex<Option<Instant>>>,
    targets: Arc<Mutex<Vec<DisplayId>>>,
    sharing: ApplySharing,
    apply: ApplyPort,
    mut config: watch::Receiver<RotationConfig>,
    mut shutdown: watch::Receiver<bool>,
) {
    loop {
        let current = *config.borrow();
        if current.interval_secs == 0 {
            *deadline.lock().unwrap() = None;
            tokio::select! {
                _ = config.changed() => continue,
                changed = shutdown.changed() => if changed.is_err() || *shutdown.borrow() { break },
            }
        } else {
            let duration = Duration::from_secs(current.interval_secs as u64);
            *deadline.lock().unwrap() = Some(Instant::now() + duration);
            tokio::select! {
                _ = tokio::time::sleep(duration) => {
                    if config.borrow().interval_secs == 0 { continue; }
                    let display_ids = targets.lock().await.clone();
                    if display_ids.is_empty() { continue; }
                    if let Some(entry_id) = cursor.lock().await.next(1) {
                        if let Err(error) = apply.apply(ApplyRequest {
                            entry_id,
                            display_ids: display_ids.clone(),
                            sharing,
                            first_frame_timeout: None,
                        }).await {
                            log::warn!("playlist rotator displays={display_ids:?} apply failed: {error:#}");
                        }
                    }
                }
                _ = config.changed() => continue,
                changed = shutdown.changed() => if changed.is_err() || *shutdown.borrow() { break },
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn ownership_includes_shared_and_independent_sessions() {
        let engine = Engine::new();
        assert!(!engine.is_owned(1).await);
        engine
            .shared
            .seed_for_test(9, &[1], vec!["a".into()], Some("a".into()), 30)
            .await;
        assert!(engine.is_owned(1).await);

        let cursor = Arc::new(Mutex::new(PlaylistCursor::new(
            vec!["a".into()],
            Mode::Sequential,
            1,
        )));
        let (handle, _receiver) = make_handle();
        engine.inner.lock().await.insert(
            2,
            DisplayRotation {
                playlist_id: 9,
                cursor,
                handle,
                deadline: Arc::new(std::sync::Mutex::new(None)),
                task: tokio::spawn(async {}),
            },
        );
        assert!(engine.is_owned(2).await);
    }

    #[tokio::test]
    async fn activation_applies_without_application_context() {
        let engine = Engine::new();
        let applied = Arc::new(Mutex::new(Vec::new()));
        let applied_for_port = applied.clone();
        let port = ApplyPort::new(move |request| {
            let applied = applied_for_port.clone();
            async move {
                applied.lock().await.push(request);
                Ok(())
            }
        });
        let (_shutdown_tx, shutdown) = watch::channel(false);
        engine
            .activate(
                Activation {
                    definition: Definition {
                        id: 1,
                        mode: Mode::Sequential,
                        interval_secs: 0,
                        items: vec!["42".into()],
                    },
                    display_ids: vec![7],
                    resume_by_display: HashMap::new(),
                    first_frame_timeout: Some(Duration::from_secs(1)),
                },
                port,
                shutdown,
            )
            .await
            .unwrap();
        let requests = applied.lock().await;
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].entry_id, "42");
        assert_eq!(requests[0].display_ids, vec![7]);
        assert_eq!(requests[0].sharing, ApplySharing::Independent);
    }
}
