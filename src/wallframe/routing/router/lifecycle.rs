use super::*;

impl Router {
    /// Update the session-level state driven by the
    /// `session_monitor` task. `None` leaves that flag unchanged.
    pub async fn update_session_state(
        self: &Arc<Self>,
        locked: Option<bool>,
        inactive: Option<bool>,
    ) {
        let display_ids = {
            let mut inner = self.inner.lock().await;
            let mut changed = false;
            if let Some(v) = locked {
                if inner.session_locked != v {
                    inner.session_locked = v;
                    changed = true;
                }
            }
            if let Some(v) = inactive {
                if inner.session_inactive != v {
                    inner.session_inactive = v;
                    changed = true;
                }
            }
            if !changed {
                Vec::new()
            } else {
                inner.displays.keys().copied().collect()
            }
        };
        for display_id in display_ids {
            let action = self.update_auto_state(display_id, None).await;
            self.run_auto_state_action(action).await;
        }
    }

    pub(super) async fn update_auto_state(
        self: &Arc<Self>,
        display_id: DisplayId,
        flags: Option<u32>,
    ) -> AutoStateAction {
        let mut inner = self.inner.lock().await;
        let session_locked = inner.session_locked;
        let session_inactive = inner.session_inactive;
        let Some(state) = inner.displays.get_mut(&display_id) else {
            return AutoStateAction::Noop;
        };
        let next_flags = flags.unwrap_or(state.auto_replay.last_flags);
        let policy = self.resolved_auto_replay(&state.info);
        let new_raw = auto_replay::decide(
            &policy,
            auto_replay::Facts {
                flags: next_flags,
                session_locked,
                session_inactive,
            },
        );
        let same_input = flags.is_some_and(|v| v == state.auto_replay.last_flags);
        if flags.is_some() {
            state.auto_replay.last_flags = next_flags;
        }
        if same_input && new_raw == state.auto_replay.raw {
            return AutoStateAction::Noop;
        }
        state.auto_replay.raw = new_raw;
        if new_raw.is_active() {
            if state.auto_replay.requested != new_raw {
                state.auto_replay.requested = new_raw;
                AutoStateAction::Reconcile
            } else {
                AutoStateAction::Noop
            }
        } else if state.auto_replay.requested.is_active() {
            state.auto_replay.requested = new_raw;
            AutoStateAction::Reconcile
        } else {
            state.auto_replay.requested = new_raw;
            AutoStateAction::Noop
        }
    }

    pub(super) async fn run_auto_state_action(self: &Arc<Self>, action: AutoStateAction) {
        match action {
            AutoStateAction::Noop => {}
            AutoStateAction::Reconcile => {
                self.apply_auto_stop_links().await;
                self.reconcile_lifecycle().await;
            }
        }
    }

    pub(super) async fn apply_auto_stop_links(self: &Arc<Self>) {
        let mut changed_displays = Vec::new();
        let mut stop_events = Vec::new();
        let mut reenabled_renderers = Vec::new();
        let mut disabled_any = false;
        {
            let mut inner = self.inner.lock().await;
            let plans: Vec<(DisplayId, bool)> = inner
                .displays
                .iter()
                .filter_map(|(display_id, state)| {
                    let should_stop = state.auto_replay.requested.action == AutoAction::Stop;
                    (state.auto_replay.stop_applied != should_stop)
                        .then_some((*display_id, should_stop))
                })
                .collect();
            for (display_id, should_stop) in plans {
                if let Some(state) = inner.displays.get_mut(&display_id) {
                    state.auto_replay.stop_applied = should_stop;
                }
                let mut link_changed = false;
                for link in inner.table.links_for_display(display_id) {
                    if inner.table.set_link_enabled(link.id, !should_stop) {
                        link_changed = true;
                        if should_stop {
                            disabled_any = true;
                        } else {
                            reenabled_renderers.push(link.renderer_id);
                        }
                    }
                }
                if link_changed {
                    if let Some(state) = inner.displays.get(&display_id) {
                        state.invalidate_consumption();
                    }
                }
                changed_displays.push(display_id);
                stop_events.push(AutoStopEvent {
                    display_id,
                    stopped: should_stop,
                });
            }
        }
        for renderer_id in reenabled_renderers {
            self.cancel_orphan_timer(&renderer_id).await;
        }
        for display_id in &changed_displays {
            self.sync_display(*display_id).await;
        }
        if disabled_any {
            self.mark_orphans(None).await;
        }
        if !changed_displays.is_empty() {
            self.reconcile_buffer_flags().await;
            let all = self.snapshot_displays().await;
            self.emit(RouterEvent::DisplaysReplace(all));
        }
        for evt in stop_events {
            if let Err(e) = self.auto_stop_tx.send(evt) {
                log::debug!("router: no auto-stop subscribers ({e})");
            }
        }
    }

    pub async fn set_manual_pause(self: &Arc<Self>, paused: bool) {
        let changed = {
            let mut inner = self.inner.lock().await;
            if inner.manual_paused == paused {
                false
            } else {
                inner.manual_paused = paused;
                true
            }
        };
        if changed {
            self.reconcile_lifecycle().await;
        }
    }

    pub async fn toggle_manual_pause(self: &Arc<Self>) -> bool {
        let paused = {
            let mut inner = self.inner.lock().await;
            inner.manual_paused = !inner.manual_paused;
            inner.manual_paused
        };
        self.reconcile_lifecycle().await;
        paused
    }

    pub async fn set_manual_mute(self: &Arc<Self>, muted: bool) {
        let changed = {
            let mut inner = self.inner.lock().await;
            if inner.manual_muted == muted {
                false
            } else {
                inner.manual_muted = muted;
                true
            }
        };
        if changed {
            self.reconcile_lifecycle().await;
        }
    }

    pub async fn toggle_manual_mute(self: &Arc<Self>) -> bool {
        let muted = {
            let mut inner = self.inner.lock().await;
            inner.manual_muted = !inner.manual_muted;
            inner.manual_muted
        };
        self.reconcile_lifecycle().await;
        muted
    }

    pub async fn set_other_playback_active(self: &Arc<Self>, active: bool) {
        let changed = {
            let mut inner = self.inner.lock().await;
            if inner.other_playback_active == active {
                false
            } else {
                inner.other_playback_active = active;
                true
            }
        };
        if changed {
            self.reconcile_lifecycle().await;
        }
    }

    pub async fn manual_lifecycle_state(self: &Arc<Self>) -> ManualLifecycleState {
        let inner = self.inner.lock().await;
        ManualLifecycleState {
            paused: inner.manual_paused,
            muted: inner.manual_muted,
        }
    }

    /// Whether this renderer is currently in the paused set (zero
    /// enabled links). Returns `false` for unknown ids.
    pub async fn is_paused(self: &Arc<Self>, renderer_id: &str) -> bool {
        self.inner
            .lock()
            .await
            .renderer_states
            .get(renderer_id)
            .is_some_and(|status| *status == PausedRendererStatus::Paused)
    }

    pub async fn is_muted(self: &Arc<Self>, renderer_id: &str) -> bool {
        self.inner
            .lock()
            .await
            .renderer_states
            .get(renderer_id)
            .is_some_and(|status| *status == PausedRendererStatus::Muted)
    }
}
