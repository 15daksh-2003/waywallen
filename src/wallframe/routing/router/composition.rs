use super::*;

impl Router {
    // Routing policy

    /// Return the renderers whose every enabled display link is
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
                let enabled: Vec<_> = links.iter().filter(|l| l.enabled).collect();
                if enabled.is_empty() {
                    // Already orphaned (no enabled links). Counts as
                    // fully replaced so the caller can clean it up too.
                    return true;
                }
                match target {
                    None => true, // relink_all replaces every display
                    Some(ts) => enabled.iter().all(|l| ts.contains(&l.display_id)),
                }
            })
            .collect()
    }

    /// Synchronously unregister + kill each `id` in `ids`. Used by
    /// the apply path to drop fully replaced renderers.
    pub async fn stop_renderers(self: &Arc<Self>, ids: &[RendererId]) {
        for id in ids {
            self.unregister_renderer(id).await;
            if let Err(e) = self.mgr.kill(id).await {
                log::warn!("router: stop_renderers: kill {id}: {e}");
            }
        }
    }

    /// Stop the listed renderers with the wallpaper-switch shutdown
    /// handshake: track unbind acks, unregister, wait, then kill.
    pub async fn stop_renderers_orderly(
        self: &Arc<Self>,
        ids: &[RendererId],
        ack_timeout: Duration,
    ) {
        for id in ids {
            self.begin_unbind_ack_tracking(id).await;
        }
        for id in ids {
            self.unregister_renderer(id).await;
        }
        for id in ids {
            if self.await_unbind_acks_for(id, ack_timeout).await.is_err() {
                log::warn!(
                    "router: stop_renderers_orderly: ack_unbind timeout \
                     for renderer {id}; proceeding with kill anyway"
                );
            }
        }
        for id in ids {
            if let Err(e) = self.mgr.kill(id).await {
                log::warn!("router: stop_renderers_orderly: kill {id}: {e}");
            }
        }
    }

    /// Re-point every enabled link to `new_renderer_id`. Used by
    /// `WallpaperApply` in single-wallpaper mode.
    pub async fn relink_displays_to(
        self: &Arc<Self>,
        display_ids: &[DisplayId],
        new_renderer_id: &str,
    ) {
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
                inner.table.add_link(new_renderer_id.to_string(), *did);
                out.push(*did);
            }
            out
        };
        for did in &applied {
            self.sync_display(*did).await;
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

    pub async fn relink_all_displays_to(self: &Arc<Self>, new_renderer_id: &str) {
        let display_ids: Vec<DisplayId> = {
            let mut inner = self.inner.lock().await;
            let ids: Vec<DisplayId> = inner.displays.keys().copied().collect();
            for did in &ids {
                let existing = inner.table.links_for_display(*did);
                for link in existing {
                    inner.table.remove_link(link.id);
                }
                inner.table.add_link(new_renderer_id.to_string(), *did);
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
