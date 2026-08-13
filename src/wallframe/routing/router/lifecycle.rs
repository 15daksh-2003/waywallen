use super::*;

impl Router {
    pub async fn set_renderer_paused(self: &Arc<Self>, renderer_id: &str, paused: bool) -> bool {
        let changed = {
            let mut inner = self.inner.lock().await;
            if !inner.renderer_slots.contains_key(renderer_id) {
                return false;
            }
            if paused {
                inner.renderer_manual_paused.insert(renderer_id.to_string())
            } else {
                inner.renderer_manual_paused.remove(renderer_id)
            }
        };
        if changed {
            self.reconcile_lifecycle().await;
        }
        true
    }

    pub async fn kill_renderer_drop(
        self: &Arc<Self>,
        renderer_id: &str,
    ) -> crate::error::Result<()> {
        self.stop_renderer_drop(renderer_id, Duration::from_secs(1))
            .await
    }

    pub(super) async fn stop_renderer_drop(
        self: &Arc<Self>,
        renderer_id: &str,
        ack_timeout: Duration,
    ) -> crate::error::Result<()> {
        let process_generation = {
            let mut inner = self.inner.lock().await;
            let Some(slot) = inner.renderer_slots.get_mut(renderer_id) else {
                return Err(crate::error::Error::RendererNotFound(
                    renderer_id.to_string(),
                ));
            };
            let transition = slot.transition(RendererLifecycleEvent::StopRequested { keep: false });
            let process_generation = match (&slot.state, transition) {
                (RendererLifecycleState::Stopping { generation, .. }, _) => Some(*generation),
                (_, RendererTransition::Remove) => None,
                _ => None,
            };
            for link in inner.table.links_for_renderer(renderer_id) {
                inner.table.set_link_enabled(link.id, false);
                if let Some(display) = inner.displays.get(&link.display_id) {
                    display.invalidate_consumption();
                }
            }
            process_generation
        };
        let Some(process_generation) = process_generation else {
            self.unregister_renderer(renderer_id).await;
            return Ok(());
        };
        self.begin_unbind_ack_tracking(renderer_id).await;
        let displays = {
            let inner = self.inner.lock().await;
            inner
                .table
                .links_for_renderer(renderer_id)
                .into_iter()
                .map(|link| link.display_id)
                .collect::<Vec<_>>()
        };
        for display_id in displays {
            self.sync_display(display_id).await;
        }
        if let Some(snapshot) = self.snapshot_renderer(renderer_id).await {
            self.emit(RouterEvent::RendererUpsert(snapshot));
        }
        if self
            .await_unbind_acks_for(renderer_id, ack_timeout)
            .await
            .is_err()
        {
            log::warn!("renderer {renderer_id}: kill unbind acknowledgement timed out");
        }
        if let Some(exit) = self
            .mgr
            .stop_generation(renderer_id, process_generation)
            .await?
        {
            self.on_renderer_process_exit(exit).await;
        }
        Ok(())
    }

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
            }
        }
        self.reconcile_assignment_activation().await;
    }

    async fn reconcile_assignment_activation(self: &Arc<Self>) {
        let (mut changed_displays, mut reenabled_renderers, mut stopped_renderers) = {
            let mut inner = self.inner.lock().await;
            let display_ids = inner.displays.keys().copied().collect::<Vec<_>>();
            let mut changed_displays = Vec::new();
            let mut reenabled_renderers = Vec::new();
            for display_id in display_ids {
                let enabled = !inner.manual_stopped
                    && !inner
                        .displays
                        .get(&display_id)
                        .is_some_and(|display| display.auto_replay.stop_applied);
                let mut changed = false;
                for link in inner.table.links_for_display(display_id) {
                    if inner.table.set_link_enabled(link.id, enabled) {
                        changed = true;
                        if enabled {
                            reenabled_renderers.push(link.renderer_id);
                        }
                    }
                }
                if changed {
                    if let Some(display) = inner.displays.get(&display_id) {
                        display.invalidate_consumption();
                    }
                    changed_displays.push(display_id);
                }
            }
            let stopped_renderers = inner
                .renderer_slots
                .keys()
                .filter(|renderer_id| {
                    let links = inner.table.links_for_renderer(renderer_id);
                    inner.manual_stopped
                        || (!links.is_empty() && links.iter().all(|link| !link.enabled))
                })
                .cloned()
                .collect::<Vec<_>>();
            (changed_displays, reenabled_renderers, stopped_renderers)
        };
        changed_displays.sort_unstable();
        changed_displays.dedup();
        reenabled_renderers.sort();
        reenabled_renderers.dedup();
        stopped_renderers.sort();
        stopped_renderers.dedup();
        for renderer_id in &stopped_renderers {
            self.begin_retained_stop(renderer_id).await;
        }
        for display_id in &changed_displays {
            self.sync_display(*display_id).await;
        }
        futures_util::future::join_all(
            stopped_renderers
                .iter()
                .map(|renderer_id| self.finish_retained_stop(renderer_id)),
        )
        .await;
        futures_util::future::join_all(reenabled_renderers.iter().map(|renderer_id| async move {
            self.start_retained_renderer(renderer_id, false).await;
            self.schedule_process_restart(renderer_id).await;
        }))
        .await;
        if !changed_displays.is_empty() {
            self.reconcile_buffer_flags().await;
            let all = self.snapshot_displays().await;
            self.emit(RouterEvent::DisplaysReplace(all));
        }
    }

    pub async fn set_manual_stop(self: &Arc<Self>, stopped: bool) {
        let changed = {
            let mut inner = self.inner.lock().await;
            if inner.manual_stopped == stopped {
                false
            } else {
                inner.manual_stopped = stopped;
                true
            }
        };
        if changed {
            self.reconcile_assignment_activation().await;
            self.reconcile_lifecycle().await;
        }
    }

    pub(super) async fn begin_retained_stop(self: &Arc<Self>, renderer_id: &str) {
        let generation = {
            let mut inner = self.inner.lock().await;
            inner.renderer_slots.get_mut(renderer_id).and_then(|slot| {
                (slot.transition(RendererLifecycleEvent::StopRequested { keep: true })
                    == RendererTransition::Changed)
                    .then(|| slot.state.generation())
                    .flatten()
            })
        };
        if generation.is_none() {
            return;
        }
        self.begin_unbind_ack_tracking(renderer_id).await;
        if let Some(snapshot) = self.snapshot_renderer(renderer_id).await {
            self.emit(RouterEvent::RendererUpsert(snapshot));
        }
    }

    pub(super) async fn finish_retained_stop(self: &Arc<Self>, renderer_id: &str) {
        let generation = {
            let inner = self.inner.lock().await;
            inner
                .renderer_slots
                .get(renderer_id)
                .and_then(|slot| match slot.state {
                    RendererLifecycleState::Stopping { generation, .. } => Some(generation),
                    _ => None,
                })
        };
        let Some(generation) = generation else { return };
        if self
            .await_unbind_acks_for(renderer_id, Duration::from_secs(1))
            .await
            .is_err()
        {
            log::warn!("renderer {renderer_id}: retained stop unbind acknowledgement timed out");
        }
        match self.mgr.stop_generation(renderer_id, generation).await {
            Ok(Some(exit)) => {
                self.on_renderer_process_exit(exit).await;
            }
            Ok(None) => {
                log::debug!("renderer {renderer_id}: skip stop for stale generation={generation}");
            }
            Err(crate::error::Error::RendererNotFound(_)) => {
                log::debug!(
                    "renderer {renderer_id}: generation={generation} is not registered while stopping"
                );
            }
            Err(error) => {
                log::warn!("renderer {renderer_id}: retained stop failed: {error}");
            }
        }
    }

    pub async fn start_retained_renderer(
        self: &Arc<Self>,
        renderer_id: &str,
        reactivate_failed: bool,
    ) {
        loop {
            let process_generation = self.mgr.reserve_process_generation();
            let (spec_revision, spawn_request) = {
                let mut inner = self.inner.lock().await;
                let has_demand = inner
                    .table
                    .links_for_renderer(renderer_id)
                    .iter()
                    .any(|link| link.enabled && inner.displays.contains_key(&link.display_id));
                if !has_demand || inner.table.get_renderer(renderer_id).is_some() {
                    return;
                }
                let Some(slot) = inner.renderer_slots.get_mut(renderer_id) else {
                    return;
                };
                if slot.transition(RendererLifecycleEvent::StartRequested {
                    generation: process_generation,
                    reactivate_failed,
                }) != RendererTransition::Changed
                {
                    return;
                }
                (slot.spec_revision, slot.spawn_request.clone())
            };
            if let Some(snapshot) = self.snapshot_renderer(renderer_id).await {
                self.emit(RouterEvent::RendererUpsert(snapshot));
            }
            match self
                .mgr
                .spawn_for_generation(renderer_id.to_string(), process_generation, spawn_request)
                .await
            {
                Ok(()) => {
                    let Some(handle) = self.mgr.get(renderer_id).await else {
                        return;
                    };
                    debug_assert_eq!(handle.process_generation, process_generation);
                    if !self
                        .register_renderer_current(
                            handle,
                            Some((spec_revision, process_generation)),
                        )
                        .await
                    {
                        let tracked = {
                            let mut inner = self.inner.lock().await;
                            let no_live_handle = inner.table.get_renderer(renderer_id).is_none();
                            inner
                                .renderer_slots
                                .get_mut(renderer_id)
                                .filter(|_| no_live_handle)
                                .is_some_and(|slot| {
                                    if matches!(
                                        slot.state,
                                        RendererLifecycleState::Starting { generation }
                                            if generation == process_generation
                                    ) {
                                        let _ = slot.transition(
                                            RendererLifecycleEvent::StopRequested { keep: true },
                                        );
                                    }
                                    matches!(
                                        slot.state,
                                        RendererLifecycleState::Stopping { generation, .. }
                                            if generation == process_generation
                                    )
                                })
                        };
                        if tracked {
                            if let Some(snapshot) = self.snapshot_renderer(renderer_id).await {
                                self.emit(RouterEvent::RendererUpsert(snapshot));
                            }
                        }
                        match self
                            .mgr
                            .stop_generation(renderer_id, process_generation)
                            .await
                        {
                            Ok(Some(exit)) if tracked => {
                                self.settle_renderer_process_exit(exit).await;
                                continue;
                            }
                            Ok(Some(exit)) => log::debug!(
                                "renderer {renderer_id}: discarded stale spawned generation {}",
                                exit.process_generation
                            ),
                            Ok(None) | Err(crate::error::Error::RendererNotFound(_)) => {}
                            Err(error) => log::warn!(
                                "renderer {renderer_id}: stale generation cleanup failed: {error}"
                            ),
                        }
                        return;
                    }
                    let displays = {
                        let inner = self.inner.lock().await;
                        inner
                            .table
                            .links_for_renderer(renderer_id)
                            .into_iter()
                            .filter(|link| link.enabled)
                            .map(|link| link.display_id)
                            .collect::<Vec<_>>()
                    };
                    for display_id in displays {
                        self.sync_display(display_id).await;
                    }
                    self.reconcile_buffer_flags().await;
                    if let Some(snapshot) = self.snapshot_renderer(renderer_id).await {
                        self.emit(RouterEvent::RendererUpsert(snapshot));
                    }
                    return;
                }
                Err(error) => {
                    log::warn!("renderer {renderer_id}: retained start failed: {error}");
                    let remove = {
                        let mut inner = self.inner.lock().await;
                        if let Some(slot) = inner.renderer_slots.get_mut(renderer_id) {
                            let transition = slot.transition(RendererLifecycleEvent::SpawnFailed {
                                generation: process_generation,
                                failure: RendererExitSnapshot {
                                    code: None,
                                    signal: None,
                                    reason: error.to_string(),
                                },
                            });
                            if transition == RendererTransition::Remove {
                                log::debug!(
                                    "renderer {renderer_id}: remove failed dropped generation={process_generation}"
                                );
                            }
                            transition == RendererTransition::Remove
                        } else {
                            false
                        }
                    };
                    if remove {
                        self.unregister_renderer(renderer_id).await;
                        return;
                    }
                    if let Some(snapshot) = self.snapshot_renderer(renderer_id).await {
                        self.emit(RouterEvent::RendererUpsert(snapshot));
                    }
                    return;
                }
            }
        }
    }

    pub(super) async fn schedule_process_restart(self: &Arc<Self>, renderer_id: &str) {
        let (spec_revision, failures, initial_delay) = {
            let mut inner = self.inner.lock().await;
            if inner.process_restart_tasks.contains_key(renderer_id) {
                return;
            }
            let has_demand = inner
                .table
                .links_for_renderer(renderer_id)
                .iter()
                .any(|link| link.enabled && inner.displays.contains_key(&link.display_id));
            let Some(slot) = inner.renderer_slots.get_mut(renderer_id) else {
                return;
            };
            if !has_demand
                || !matches!(
                    slot.state,
                    RendererLifecycleState::Killed { keep: true, .. }
                )
                || slot.restart_failures >= PROCESS_RESTART_MAX_FAILURES
            {
                return;
            }
            slot.restart_failures += 1;
            (
                slot.spec_revision,
                slot.restart_failures,
                resume_retry_delay(slot.restart_failures),
            )
        };
        log::warn!("renderer {renderer_id}: restart attempt {failures}/{PROCESS_RESTART_MAX_FAILURES} in {initial_delay:?}");
        let weak = Arc::downgrade(self);
        let id = renderer_id.to_string();
        let task_id = id.clone();
        let task = tokio::spawn(async move {
            tokio::time::sleep(initial_delay).await;
            let Some(router) = weak.upgrade() else { return };
            let should_start = {
                let inner = router.inner.lock().await;
                let has_demand = inner
                    .table
                    .links_for_renderer(&task_id)
                    .iter()
                    .any(|link| link.enabled && inner.displays.contains_key(&link.display_id));
                inner.renderer_slots.get(&task_id).is_some_and(|slot| {
                    has_demand
                        && slot.spec_revision == spec_revision
                        && matches!(
                            slot.state,
                            RendererLifecycleState::Killed { keep: true, .. }
                        )
                })
            };
            if should_start {
                router.start_retained_renderer(&task_id, false).await;
            }
            router
                .inner
                .lock()
                .await
                .process_restart_tasks
                .remove(&task_id);
        });
        let mut inner = self.inner.lock().await;
        if let Some(previous) = inner.process_restart_tasks.insert(id, task) {
            previous.abort();
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
            stopped: inner.manual_stopped,
        }
    }

    /// Whether this renderer's effective commanded activity is paused.
    /// Returns `false` for unknown ids.
    pub async fn is_paused(self: &Arc<Self>, renderer_id: &str) -> bool {
        self.inner
            .lock()
            .await
            .renderer_slots
            .get(renderer_id)
            .is_some_and(|slot| slot.state.activity() == Some(RendererActivity::Paused))
    }

    pub async fn is_muted(self: &Arc<Self>, renderer_id: &str) -> bool {
        self.inner
            .lock()
            .await
            .renderer_slots
            .get(renderer_id)
            .is_some_and(|slot| slot.state.activity() == Some(RendererActivity::Muted))
    }
}
