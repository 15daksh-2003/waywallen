use super::*;

impl Router {
    pub async fn reusable_renderer_for_target(
        self: &Arc<Self>,
        request: &crate::wallframe::renderer_manager::SpawnRequest,
        target_ids: &[DisplayId],
        duplicate_renderer: bool,
    ) -> Option<RendererId> {
        let normalized_defaults = crate::catalog::properties::normalize_renderer_user_properties(
            request.default_user_properties.clone(),
        );
        let inner = self.inner.lock().await;
        let mut ids = inner
            .renderer_slots
            .iter()
            .filter_map(|(renderer_id, slot)| {
                let renderer_name_matches = request
                    .renderer_name
                    .as_deref()
                    .is_none_or(|name| name == slot.name);
                let identity_matches = slot.process_state == RendererProcessState::Running
                    && inner.table.get_renderer(renderer_id).is_some()
                    && slot.spawn_request.wp_type == request.wp_type
                    && renderer_name_matches
                    && slot.spawn_request.extras == request.extras
                    && slot.spawn_request.default_user_properties == normalized_defaults;
                let target_matches = !duplicate_renderer
                    || inner
                        .table
                        .links_for_renderer(renderer_id)
                        .iter()
                        .all(|link| target_ids.contains(&link.display_id));
                (identity_matches && target_matches).then(|| renderer_id.clone())
            })
            .collect::<Vec<_>>();
        ids.sort();
        ids.into_iter().next()
    }

    pub async fn renderer_ids_by_resource(self: &Arc<Self>, resource: &str) -> Vec<RendererId> {
        let inner = self.inner.lock().await;
        let mut ids = inner
            .renderer_slots
            .iter()
            .filter_map(|(id, slot)| {
                (slot.spawn_request.extras.get("path").map(String::as_str) == Some(resource))
                    .then(|| id.clone())
            })
            .collect::<Vec<_>>();
        ids.sort();
        ids
    }

    pub async fn update_renderer_assignment_property(
        self: &Arc<Self>,
        renderer_id: &str,
        key: &str,
        value: Option<&str>,
    ) -> bool {
        let mut inner = self.inner.lock().await;
        let Some(slot) = inner.renderer_slots.get_mut(renderer_id) else {
            return false;
        };
        let key = crate::catalog::properties::canonical_user_property_key(key).to_string();
        match value {
            Some(value) => {
                slot.spawn_request
                    .user_property_overrides
                    .insert(key, value.to_string());
            }
            None => {
                slot.spawn_request.user_property_overrides.remove(&key);
            }
        }
        slot.spec_revision = slot.spec_revision.wrapping_add(1).max(1);
        true
    }

    pub async fn update_renderer_assignment_settings(
        self: &Arc<Self>,
        renderer_name: &str,
        settings: &[(String, String)],
    ) -> Vec<RendererId> {
        let mut inner = self.inner.lock().await;
        let mut updated = Vec::new();
        for (renderer_id, slot) in &mut inner.renderer_slots {
            if slot.name != renderer_name {
                continue;
            }
            let mut changed = false;
            for (key, value) in settings {
                if slot.spawn_request.settings.get(key) != Some(value) {
                    slot.spawn_request
                        .settings
                        .insert(key.clone(), value.clone());
                    changed = true;
                }
            }
            if changed {
                slot.spec_revision = slot.spec_revision.wrapping_add(1).max(1);
                updated.push(renderer_id.clone());
            }
        }
        updated.sort();
        updated
    }

    pub async fn update_renderer_assignment_fps(
        self: &Arc<Self>,
        renderer_id: &str,
        fps: u32,
    ) -> bool {
        let mut inner = self.inner.lock().await;
        let Some(slot) = inner.renderer_slots.get_mut(renderer_id) else {
            return false;
        };
        let value = fps.to_string();
        if slot.spawn_request.settings.get("fps") != Some(&value) {
            slot.spawn_request.settings.insert("fps".to_string(), value);
            slot.spec_revision = slot.spec_revision.wrapping_add(1).max(1);
        }
        true
    }

    pub async fn update_renderer_assignment_layout(
        self: &Arc<Self>,
        renderer_id: &str,
        layout: WallpaperLayoutOverride,
    ) -> bool {
        let exists = self
            .inner
            .lock()
            .await
            .renderer_slots
            .contains_key(renderer_id);
        if !exists {
            return false;
        }
        self.set_renderer_wallpaper_layout_override(renderer_id, layout)
            .await
    }

    pub async fn displays_are_auto_stopped(self: &Arc<Self>, display_ids: &[DisplayId]) -> bool {
        if display_ids.is_empty() {
            return false;
        }
        let inner = self.inner.lock().await;
        display_ids.iter().all(|display_id| {
            inner
                .displays
                .get(display_id)
                .is_some_and(|display| display.auto_replay.stop_applied)
        })
    }

    pub async fn apply_retained_assignment(
        self: &Arc<Self>,
        spawn_request: crate::wallframe::renderer_manager::SpawnRequest,
        renderer_name: String,
        display_ids: &[DisplayId],
        duplicate_renderers: bool,
        wallpaper_layout_override: WallpaperLayoutOverride,
    ) -> RendererId {
        let mut removed = Vec::new();
        let mut to_drop = Vec::new();
        let mut applied = Vec::new();
        let mut applied_slots = Vec::new();
        let first_id = {
            let mut inner = self.inner.lock().await;
            let groups: Vec<Vec<DisplayId>> = if duplicate_renderers {
                display_ids.iter().map(|id| vec![*id]).collect()
            } else {
                vec![display_ids.to_vec()]
            };
            let mut first_id = None;
            for group in groups {
                let existing: HashSet<RendererId> = group
                    .iter()
                    .flat_map(|display_id| inner.table.links_for_display(*display_id))
                    .map(|link| link.renderer_id)
                    .collect();
                let reusable = (existing.len() == 1)
                    .then(|| existing.iter().next().cloned())
                    .flatten()
                    .filter(|renderer_id| {
                        inner
                            .table
                            .links_for_renderer(renderer_id)
                            .iter()
                            .all(|link| group.contains(&link.display_id))
                            && inner.renderer_slots.get(renderer_id).is_some_and(|slot| {
                                slot.retention == RendererRetention::Keep
                                    && !matches!(
                                        slot.process_state,
                                        RendererProcessState::Starting
                                            | RendererProcessState::Running
                                    )
                            })
                    });
                let renderer_id = reusable.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
                if let Some(slot) = inner.renderer_slots.get_mut(&renderer_id) {
                    slot.replace_spec(spawn_request.clone(), renderer_name.clone());
                    slot.retention = RendererRetention::Keep;
                } else {
                    inner.renderer_slots.insert(
                        renderer_id.clone(),
                        RendererSlot {
                            spawn_request: spawn_request.clone(),
                            name: renderer_name.clone(),
                            spec_revision: 1,
                            process_generation: 0,
                            process_state: RendererProcessState::Stopped,
                            activity_state: RendererStatus::Stopped,
                            retention: RendererRetention::Keep,
                            last_exit: None,
                            restart_failures: 0,
                        },
                    );
                }
                if wallpaper_layout_override.is_empty() {
                    inner.wallpaper_layout_overrides.remove(&renderer_id);
                } else {
                    inner
                        .wallpaper_layout_overrides
                        .insert(renderer_id.clone(), wallpaper_layout_override);
                }
                for display_id in &group {
                    for link in inner.table.links_for_display(*display_id) {
                        inner.table.remove_link(link.id);
                    }
                    inner
                        .table
                        .add_link_with_enabled(renderer_id.clone(), *display_id, false);
                    applied.push(*display_id);
                }
                first_id.get_or_insert_with(|| renderer_id.clone());
                applied_slots.push(renderer_id.clone());
                for old_id in existing {
                    if old_id == renderer_id || !inner.table.links_for_renderer(&old_id).is_empty()
                    {
                        continue;
                    }
                    let has_process = inner.table.get_renderer(&old_id).is_some();
                    if has_process {
                        if let Some(slot) = inner.renderer_slots.get_mut(&old_id) {
                            slot.retention = RendererRetention::Drop;
                        }
                        to_drop.push(old_id);
                    } else {
                        inner.renderer_slots.remove(&old_id);
                        inner.wallpaper_layout_overrides.remove(&old_id);
                        removed.push(old_id);
                    }
                }
            }
            first_id.expect("retained assignment requires at least one display")
        };
        for renderer_id in removed {
            self.emit(RouterEvent::RendererRemoved(renderer_id));
        }
        for display_id in applied {
            self.sync_display(display_id).await;
        }
        for renderer_id in to_drop {
            if let Err(error) = self.kill_renderer_drop(&renderer_id).await {
                log::warn!("renderer {renderer_id}: deferred replacement cleanup failed: {error}");
            }
        }
        applied_slots.sort();
        applied_slots.dedup();
        for renderer_id in applied_slots {
            if let Some(snapshot) = self.snapshot_renderer(&renderer_id).await {
                self.emit(RouterEvent::RendererUpsert(snapshot));
            }
        }
        self.emit(RouterEvent::DisplaysReplace(self.snapshot_displays().await));
        first_id
    }

    pub async fn apply_active_assignment(
        self: &Arc<Self>,
        spawn_request: crate::wallframe::renderer_manager::SpawnRequest,
        display_ids: &[DisplayId],
        exclusive_target: bool,
        wallpaper_layout_override: WallpaperLayoutOverride,
    ) -> crate::error::Result<RendererId> {
        let renderer_id = if let Some(renderer_id) = self
            .reusable_renderer_for_target(&spawn_request, display_ids, exclusive_target)
            .await
        {
            renderer_id
        } else {
            let to_stop = self.renderers_fully_replaced_by(Some(display_ids)).await;
            if !to_stop.is_empty() {
                self.stop_renderers_orderly(&to_stop, Duration::from_secs(1))
                    .await;
            }
            let renderer_id = self.mgr.spawn(spawn_request).await?;
            if let Some(handle) = self.mgr.get(&renderer_id).await {
                self.register_renderer(handle).await;
            }
            renderer_id
        };
        self.set_renderer_wallpaper_layout_override(&renderer_id, wallpaper_layout_override)
            .await;
        self.relink_displays_to(display_ids, &renderer_id).await;
        Ok(renderer_id)
    }

    // Routing policy

    /// Return the live renderers whose every display assignment is
    /// covered by `target`, meaning an imminent relink fully replaces them.
    pub async fn renderers_fully_replaced_by(
        self: &Arc<Self>,
        target: Option<&[DisplayId]>,
    ) -> Vec<RendererId> {
        let inner = self.inner.lock().await;
        inner
            .table
            .renderer_ids()
            .into_iter()
            .filter(|rid| {
                let links = inner.table.links_for_renderer(rid);
                if links.is_empty() {
                    return true;
                }
                match target {
                    None => true,
                    Some(ts) => links.iter().all(|l| ts.contains(&l.display_id)),
                }
            })
            .collect()
    }

    /// Stop and drop each logical renderer.
    pub async fn stop_renderers(self: &Arc<Self>, ids: &[RendererId]) {
        for id in ids {
            if let Err(error) = self.kill_renderer_drop(id).await {
                log::warn!("router: stop_renderers: kill {id}: {error}");
            }
        }
    }

    /// Stop the listed renderers after their displays release live bindings.
    pub async fn stop_renderers_orderly(
        self: &Arc<Self>,
        ids: &[RendererId],
        ack_timeout: Duration,
    ) {
        for id in ids {
            if let Err(error) = self.stop_renderer_drop(id, ack_timeout).await {
                log::warn!("router: stop_renderers_orderly: kill {id}: {error}");
            }
        }
    }

    /// Re-point each display assignment to `new_renderer_id`.
    pub async fn relink_displays_to(
        self: &Arc<Self>,
        display_ids: &[DisplayId],
        new_renderer_id: &str,
    ) {
        let retained_stops = self
            .renderers_inactivated_by_relink(display_ids, new_renderer_id)
            .await;
        for renderer_id in &retained_stops {
            self.begin_retained_stop(renderer_id).await;
        }
        let applied: Vec<DisplayId> = {
            let mut inner = self.inner.lock().await;
            let mut out = Vec::with_capacity(display_ids.len());
            for did in display_ids {
                if !inner.displays.contains_key(did) {
                    continue;
                }
                let existing = inner.table.links_for_display(*did);
                for link in existing {
                    inner.table.remove_link(link.id);
                }
                let enabled = !inner
                    .displays
                    .get(did)
                    .is_some_and(|display| display.auto_replay.stop_applied);
                inner
                    .table
                    .add_link_with_enabled(new_renderer_id.to_string(), *did, enabled);
                out.push(*did);
            }
            out
        };
        for did in &applied {
            self.sync_display(*did).await;
        }
        for renderer_id in retained_stops {
            self.finish_retained_stop(&renderer_id).await;
        }
        self.reconcile_lifecycle().await;
        // See `relink_all_displays_to` for the GC rationale. We always
        // run the mark pass so partially displaced renderers are handled.
        self.mark_orphans(Some(new_renderer_id)).await;
        self.reconcile_buffer_flags().await;
        if !applied.is_empty() {
            let all = self.snapshot_displays().await;
            self.emit(RouterEvent::DisplaysReplace(all));
        }
    }

    async fn renderers_inactivated_by_relink(
        self: &Arc<Self>,
        display_ids: &[DisplayId],
        new_renderer_id: &str,
    ) -> Vec<RendererId> {
        let inner = self.inner.lock().await;
        inner
            .table
            .renderer_ids()
            .into_iter()
            .filter(|renderer_id| renderer_id != new_renderer_id)
            .filter(|renderer_id| {
                let links = inner.table.links_for_renderer(renderer_id);
                links
                    .iter()
                    .any(|link| !display_ids.contains(&link.display_id))
                    && links
                        .iter()
                        .filter(|link| link.enabled)
                        .all(|link| display_ids.contains(&link.display_id))
            })
            .collect()
    }

    pub async fn relink_all_displays_to(self: &Arc<Self>, new_renderer_id: &str) {
        let display_ids: Vec<DisplayId> = {
            let mut inner = self.inner.lock().await;
            let ids: Vec<DisplayId> = inner.displays.keys().copied().collect();
            for did in &ids {
                let existing = inner.table.links_for_display(*did);
                for link in existing {
                    inner.table.remove_link(link.id);
                }
                let enabled = !inner
                    .displays
                    .get(did)
                    .is_some_and(|display| display.auto_replay.stop_applied);
                inner
                    .table
                    .add_link_with_enabled(new_renderer_id.to_string(), *did, enabled);
            }
            ids
        };
        let had_ids = !display_ids.is_empty();
        for did in display_ids {
            self.sync_display(did).await;
        }
        self.reconcile_lifecycle().await;
        // Active GC: any renderer that is no longer referenced by any
        // display gets a reap timer; the new renderer is kept.
        self.mark_orphans(Some(new_renderer_id)).await;
        self.reconcile_buffer_flags().await;
        if had_ids {
            let all = self.snapshot_displays().await;
            self.emit(RouterEvent::DisplaysReplace(all));
        }
    }

    /// Mutate a link's geometry/clear color and re-emit `SetCompositionConfig` to
    /// the affected display, without Bind or Unbind.
    pub async fn set_link_geometry(
        self: &Arc<Self>,
        link_id: LinkId,
        src: Option<LinkSrcRect>,
        dst: Option<LinkDstRect>,
        transform: Option<u32>,
        clear_rgba: Option<[f32; 4]>,
        z_order: Option<i32>,
    ) -> bool {
        let affected_display = {
            let mut inner = self.inner.lock().await;
            let changed = inner
                .table
                .update_link_geometry(link_id, src, dst, transform, clear_rgba, z_order);
            if !changed {
                return false;
            }
            let Some(link) = inner.table.get_link(link_id).cloned() else {
                return false;
            };
            inner
                .displays
                .contains_key(&link.display_id)
                .then_some(link.display_id)
        };
        if let Some(did) = affected_display {
            self.resync_display_composition(did).await;
            if let Some(snap) = self.snapshot_display(did).await {
                self.emit(RouterEvent::DisplayUpsert(snap));
            }
        } else {
            return false;
        }
        true
    }

    // ---------------------------------------------------------------
}
