use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RendererActivity {
    Playing,
    Paused,
    Muted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RendererStartCause {
    ExplicitApply { preempt_pending: bool },
    ExplicitSpawn,
    AutoReplayResume,
    ManualStopResume,
    DisplayReconnect,
    ProcessRestart,
}

impl RendererStartCause {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ExplicitApply {
                preempt_pending: true,
            } => "explicit-immediate",
            Self::ExplicitApply {
                preempt_pending: false,
            } => "explicit-coalescing",
            Self::ExplicitSpawn => "explicit-spawn",
            Self::AutoReplayResume => "auto-replay",
            Self::ManualStopResume => "manual-stop-resume",
            Self::DisplayReconnect => "display-reconnect",
            Self::ProcessRestart => "process-restart",
        }
    }

    pub fn allows_failed(self) -> bool {
        matches!(
            self,
            Self::ExplicitApply {
                preempt_pending: true
            } | Self::ExplicitSpawn
                | Self::DisplayReconnect
        )
    }

    pub fn preempts_pending(self) -> bool {
        matches!(
            self,
            Self::ExplicitApply {
                preempt_pending: true
            } | Self::ExplicitSpawn
                | Self::ManualStopResume
                | Self::DisplayReconnect
        )
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) struct PendingRendererStart {
    pub cause: RendererStartCause,
    pub not_before: tokio::time::Instant,
    pub token: u64,
}

impl RendererActivity {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Playing => "playing",
            Self::Paused => "paused",
            Self::Muted => "muted",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RendererExitSnapshot {
    pub code: Option<i32>,
    pub signal: Option<i32>,
    pub reason: String,
}

impl From<&crate::wallframe::renderer_manager::RendererProcessExit> for RendererExitSnapshot {
    fn from(exit: &crate::wallframe::renderer_manager::RendererProcessExit) -> Self {
        Self {
            code: exit.code,
            signal: exit.signal,
            reason: exit.reason.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RendererLifecycleState {
    Starting {
        generation: u64,
    },
    Running {
        generation: u64,
        activity: RendererActivity,
    },
    Stopping {
        generation: u64,
        keep: bool,
    },
    Stopped {
        keep: bool,
        last_exit: Option<RendererExitSnapshot>,
    },
    Killed {
        keep: bool,
        last_exit: RendererExitSnapshot,
    },
    Failed {
        failure: RendererExitSnapshot,
    },
}

impl RendererLifecycleState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Starting { .. } => "starting",
            Self::Running { activity, .. } => activity.as_str(),
            Self::Stopping { .. } => "stopping",
            Self::Stopped { .. } => "stopped",
            Self::Killed { .. } => "killed",
            Self::Failed { .. } => "failed",
        }
    }

    pub fn generation(&self) -> Option<u64> {
        match self {
            Self::Starting { generation }
            | Self::Running { generation, .. }
            | Self::Stopping { generation, .. } => Some(*generation),
            Self::Stopped { .. } | Self::Killed { .. } | Self::Failed { .. } => None,
        }
    }

    pub fn activity(&self) -> Option<RendererActivity> {
        match self {
            Self::Running { activity, .. } => Some(*activity),
            _ => None,
        }
    }

    pub fn has_process(&self) -> bool {
        matches!(
            self,
            Self::Starting { .. } | Self::Running { .. } | Self::Stopping { .. }
        )
    }

    pub fn is_running(&self) -> bool {
        matches!(self, Self::Running { .. })
    }

    pub fn is_failed(&self) -> bool {
        matches!(self, Self::Failed { .. })
    }

    pub fn last_exit(&self) -> Option<&RendererExitSnapshot> {
        match self {
            Self::Stopped { last_exit, .. } => last_exit.as_ref(),
            Self::Killed { last_exit, .. } => Some(last_exit),
            Self::Failed { failure } => Some(failure),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub(super) enum RendererLifecycleEvent {
    StartRequested {
        generation: u64,
        start_token: u64,
        reactivate_failed: bool,
    },
    ProcessAttached {
        generation: u64,
    },
    ActivityResolved(RendererActivity),
    StopRequested {
        keep: bool,
    },
    ProcessExited {
        generation: u64,
        kind: crate::wallframe::renderer_manager::RendererProcessExitKind,
        exit: RendererExitSnapshot,
    },
    SpawnFailed {
        generation: u64,
        failure: RendererExitSnapshot,
    },
    SpecReplaced {
        reactivate_failed: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RendererTransition {
    Changed,
    Unchanged,
    Remove,
}

pub(super) struct RendererSlot {
    pub spawn_request: crate::wallframe::renderer_manager::SpawnRequest,
    pub name: String,
    pub spec_revision: u64,
    pub state: RendererLifecycleState,
    pub restart_failures: u32,
    pub pending_start: Option<PendingRendererStart>,
    pub active_start_token: Option<u64>,
}

impl RendererSlot {
    pub fn running(handle: &RendererHandle) -> Self {
        Self {
            spawn_request: handle.spawn_request(),
            name: handle.name.clone(),
            spec_revision: 1,
            state: RendererLifecycleState::Running {
                generation: handle.process_generation,
                activity: RendererActivity::Playing,
            },
            restart_failures: 0,
            pending_start: None,
            active_start_token: None,
        }
    }

    pub fn retained(
        spawn_request: crate::wallframe::renderer_manager::SpawnRequest,
        name: String,
    ) -> Self {
        Self {
            spawn_request,
            name,
            spec_revision: 1,
            state: RendererLifecycleState::Stopped {
                keep: true,
                last_exit: None,
            },
            restart_failures: 0,
            pending_start: None,
            active_start_token: None,
        }
    }

    pub fn replace_spec(
        &mut self,
        spawn_request: crate::wallframe::renderer_manager::SpawnRequest,
        name: String,
        reactivate_failed: bool,
    ) {
        self.spawn_request = spawn_request;
        self.name = name;
        self.spec_revision = self.spec_revision.wrapping_add(1).max(1);
        self.restart_failures = 0;
        let _ = self.transition(RendererLifecycleEvent::SpecReplaced { reactivate_failed });
    }

    pub fn transition(&mut self, event: RendererLifecycleEvent) -> RendererTransition {
        use crate::wallframe::renderer_manager::RendererProcessExitKind;
        use RendererLifecycleState as State;

        match event {
            RendererLifecycleEvent::StartRequested {
                generation,
                start_token,
                reactivate_failed,
            } => match &self.state {
                State::Stopped { keep: true, .. } | State::Killed { keep: true, .. } => {
                    self.state = State::Starting { generation };
                    self.active_start_token = Some(start_token);
                    RendererTransition::Changed
                }
                State::Failed { .. } if reactivate_failed => {
                    self.state = State::Starting { generation };
                    self.active_start_token = Some(start_token);
                    RendererTransition::Changed
                }
                _ => RendererTransition::Unchanged,
            },
            RendererLifecycleEvent::ProcessAttached { generation } => match self.state {
                State::Starting {
                    generation: current,
                } if current == generation => {
                    self.state = State::Running {
                        generation,
                        activity: RendererActivity::Playing,
                    };
                    self.active_start_token = None;
                    RendererTransition::Changed
                }
                _ => RendererTransition::Unchanged,
            },
            RendererLifecycleEvent::ActivityResolved(activity) => match &mut self.state {
                State::Running {
                    activity: current, ..
                } if *current != activity => {
                    *current = activity;
                    RendererTransition::Changed
                }
                _ => RendererTransition::Unchanged,
            },
            RendererLifecycleEvent::StopRequested { keep } => match &mut self.state {
                State::Starting { generation } | State::Running { generation, .. } => {
                    self.state = State::Stopping {
                        generation: *generation,
                        keep,
                    };
                    RendererTransition::Changed
                }
                State::Stopping { keep: current, .. } if *current && !keep => {
                    *current = false;
                    RendererTransition::Changed
                }
                State::Stopped { keep: current, .. } | State::Killed { keep: current, .. }
                    if *current && !keep =>
                {
                    *current = false;
                    RendererTransition::Remove
                }
                State::Failed { .. } if !keep => RendererTransition::Remove,
                _ => RendererTransition::Unchanged,
            },
            RendererLifecycleEvent::ProcessExited {
                generation,
                kind,
                exit,
            } => {
                let keep = match &self.state {
                    State::Starting {
                        generation: current,
                    }
                    | State::Running {
                        generation: current,
                        ..
                    } if *current == generation => true,
                    State::Stopping {
                        generation: current,
                        keep,
                    } if *current == generation => *keep,
                    _ => return RendererTransition::Unchanged,
                };
                self.state = match kind {
                    RendererProcessExitKind::Stopped => State::Stopped {
                        keep,
                        last_exit: Some(exit),
                    },
                    RendererProcessExitKind::Killed => State::Killed {
                        keep,
                        last_exit: exit,
                    },
                    RendererProcessExitKind::Failed if keep => State::Failed { failure: exit },
                    RendererProcessExitKind::Failed => return RendererTransition::Remove,
                };
                self.active_start_token = None;
                if keep {
                    RendererTransition::Changed
                } else {
                    RendererTransition::Remove
                }
            }
            RendererLifecycleEvent::SpawnFailed {
                generation,
                failure,
            } => {
                let keep = match self.state {
                    State::Starting {
                        generation: current,
                    } if current == generation => true,
                    State::Stopping {
                        generation: current,
                        keep,
                    } if current == generation => keep,
                    _ => return RendererTransition::Unchanged,
                };
                if keep {
                    self.state = State::Failed { failure };
                    self.active_start_token = None;
                    RendererTransition::Changed
                } else {
                    RendererTransition::Remove
                }
            }
            RendererLifecycleEvent::SpecReplaced { reactivate_failed } => match self.state {
                State::Stopped { keep: true, .. } | State::Killed { keep: true, .. } => {
                    self.state = State::Stopped {
                        keep: true,
                        last_exit: None,
                    };
                    RendererTransition::Changed
                }
                State::Failed { .. } if reactivate_failed => {
                    self.state = State::Stopped {
                        keep: true,
                        last_exit: None,
                    };
                    RendererTransition::Changed
                }
                _ => RendererTransition::Unchanged,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exit(reason: &str) -> RendererExitSnapshot {
        RendererExitSnapshot {
            code: None,
            signal: None,
            reason: reason.into(),
        }
    }

    fn slot(state: RendererLifecycleState) -> RendererSlot {
        RendererSlot {
            spawn_request: Default::default(),
            name: "image".into(),
            spec_revision: 1,
            state,
            restart_failures: 0,
            pending_start: None,
            active_start_token: None,
        }
    }

    #[test]
    fn keep_is_carried_through_stop_completion() {
        let mut slot = slot(RendererLifecycleState::Running {
            generation: 7,
            activity: RendererActivity::Playing,
        });
        assert_eq!(
            slot.transition(RendererLifecycleEvent::StopRequested { keep: true }),
            RendererTransition::Changed
        );
        assert_eq!(
            slot.transition(RendererLifecycleEvent::ProcessExited {
                generation: 7,
                kind: crate::wallframe::renderer_manager::RendererProcessExitKind::Stopped,
                exit: exit("stopped"),
            }),
            RendererTransition::Changed
        );
        assert!(matches!(
            slot.state,
            RendererLifecycleState::Stopped { keep: true, .. }
        ));
    }

    #[test]
    fn drop_is_irreversible_while_stopping() {
        let mut slot = slot(RendererLifecycleState::Stopping {
            generation: 7,
            keep: true,
        });
        assert_eq!(
            slot.transition(RendererLifecycleEvent::StopRequested { keep: false }),
            RendererTransition::Changed
        );
        assert_eq!(
            slot.transition(RendererLifecycleEvent::StopRequested { keep: true }),
            RendererTransition::Unchanged
        );
        assert!(matches!(
            slot.state,
            RendererLifecycleState::Stopping { keep: false, .. }
        ));
    }

    #[test]
    fn failed_requires_explicit_reactivation() {
        let mut slot = slot(RendererLifecycleState::Failed {
            failure: exit("failed"),
        });
        assert_eq!(
            slot.transition(RendererLifecycleEvent::StartRequested {
                generation: 8,
                start_token: 1,
                reactivate_failed: false,
            }),
            RendererTransition::Unchanged
        );
        assert_eq!(
            slot.transition(RendererLifecycleEvent::StartRequested {
                generation: 8,
                start_token: 2,
                reactivate_failed: true,
            }),
            RendererTransition::Changed
        );
        assert!(matches!(
            slot.state,
            RendererLifecycleState::Starting { generation: 8 }
        ));
    }

    #[test]
    fn replacing_failed_spec_makes_it_startable() {
        let mut slot = slot(RendererLifecycleState::Failed {
            failure: exit("failed"),
        });
        assert_eq!(
            slot.transition(RendererLifecycleEvent::SpecReplaced {
                reactivate_failed: true,
            }),
            RendererTransition::Changed
        );
        assert!(matches!(
            slot.state,
            RendererLifecycleState::Stopped {
                keep: true,
                last_exit: None
            }
        ));
    }

    #[test]
    fn stale_exit_does_not_replace_current_generation() {
        let mut slot = slot(RendererLifecycleState::Running {
            generation: 8,
            activity: RendererActivity::Playing,
        });
        assert_eq!(
            slot.transition(RendererLifecycleEvent::ProcessExited {
                generation: 7,
                kind: crate::wallframe::renderer_manager::RendererProcessExitKind::Failed,
                exit: exit("stale"),
            }),
            RendererTransition::Unchanged
        );
        assert!(matches!(
            slot.state,
            RendererLifecycleState::Running { generation: 8, .. }
        ));
    }
}
