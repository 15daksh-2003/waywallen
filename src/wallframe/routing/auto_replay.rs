use crate::settings::{AutoAction, AutoCondition, AutoReplayPolicy};

/// Some mapped (non-minimized) window covers this display.
pub const FLAG_NON_MINIMIZED: u32 = 1 << 0;
/// Some window on this display has keyboard focus.
pub const FLAG_ACTIVE: u32 = 1 << 1;
/// Some window is H+V maximized (and NOT fullscreen).
pub const FLAG_MAXIMIZED: u32 = 1 << 2;
/// Some window is fullscreen.
pub const FLAG_FULLSCREEN: u32 = 1 << 3;

/// Bits the daemon understands. Higher bits are reserved and ignored.
pub const FLAGS_KNOWN: u32 = FLAG_NON_MINIMIZED | FLAG_ACTIVE | FLAG_MAXIMIZED | FLAG_FULLSCREEN;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Facts {
    pub flags: u32,
    pub session_locked: bool,
    pub session_inactive: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Decision {
    pub action: AutoAction,
}

impl Default for Decision {
    fn default() -> Self {
        Self {
            action: AutoAction::None,
        }
    }
}

impl Decision {
    pub fn from_action(action: AutoAction) -> Self {
        Self { action }
    }

    pub fn is_active(self) -> bool {
        self.action != AutoAction::None
    }
}

/// Per-display auto replay state held by the router.
#[derive(Debug, Default)]
pub struct State {
    pub last_flags: u32,
    pub raw: Decision,
    pub requested: Decision,
    pub stop_applied: bool,
}

impl State {
    pub fn new() -> Self {
        Self::default()
    }
}

pub fn decide(policy: &AutoReplayPolicy, facts: Facts) -> Decision {
    let mut best = Decision::from_action(AutoAction::None);
    for condition in AUTO_CONDITIONS {
        if !condition_matches(condition, facts) {
            continue;
        }
        let action = policy.action_for(condition);
        if action.priority() > best.action.priority() {
            best = Decision::from_action(action);
        }
    }
    best
}

const AUTO_CONDITIONS: [AutoCondition; 6] = [
    AutoCondition::AnyWindow,
    AutoCondition::Focused,
    AutoCondition::Maximized,
    AutoCondition::Fullscreen,
    AutoCondition::SessionLocked,
    AutoCondition::SessionInactive,
];

fn condition_matches(condition: AutoCondition, facts: Facts) -> bool {
    let has = |b: u32| facts.flags & b != 0;
    match condition {
        AutoCondition::AnyWindow => has(FLAG_NON_MINIMIZED),
        AutoCondition::Focused => has(FLAG_ACTIVE),
        AutoCondition::Maximized => has(FLAG_MAXIMIZED),
        AutoCondition::Fullscreen => has(FLAG_FULLSCREEN),
        AutoCondition::SessionLocked => facts.session_locked,
        AutoCondition::SessionInactive => facts.session_inactive,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy(actions: &[(AutoCondition, AutoAction)]) -> AutoReplayPolicy {
        let mut policy = AutoReplayPolicy::default();
        for (condition, action) in actions {
            policy.set_action(*condition, *action);
        }
        policy
    }

    #[test]
    fn decide_selects_matching_action() {
        let policy = policy(&[(AutoCondition::Fullscreen, AutoAction::Pause)]);
        let decision = decide(
            &policy,
            Facts {
                flags: FLAG_FULLSCREEN,
                session_locked: false,
                session_inactive: false,
            },
        );
        assert_eq!(decision.action, AutoAction::Pause);
    }

    #[test]
    fn action_priority_selects_stronger_action() {
        let policy = policy(&[
            (AutoCondition::Focused, AutoAction::Mute),
            (AutoCondition::Fullscreen, AutoAction::Pause),
        ]);
        let decision = decide(
            &policy,
            Facts {
                flags: FLAG_ACTIVE | FLAG_FULLSCREEN,
                session_locked: false,
                session_inactive: false,
            },
        );
        assert_eq!(decision.action, AutoAction::Pause);
    }

    #[test]
    fn stop_wins_over_pause() {
        let policy = policy(&[
            (AutoCondition::Focused, AutoAction::Pause),
            (AutoCondition::Fullscreen, AutoAction::Stop),
        ]);
        let decision = decide(
            &policy,
            Facts {
                flags: FLAG_ACTIVE | FLAG_FULLSCREEN,
                session_locked: false,
                session_inactive: false,
            },
        );
        assert_eq!(decision.action, AutoAction::Stop);
    }

    #[test]
    fn none_does_not_override_stronger_actions() {
        let policy = policy(&[
            (AutoCondition::Focused, AutoAction::Pause),
            (AutoCondition::Fullscreen, AutoAction::None),
        ]);
        let decision = decide(
            &policy,
            Facts {
                flags: FLAG_ACTIVE | FLAG_FULLSCREEN,
                session_locked: false,
                session_inactive: false,
            },
        );
        assert_eq!(decision.action, AutoAction::Pause);
    }
}
