use super::*;

type WriterCompletion = std::sync::mpsc::SyncSender<std::result::Result<(), String>>;

enum WriterCommand {
    Reliable {
        msg: ControlMsg,
        commit: Option<RendererSubscription>,
        done: WriterCompletion,
    },
    WakeAudio,
}

#[derive(Clone)]
pub(super) struct RendererWriter {
    tx: std::sync::mpsc::SyncSender<WriterCommand>,
    audio: Arc<StdMutex<Option<(u64, ControlMsg)>>>,
    audio_wake_pending: Arc<AtomicBool>,
}

impl RendererWriter {
    pub(super) fn spawn(
        id: RendererId,
        stream: StdUnixStream,
        subscriptions: Arc<RendererSubscriptionRegistry>,
        reap_tx: tokio::sync::mpsc::UnboundedSender<RendererId>,
    ) -> Self {
        let (tx, rx) = std::sync::mpsc::sync_channel(WRITER_QUEUE_CAPACITY);
        let audio = Arc::new(StdMutex::new(None));
        let writer_audio = Arc::clone(&audio);
        let audio_wake_pending = Arc::new(AtomicBool::new(false));
        let writer_audio_wake_pending = Arc::clone(&audio_wake_pending);
        thread::spawn(move || {
            'writer: loop {
                let first = match rx.recv() {
                    Ok(command) => command,
                    Err(_) => break,
                };
                let mut commands = std::collections::VecDeque::from([first]);
                while commands.len() < WRITER_QUEUE_CAPACITY {
                    match rx.try_recv() {
                        Ok(command) => commands.push_back(command),
                        Err(_) => break,
                    }
                }

                while let Some(command) = commands.pop_front() {
                    let WriterCommand::Reliable { msg, commit, done } = command else {
                        continue;
                    };
                    match send_control(&stream, &msg, &[]) {
                        Ok(()) => {
                            if let Some(subscription) = commit {
                                if let Ok(mut audio) = writer_audio.lock() {
                                    *audio = None;
                                }
                                subscriptions.commit(id.clone(), subscription);
                            }
                            let _ = done.send(Ok(()));
                        }
                        Err(error) => {
                            let reason = error.to_string();
                            let _ = done.send(Err(reason.clone()));
                            for pending in commands.drain(..).chain(rx.try_iter()) {
                                if let WriterCommand::Reliable { done, .. } = pending {
                                    let _ = done.send(Err(reason.clone()));
                                }
                            }
                            log::warn!("renderer {id}: writer exit: {reason}");
                            let _ = reap_tx.send(id.clone());
                            break 'writer;
                        }
                    }
                }

                writer_audio_wake_pending.store(false, Ordering::Release);
                let latest = writer_audio.lock().ok().and_then(|mut audio| audio.take());
                if let Some((revision, msg)) = latest {
                    if subscriptions
                        .snapshot()
                        .revision_for(&id, RendererEventKind::Audio)
                        == Some(revision)
                    {
                        if let Err(error) = send_control(&stream, &msg, &[]) {
                            log::warn!("renderer {id}: audio writer exit: {error}");
                            let _ = reap_tx.send(id.clone());
                            break;
                        }
                    }
                }
            }

            for command in rx.try_iter() {
                if let WriterCommand::Reliable { done, .. } = command {
                    let _ = done.send(Err("renderer writer stopped".to_string()));
                }
            }
        });
        Self {
            tx,
            audio,
            audio_wake_pending,
        }
    }

    pub(super) fn send_blocking(
        &self,
        msg: ControlMsg,
        commit: Option<RendererSubscription>,
    ) -> std::result::Result<(), String> {
        let (done_tx, done_rx) = std::sync::mpsc::sync_channel(1);
        self.tx
            .send(WriterCommand::Reliable {
                msg,
                commit,
                done: done_tx,
            })
            .map_err(|_| "renderer writer stopped".to_string())?;
        done_rx
            .recv()
            .map_err(|_| "renderer writer stopped".to_string())?
    }

    pub(super) fn replace_audio(
        &self,
        revision: u64,
        msg: ControlMsg,
    ) -> std::result::Result<(), String> {
        *self
            .audio
            .lock()
            .map_err(|_| "audio slot mutex poisoned".to_string())? = Some((revision, msg));
        if self.audio_wake_pending.swap(true, Ordering::AcqRel) {
            return Ok(());
        }
        match self.tx.try_send(WriterCommand::WakeAudio) {
            Ok(()) | Err(std::sync::mpsc::TrySendError::Full(_)) => Ok(()),
            Err(std::sync::mpsc::TrySendError::Disconnected(_)) => {
                self.audio_wake_pending.store(false, Ordering::Release);
                Err("renderer writer stopped".to_string())
            }
        }
    }
}
