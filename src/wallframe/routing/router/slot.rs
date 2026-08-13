use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RendererRetention {
    Keep,
    Drop,
}

impl RendererRetention {
    pub fn keep(self) -> bool {
        self == Self::Keep
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RendererProcessState {
    Starting,
    Running,
    Stopping,
    Stopped,
    Killed,
    Failed,
}

impl RendererProcessState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Starting => "starting",
            Self::Running => "running",
            Self::Stopping => "stopping",
            Self::Stopped => "stopped",
            Self::Killed => "killed",
            Self::Failed => "failed",
        }
    }

    pub fn has_process(self) -> bool {
        matches!(self, Self::Starting | Self::Running | Self::Stopping)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RendererExitSnapshot {
    pub code: Option<i32>,
    pub signal: Option<i32>,
    pub reason: String,
}

pub(super) struct RendererSlot {
    pub spawn_request: crate::wallframe::renderer_manager::SpawnRequest,
    pub name: String,
    pub spec_revision: u64,
    pub process_generation: u64,
    pub process_state: RendererProcessState,
    pub activity_state: RendererStatus,
    pub retention: RendererRetention,
    pub last_exit: Option<RendererExitSnapshot>,
    pub restart_failures: u32,
}

impl RendererSlot {
    pub fn running(handle: &RendererHandle) -> Self {
        Self {
            spawn_request: handle.spawn_request(),
            name: handle.name.clone(),
            spec_revision: 1,
            process_generation: handle.process_generation,
            process_state: RendererProcessState::Running,
            activity_state: RendererStatus::Playing,
            retention: RendererRetention::Keep,
            last_exit: None,
            restart_failures: 0,
        }
    }

    pub fn replace_spec(
        &mut self,
        spawn_request: crate::wallframe::renderer_manager::SpawnRequest,
        name: String,
    ) {
        self.spawn_request = spawn_request;
        self.name = name;
        self.spec_revision = self.spec_revision.wrapping_add(1).max(1);
        self.last_exit = None;
        self.restart_failures = 0;
    }

    pub fn attach_process(&mut self, handle: &RendererHandle) {
        self.spawn_request = handle.spawn_request();
        self.name = handle.name.clone();
        self.process_generation = handle.process_generation;
        self.process_state = RendererProcessState::Running;
        self.last_exit = None;
    }

    pub fn begin_start(&mut self, process_generation: u64) -> (u64, u64) {
        self.process_generation = process_generation;
        self.process_state = RendererProcessState::Starting;
        self.retention = RendererRetention::Keep;
        (self.spec_revision, self.process_generation)
    }

    pub fn begin_stop(&mut self, retention: RendererRetention) -> Option<u64> {
        if !self.process_state.has_process() {
            return None;
        }
        self.process_state = RendererProcessState::Stopping;
        self.activity_state = RendererStatus::Stopped;
        self.retention = retention;
        Some(self.process_generation)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slot() -> RendererSlot {
        RendererSlot {
            spawn_request: Default::default(),
            name: "image".into(),
            spec_revision: 1,
            process_generation: 7,
            process_state: RendererProcessState::Running,
            activity_state: RendererStatus::Playing,
            retention: RendererRetention::Keep,
            last_exit: None,
            restart_failures: 0,
        }
    }

    #[test]
    fn stop_keeps_process_generation() {
        let mut slot = slot();
        assert_eq!(slot.begin_stop(RendererRetention::Keep), Some(7));
        assert_eq!(slot.process_state, RendererProcessState::Stopping);
        assert!(slot.retention.keep());
    }

    #[test]
    fn replacing_spec_does_not_change_process_generation() {
        let mut slot = slot();
        slot.process_state = RendererProcessState::Stopped;
        slot.replace_spec(Default::default(), "video".into());
        assert_eq!(slot.spec_revision, 2);
        assert_eq!(slot.process_generation, 7);
        assert_eq!(slot.name, "video");
    }
}
