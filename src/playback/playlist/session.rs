use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::{watch, Mutex};
use tokio::task::JoinHandle;

use super::cursor::PlaylistCursor;
use super::engine::{self, Definition, DisplayStatus};
use super::port::{ApplyPort, ApplyRequest, ApplySharing};
use crate::error::Result;
use crate::playback::rotation::{make_handle, RotationHandle};
use crate::wallframe::scheduler::DisplayId;

struct SharedSession {
    playlist_id: i64,
    cursor: Arc<Mutex<PlaylistCursor>>,
    handle: RotationHandle,
    deadline: Arc<std::sync::Mutex<Option<Instant>>>,
    members: Arc<Mutex<Vec<DisplayId>>>,
    task: JoinHandle<()>,
}

impl Drop for SharedSession {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[derive(Default)]
pub(super) struct Sessions {
    by_playlist: Mutex<HashMap<i64, Arc<SharedSession>>>,
    by_display: Mutex<HashMap<DisplayId, i64>>,
}

impl Sessions {
    pub async fn owned_display_ids(&self) -> Vec<DisplayId> {
        self.by_display.lock().await.keys().copied().collect()
    }

    pub async fn is_owned(&self, display_id: DisplayId) -> bool {
        self.by_display.lock().await.contains_key(&display_id)
    }

    pub async fn activate(
        &self,
        targets: &[DisplayId],
        definition: Definition,
        resume_id: Option<String>,
        first_frame_timeout: Option<Duration>,
        apply: ApplyPort,
        shutdown: watch::Receiver<bool>,
    ) -> Result<()> {
        if targets.is_empty() {
            return Ok(());
        }
        self.release(targets).await;
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos() as u64)
            .unwrap_or(1)
            .max(1);
        let cursor = Arc::new(Mutex::new(PlaylistCursor::new(
            definition.items,
            definition.mode,
            seed,
        )));
        let (handle, receiver) = make_handle();
        handle.set_interval(definition.interval_secs);
        let deadline = Arc::new(std::sync::Mutex::new(None));
        let members = Arc::new(Mutex::new(targets.to_vec()));
        let first = {
            let mut cursor = cursor.lock().await;
            match resume_id {
                Some(id) if cursor.set_current(&id) => Some(id),
                _ => cursor.first(),
            }
        };
        if let Some(entry_id) = first {
            apply
                .apply(ApplyRequest {
                    entry_id,
                    display_ids: targets.to_vec(),
                    sharing: ApplySharing::Shared,
                    first_frame_timeout,
                })
                .await?;
        }
        let task = tokio::spawn(engine::run_playlist_rotator(
            cursor.clone(),
            deadline.clone(),
            members.clone(),
            ApplySharing::Shared,
            apply,
            receiver,
            shutdown,
        ));
        let session = Arc::new(SharedSession {
            playlist_id: definition.id,
            cursor,
            handle,
            deadline,
            members,
            task,
        });
        self.by_playlist.lock().await.insert(definition.id, session);
        let mut by_display = self.by_display.lock().await;
        for display_id in targets {
            by_display.insert(*display_id, definition.id);
        }
        Ok(())
    }

    pub async fn attach(
        &self,
        display_id: DisplayId,
        playlist_id: i64,
        first_frame_timeout: Duration,
        apply: ApplyPort,
    ) -> Result<bool> {
        let session = match self.attach_start(display_id, playlist_id).await {
            AttachStart::AlreadyOwned => return Ok(true),
            AttachStart::NoSession => return Ok(false),
            AttachStart::Session(session) => session,
        };
        if let Some(entry_id) = session.cursor.lock().await.current.clone() {
            apply
                .apply(ApplyRequest {
                    entry_id,
                    display_ids: vec![display_id],
                    sharing: ApplySharing::Shared,
                    first_frame_timeout: Some(first_frame_timeout),
                })
                .await?;
        }
        session.members.lock().await.push(display_id);
        self.by_display.lock().await.insert(display_id, playlist_id);
        Ok(true)
    }

    pub async fn release(&self, display_ids: &[DisplayId]) {
        let mut by_display = self.by_display.lock().await;
        let mut touched = Vec::new();
        for display_id in display_ids {
            if let Some(playlist_id) = by_display.remove(display_id) {
                touched.push((*display_id, playlist_id));
            }
        }
        drop(by_display);
        let mut by_playlist = self.by_playlist.lock().await;
        for (display_id, playlist_id) in touched {
            let Some(session) = by_playlist.get(&playlist_id).cloned() else {
                continue;
            };
            session.members.lock().await.retain(|id| *id != display_id);
            if session.members.lock().await.is_empty() {
                by_playlist.remove(&playlist_id);
            }
        }
    }

    pub async fn jump_to(
        &self,
        playlist_id: i64,
        entry_id: &str,
        apply: ApplyPort,
    ) -> Result<bool> {
        let session = self.by_playlist.lock().await.get(&playlist_id).cloned();
        let Some(session) = session else {
            return Ok(false);
        };
        if !session.cursor.lock().await.set_current(entry_id) {
            return Ok(true);
        }
        let display_ids = session.members.lock().await.clone();
        if !display_ids.is_empty() {
            apply
                .apply(ApplyRequest {
                    entry_id: entry_id.to_owned(),
                    display_ids,
                    sharing: ApplySharing::Shared,
                    first_frame_timeout: None,
                })
                .await?;
        }
        session.handle.kick();
        Ok(true)
    }

    pub async fn status(&self) -> Vec<DisplayStatus> {
        let sessions: Vec<_> = self.by_playlist.lock().await.values().cloned().collect();
        let now = Instant::now();
        let mut statuses = Vec::new();
        for session in sessions {
            let remaining_secs = session
                .deadline
                .lock()
                .unwrap()
                .map(|deadline| deadline.saturating_duration_since(now).as_secs() as u32)
                .unwrap_or(0);
            let interval_secs = session.handle.interval();
            let (mode, current_id, position, count) = {
                let cursor = session.cursor.lock().await;
                (
                    cursor.mode,
                    cursor.current.clone(),
                    cursor.pos as u32,
                    cursor.len() as u32,
                )
            };
            for display_id in session.members.lock().await.clone() {
                statuses.push(DisplayStatus {
                    display_id,
                    active_id: session.playlist_id,
                    mode,
                    interval_secs,
                    current_id: current_id.clone(),
                    position,
                    count,
                    remaining_secs,
                });
            }
        }
        statuses
    }

    pub async fn drop_display(&self, display_id: DisplayId) {
        self.release(&[display_id]).await;
    }

    pub async fn shutdown(&self) {
        self.by_display.lock().await.clear();
        self.by_playlist.lock().await.clear();
    }

    pub async fn deactivate_playlist(&self, playlist_id: i64) -> Vec<DisplayId> {
        let members = match self.by_playlist.lock().await.get(&playlist_id).cloned() {
            Some(session) => session.members.lock().await.clone(),
            None => return Vec::new(),
        };
        self.release(&members).await;
        members
    }

    pub async fn rebuild(
        &self,
        definition: Definition,
        apply: ApplyPort,
    ) -> Result<Option<Vec<DisplayId>>> {
        let session = self.by_playlist.lock().await.get(&definition.id).cloned();
        let Some(session) = session else {
            return Ok(None);
        };
        if definition.items.is_empty() {
            return Ok(Some(self.deactivate_playlist(definition.id).await));
        }
        let (entry_id, needs_apply) = {
            let mut cursor = session.cursor.lock().await;
            let current = cursor.current.clone();
            cursor.items = definition.items;
            cursor.mode = definition.mode;
            match current {
                Some(id) if cursor.items.iter().any(|item| item == &id) => {
                    cursor.set_current(&id);
                    (id, false)
                }
                _ => (cursor.first().unwrap_or_default(), true),
            }
        };
        session.handle.set_interval(definition.interval_secs);
        if needs_apply && !entry_id.is_empty() {
            let display_ids = session.members.lock().await.clone();
            if !display_ids.is_empty() {
                apply
                    .apply(ApplyRequest {
                        entry_id,
                        display_ids,
                        sharing: ApplySharing::Shared,
                        first_frame_timeout: None,
                    })
                    .await?;
            }
            session.handle.kick();
        }
        Ok(Some(Vec::new()))
    }

    pub async fn set_interval(&self, playlist_id: i64, interval_secs: u32) -> bool {
        let by_playlist = self.by_playlist.lock().await;
        let Some(session) = by_playlist.get(&playlist_id) else {
            return false;
        };
        session.handle.set_interval(interval_secs);
        true
    }

    async fn attach_start(&self, display_id: DisplayId, playlist_id: i64) -> AttachStart {
        if self.is_owned(display_id).await {
            return AttachStart::AlreadyOwned;
        }
        match self.by_playlist.lock().await.get(&playlist_id).cloned() {
            Some(session) => AttachStart::Session(session),
            None => AttachStart::NoSession,
        }
    }
}

enum AttachStart {
    AlreadyOwned,
    NoSession,
    Session(Arc<SharedSession>),
}

#[cfg(test)]
impl Sessions {
    pub async fn seed_for_test(
        &self,
        playlist_id: i64,
        members: &[DisplayId],
        items: Vec<String>,
        current: Option<String>,
        interval_secs: u32,
    ) {
        let mut cursor = PlaylistCursor::new(items, crate::playback::Mode::Sequential, 1);
        if let Some(id) = current {
            cursor.set_current(&id);
        } else {
            cursor.first();
        }
        let (handle, receiver) = make_handle();
        handle.set_interval(interval_secs);
        let task = tokio::spawn(async move {
            let _receiver = receiver;
            std::future::pending::<()>().await
        });
        let session = Arc::new(SharedSession {
            playlist_id,
            cursor: Arc::new(Mutex::new(cursor)),
            handle,
            deadline: Arc::new(std::sync::Mutex::new(None)),
            members: Arc::new(Mutex::new(members.to_vec())),
            task,
        });
        self.by_playlist.lock().await.insert(playlist_id, session);
        let mut by_display = self.by_display.lock().await;
        for display_id in members {
            by_display.insert(*display_id, playlist_id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn releasing_last_member_drops_session() {
        let sessions = Sessions::default();
        sessions
            .seed_for_test(1, &[10, 20], vec!["a".into()], Some("a".into()), 30)
            .await;
        sessions.release(&[10]).await;
        assert!(!sessions.is_owned(10).await);
        assert!(sessions.is_owned(20).await);
        sessions.release(&[20]).await;
        assert!(sessions.by_playlist.lock().await.is_empty());
    }

    #[tokio::test]
    async fn status_fans_out_one_cursor_to_members() {
        let sessions = Sessions::default();
        sessions
            .seed_for_test(
                8,
                &[100, 200],
                vec!["a".into(), "b".into()],
                Some("b".into()),
                45,
            )
            .await;
        let statuses = sessions.status().await;
        assert_eq!(statuses.len(), 2);
        assert!(statuses
            .iter()
            .all(|status| status.current_id.as_deref() == Some("b")));
    }
}
