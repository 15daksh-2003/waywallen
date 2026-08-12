use super::*;

pub struct SettingsStore {
    inner: Arc<StdRwLock<Settings>>,
    notify: Arc<Notify>,
    path: PathBuf,
    /// Serializes concurrent `flush()` calls.
    /// Covers both the debounced writer and shutdown flush.
    flush_lock: tokio::sync::Mutex<()>,
    /// Set when the in-memory state diverges from disk.
    /// Cleared by a successful `flush()`.
    dirty: AtomicBool,
    writer_task: StdMutex<Option<tokio::task::JoinHandle<()>>>,
}

impl SettingsStore {
    #[cfg(test)]
    pub(super) fn from_test_settings(settings: Settings) -> Arc<Self> {
        Arc::new(Self {
            inner: Arc::new(StdRwLock::new(settings)),
            notify: Arc::new(Notify::new()),
            path: PathBuf::from("/dev/null"),
            flush_lock: tokio::sync::Mutex::new(()),
            dirty: AtomicBool::new(false),
            writer_task: StdMutex::new(None),
        })
    }

    /// Load from `path`, or fall back to defaults and seed the file.
    /// Seeding makes the config visible to users immediately.
    pub async fn load_or_default(path: PathBuf) -> Arc<Self> {
        let mut seed_on_disk = false;
        let initial = match tokio::fs::read_to_string(&path).await {
            Ok(s) => match toml::from_str::<Settings>(&s) {
                Ok(parsed) => {
                    log::info!("settings loaded from {}", path.display());
                    parsed
                }
                Err(e) => {
                    log::warn!(
                        "settings parse {}: {e}; continuing with defaults",
                        path.display()
                    );
                    Settings::default()
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                log::info!(
                    "settings file {} not found, seeding defaults",
                    path.display()
                );
                seed_on_disk = true;
                let mut settings = Settings::default();
                settings.global.auto_replay = Some(AutoReplayPolicy::default());
                settings
            }
            Err(e) => {
                log::warn!(
                    "settings file {} not readable ({e}); using defaults",
                    path.display()
                );
                Settings::default()
            }
        };

        let store = Arc::new(Self {
            inner: Arc::new(StdRwLock::new(initial)),
            notify: Arc::new(Notify::new()),
            path,
            flush_lock: tokio::sync::Mutex::new(()),
            // Mark dirty when no on-disk file exists so the seed flush
            // writes the default config.
            dirty: AtomicBool::new(seed_on_disk),
            writer_task: StdMutex::new(None),
        });

        if seed_on_disk {
            store.flush().await;
        }

        // Debounced writer task.
        let writer = Arc::clone(&store);
        let task = tokio::spawn(async move {
            writer.writer_loop().await;
        });
        *store.writer_task.lock().expect("settings writer poisoned") = Some(task);

        store
    }

    pub async fn stop_writer(&self) {
        let task = self
            .writer_task
            .lock()
            .expect("settings writer poisoned")
            .take();
        if let Some(task) = task {
            task.abort();
            let _ = task.await;
        }
    }

    /// Snapshot the current settings by cloning under a read lock.
    /// Callers needing only globals should use narrower helpers.
    pub fn snapshot(&self) -> Settings {
        self.inner.read().expect("settings poisoned").clone()
    }

    /// Copy the `GlobalSettings` subset.
    pub fn global(&self) -> GlobalSettings {
        self.inner.read().expect("settings poisoned").global.clone()
    }

    pub fn pointer_forwarding_enabled(&self) -> bool {
        self.inner
            .read()
            .expect("settings poisoned")
            .global
            .pointer_forwarding_enabled
    }

    /// Clone the value map for a single plugin, or `None` if the
    /// plugin has no recorded settings.
    pub fn plugin(&self, plugin_name: &str) -> Option<HashMap<String, String>> {
        self.inner
            .read()
            .expect("settings poisoned")
            .plugins
            .get(plugin_name)
            .cloned()
    }

    pub fn resolved_renderer_settings(
        &self,
        renderer: &crate::plugin::renderer_registry::RendererDef,
    ) -> HashMap<String, String> {
        self.inner
            .read()
            .expect("settings poisoned")
            .resolved_renderer_settings(renderer)
    }

    pub fn apply_global_renderer_settings(
        &self,
        renderer: &crate::plugin::renderer_registry::RendererDef,
        values: &mut HashMap<String, String>,
    ) {
        self.inner
            .read()
            .expect("settings poisoned")
            .apply_global_renderer_settings(renderer, values);
    }

    /// Resolve the effective layout for a display name.
    /// Per-display overrides win field by field.
    pub fn resolved_layout(&self, display_name: &str) -> ResolvedLayout {
        let g = self.inner.read().expect("settings poisoned");
        let defaults = &g.global.layout;
        let prefs = g.displays.get(display_name);
        let default_location = defaults
            .location
            .unwrap_or_else(|| Location::from_align(defaults.align));
        ResolvedLayout {
            fillmode: prefs.and_then(|p| p.fillmode).unwrap_or(defaults.fillmode),
            location: prefs
                .and_then(|p| p.location)
                .or_else(|| prefs.and_then(|p| p.align.map(Location::from_align)))
                .unwrap_or(default_location),
            rotation: prefs.and_then(|p| p.rotation).unwrap_or(defaults.rotation),
        }
    }

    pub fn resolved_auto_replay(&self, display_name: &str) -> AutoReplayPolicy {
        let g = self.inner.read().expect("settings poisoned");
        if let Some(policy) = g
            .displays
            .get(display_name)
            .and_then(|prefs| prefs.auto_replay)
        {
            return policy;
        }
        if let Some(policy) = &g.global.auto_replay {
            return *policy;
        }
        AutoReplayPolicy::default()
    }

    /// Per-display wallpaper id with fallback to global `last_wallpaper`.
    /// Used by hot-plug recall and startup restore.
    pub fn resolved_last_wallpaper(&self, display_key: &str) -> Option<String> {
        let g = self.inner.read().expect("settings poisoned");
        if let Some(prefs) = g.displays.get(display_key) {
            if let Some(id) = &prefs.last_wallpaper {
                return Some(id.clone());
            }
        }
        g.global.last_wallpaper.clone()
    }

    /// Snapshot just the cloned per-display preferences.
    /// Used to expose overrides over the control plane.
    pub fn display_prefs(&self, display_name: &str) -> Option<DisplayPrefs> {
        self.inner
            .read()
            .expect("settings poisoned")
            .displays
            .get(display_name)
            .cloned()
    }

    /// Snapshot every registered display name in the prefs map.
    pub fn display_pref_names(&self) -> Vec<String> {
        self.inner
            .read()
            .expect("settings poisoned")
            .displays
            .keys()
            .cloned()
            .collect()
    }

    /// Apply an in-memory mutation and compare before/after state.
    /// Only changed settings mark the store dirty.
    pub fn update<F>(&self, f: F)
    where
        F: FnOnce(&mut Settings),
    {
        let changed = {
            let mut g = self.inner.write().expect("settings poisoned");
            let before = g.clone();
            f(&mut g);
            *g != before
        };
        if changed {
            self.dirty.store(true, Ordering::SeqCst);
            self.notify.notify_one();
        }
    }

    async fn writer_loop(self: Arc<Self>) {
        loop {
            // Block until something needs to be written.
            self.notify.notified().await;
            // Debounce: keep resetting the timer until DEBOUNCE_WRITE
            // elapses without another update.
            loop {
                tokio::select! {
                    _ = tokio::time::sleep(DEBOUNCE_WRITE) => break,
                    _ = self.notify.notified() => {}
                }
            }
            self.flush().await;
        }
    }

    /// Force a synchronous flush of current settings to disk.
    /// Bypasses the debounce window for shutdown.
    pub async fn flush_now(&self) {
        self.flush().await;
    }

    async fn flush(&self) {
        // Cheap fast path before grabbing the lock: if nothing has
        // changed since the last successful flush, skip entirely.
        if !self.dirty.load(Ordering::SeqCst) {
            return;
        }
        let _g = self.flush_lock.lock().await;
        // Re-check under the lock — another flush may have just
        // raced us to the same state.
        if !self.dirty.swap(false, Ordering::SeqCst) {
            return;
        }

        let snapshot = self.snapshot();
        let serialized = match toml::to_string_pretty(&snapshot) {
            Ok(s) => s,
            Err(e) => {
                log::warn!("settings serialize failed: {e}");
                self.dirty.store(true, Ordering::SeqCst);
                return;
            }
        };

        if let Some(parent) = self.path.parent() {
            if let Err(e) = tokio::fs::create_dir_all(parent).await {
                log::warn!("settings create_dir_all {}: {e}", parent.display());
                self.dirty.store(true, Ordering::SeqCst);
                return;
            }
        }

        let tmp = {
            let mut p = self.path.clone();
            let new_name = match p.file_name() {
                Some(n) => {
                    let mut s = n.to_os_string();
                    s.push(".tmp");
                    s
                }
                None => {
                    self.dirty.store(true, Ordering::SeqCst);
                    return;
                }
            };
            p.set_file_name(new_name);
            p
        };
        if let Err(e) = tokio::fs::write(&tmp, serialized).await {
            log::warn!("settings write {}: {e}", tmp.display());
            self.dirty.store(true, Ordering::SeqCst);
            return;
        }
        if let Err(e) = tokio::fs::rename(&tmp, &self.path).await {
            log::warn!(
                "settings rename {} → {}: {e}",
                tmp.display(),
                self.path.display()
            );
            self.dirty.store(true, Ordering::SeqCst);
            return;
        }
        log::debug!("settings flushed to {}", self.path.display());
    }

    /// Read-only view of the on-disk path.
    /// Useful before the rest of `DaemonContext` is constructed.
    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    /// Bring in-memory plugin tables in line with loaded renderer
    /// manifest schemas.
    pub fn reconcile(&self, registry: &crate::plugin::renderer_registry::RendererRegistry) -> bool {
        use crate::plugin::renderer_registry::{
            check_setting_bounds, setting_default_value, SettingDef,
        };

        let mut changed = false;
        let mut g = self.inner.write().expect("settings poisoned");

        // Pre-compute manifest schemas keyed by plugin name so user
        // tables can be checked for unknown plugins.
        let manifests: HashMap<String, &HashMap<String, SettingDef>> = registry
            .all_renderers()
            .into_iter()
            .map(|d| (d.name.clone(), &d.settings))
            .collect();

        // 1) Reconcile each known plugin's table.
        for (plugin_name, schema) in &manifests {
            if schema.is_empty() {
                continue;
            }
            let entry = g.plugins.entry(plugin_name.clone()).or_default();

            // Drop keys that aren't in the manifest anymore.
            let stale: Vec<String> = entry
                .keys()
                .filter(|k| !schema.contains_key(*k))
                .cloned()
                .collect();
            for k in stale {
                log::warn!(
                    "settings: dropping unknown key '{plugin_name}.{k}' \
                     (no longer in manifest schema)"
                );
                entry.remove(&k);
                changed = true;
            }

            // Fill in / reset bad values for declared keys.
            for (key, def) in schema.iter() {
                let needs_default = match entry.get(key) {
                    None => true,
                    Some(v) => match check_setting_bounds(key, v, def) {
                        Ok(()) => false,
                        Err(e) => {
                            log::warn!(
                                "settings: '{plugin_name}.{key}' = {v:?} \
                                 violates schema ({e}); resetting to default"
                            );
                            true
                        }
                    },
                };
                if needs_default {
                    let default = setting_default_value(def);
                    if entry.get(key) != Some(&default) {
                        entry.insert(key.clone(), default);
                        changed = true;
                    }
                }
            }
        }

        // Keep tables without a renderer manifest. They may belong to a Lua
        // source, or to a temporarily unavailable renderer.
        for plugin_name in g.plugins.keys() {
            if !manifests.contains_key(plugin_name) {
                log::warn!(
                    "settings: component '{plugin_name}' has persisted values \
                     but no matching renderer manifest is loaded; leaving as-is"
                );
            }
        }

        if changed {
            self.dirty.store(true, Ordering::SeqCst);
            self.notify.notify_one();
        }
        changed
    }
}
