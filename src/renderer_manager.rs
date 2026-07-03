use crate::error::{Error, Result, ResultExt};
use std::collections::HashMap;
use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream as StdUnixStream;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Arc, Mutex as StdMutex, OnceLock, Weak as StdWeak};
use std::thread;
use std::time::Duration;
use tokio::process::{Child, Command};
use tokio::sync::{broadcast, Mutex as TokioMutex};
use uuid::Uuid;

use crate::ipc::proto::{ControlMsg, EventMsg};
use crate::ipc::uds::{recv_event, send_control, CodecError};

/// Renderer IPC compatibility version the daemon currently emits. Bump
/// this when the daemon/renderer wire contract changes.
pub const SPAWN_VERSION: u32 = 6;
use crate::plugin::renderer_registry::{
    RendererDef, RendererRegistry, EVENT_KIND_MPRIS, EVENT_KIND_POINTER,
};
use crate::routing::Router;
use crate::wallpaper::types::WallpaperType;

// ---------------------------------------------------------------------------
// Public types

pub type RendererId = String;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MprisSnapshot {
    pub state: u32,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub album_artist: String,
    pub art_url: String,
    pub previous_art_url: String,
}

impl Default for MprisSnapshot {
    fn default() -> Self {
        Self {
            state: 0,
            title: String::new(),
            artist: String::new(),
            album: String::new(),
            album_artist: String::new(),
            art_url: String::new(),
            previous_art_url: String::new(),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct SpawnRequest {
    /// The wallpaper type determines which renderer binary is spawned.
    pub wp_type: WallpaperType,
    /// CLI argv dictionary the daemon turns into `--<key> <value>`
    /// pairs after `--ipc <socket>`.
    pub extras: HashMap<String, String>,
    /// Plugin settings kv that flows directly into `Init.settings`.
    /// Callers usually source this from the reconciled settings store.
    pub settings: HashMap<String, String>,
    /// When true, pass `--test-pattern` to the renderer host, which
    /// lets test renderers bypass normal content loading.
    pub test_pattern: bool,
    /// Optional explicit renderer plugin name. `None` (default) lets
    /// `spawn` and `find_reusable` pick by type priority.
    pub renderer_name: Option<String>,
    /// Renderer-owned subset of the DB row's `user_property_overrides`
    /// column; daemon-owned layout keys are filtered out before spawn.
    pub user_properties_json: Option<String>,
}

/// Snapshot of the most recent `BindBuffers` event, plus the DMA-BUF FDs
/// attached to it. Owned here and copied out to display endpoints.
pub struct BindSnapshot {
    /// Monotonically increasing per-renderer pool generation. Sourced
    /// from the renderer's `bind_buffers.generation` field.
    pub generation: u64,
    /// Placement flag set the renderer used when allocating this pool.
    /// Bit 0 = host_visible (GTT). See `BUF_HOST_VISIBLE`.
    pub flags: u32,
    pub count: u32,
    pub fourcc: u32,
    pub width: u32,
    pub height: u32,
    pub modifier: u64,
    pub planes_per_buffer: u32,
    /// `count * planes_per_buffer` entries, flattened (buffer, plane).
    pub stride: Vec<u32>,
    /// `count * planes_per_buffer` entries, flattened (buffer, plane).
    pub plane_offset: Vec<u32>,
    /// `count * planes_per_buffer` entries, flattened (buffer, plane).
    /// Per-plane memory span in bytes.
    pub size: Vec<u64>,
    /// `count * planes_per_buffer` entries, flattened (buffer, plane).
    /// Multi-plane modifiers may repeat the same underlying dma-buf fd.
    pub fds: Vec<OwnedFd>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameSnapshot {
    pub buffer_generation: u64,
    pub buffer_index: u32,
    pub seq: u64,
    pub release_point: u64,
}

/// Bit 0 of `BindSnapshot::flags` / `ControlMsg::ConfigureBuffers.flags`:
/// the renderer must back the dmabuf with HOST_VISIBLE memory.
pub const BUF_HOST_VISIBLE: u32 = 1 << 0;

/// DRM render-node identity reported by a renderer in its `Ready` event.
/// `(0, 0)` is the sentinel for an unknown render node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct DrmNode {
    pub major: u32,
    pub minor: u32,
}

impl DrmNode {
    pub const UNKNOWN: Self = Self { major: 0, minor: 0 };
    pub fn is_known(&self) -> bool {
        self.major != 0 || self.minor != 0
    }
}

/// Upper bound on the number of per-seq sync_fd entries the reader
/// keeps around before evicting the oldest.
const SYNC_FD_RETENTION: usize = 16;

/// Per-renderer state. Cheap to clone via `Arc`; the inner fields are
/// shared across HTTP handlers and the reader thread.
pub struct RendererHandle {
    pub id: RendererId,
    pub wp_type: WallpaperType,
    /// The `SpawnRequest.extras` this renderer was started with —
    /// canonical resource path plus manifest-allowlisted keys.
    pub extras: HashMap<String, String>,
    /// Renderer plugin name from the resolved `RendererDef` (e.g.
    /// `"wescene"`). Surfaced to the UI as the renderer name.
    pub name: String,
    /// OS pid of the renderer child captured right after `spawn()`.
    /// `None` only if Tokio could not return a child pid.
    pub pid: Option<u32>,
    /// DRM render-node id of the GPU the renderer's Vulkan instance
    /// picked. Reported in Ready and used by DMA-BUF negotiation.
    pub gpu: DrmNode,

    /// Blocking std UnixStream. Guarded by a std Mutex so HTTP handlers
    /// hold the lock only while a `sendmsg` is in flight.
    sock: Arc<StdMutex<StdUnixStream>>,

    /// Broadcast of every event the host emits (besides the FDs on the
    /// initial BindBuffers, whose fds are stored in `bind_snapshot`).
    events: broadcast::Sender<EventMsg>,

    /// Populated when the host sends its first `BindBuffers` event.
    bind_snapshot: Arc<StdMutex<Option<BindSnapshot>>>,

    /// In-flight `ConfigureBuffers` request. `Some(flags)` while the
    /// router has asked for a re-export not yet answered by BindBuffers.
    pending_configure: Arc<StdMutex<Option<u32>>>,

    /// Per-frame acquire fence file descriptors, indexed by `seq`.
    /// The reader thread stashes the fd attached to each FrameReady event.
    sync_fds: Arc<StdMutex<std::collections::VecDeque<(u64, OwnedFd)>>>,

    /// Most recent frame metadata, tied to the active bind generation.
    latest_frame: Arc<StdMutex<Option<FrameSnapshot>>>,

    /// Producer-exported timeline drm_syncobj used as the release
    /// fence target. Populated by a ReleaseSyncobj event.
    release_syncobj: Arc<StdMutex<Option<OwnedFd>>>,

    /// Modifier-negotiation capabilities the producer declared in
    /// its FormatCaps event.
    format_caps: Arc<StdMutex<Option<crate::dma::negotiate::PeerCaps>>>,

    /// Last `NegotiatedScheme` the daemon dispatched via
    /// NegotiateBuffers to this renderer, used for idempotence.
    last_dispatched_scheme: Arc<StdMutex<Option<crate::dma::negotiate::NegotiatedScheme>>>,

    /// Sink for per-frame [`crate::sync::FrameRecord`]s. The display
    /// endpoint pushes one record per consumer per frame.
    frame_record_tx: Option<tokio::sync::mpsc::UnboundedSender<crate::sync::FrameRecord>>,

    /// The child process. Kept alive so dropping the manager reaps it.
    child: Arc<TokioMutex<Option<Child>>>,

    /// Inbound-event family subscriptions copied from the renderer's
    /// manifest at spawn time. Pointer senders consult this before dispatch.
    events_subscribed: Arc<Vec<String>>,

    /// Renderer-published clear color (RGBA, 0..=1, sRGB straight
    /// alpha). Sole source for outbound display clear color.
    clear_rgba: Arc<StdMutex<[f32; 4]>>,
}

impl RendererHandle {
    pub fn events(&self) -> broadcast::Receiver<EventMsg> {
        self.events.subscribe()
    }

    pub fn frame_ready_seen(&self) -> bool {
        self.sync_fds.lock().map(|g| !g.is_empty()).unwrap_or(false)
    }

    pub fn latest_frame(&self) -> Option<FrameSnapshot> {
        let frame = self.latest_frame.lock().ok().and_then(|g| *g)?;
        let sync_fds = self.sync_fds.lock().ok()?;
        sync_fds
            .iter()
            .any(|(seq, _)| *seq == frame.seq)
            .then_some(frame)
    }

    /// Borrow the cached bind snapshot. Returns `None` until the host's
    /// first frame has been rendered and the fds arrived.
    pub fn bind_snapshot(&self) -> Arc<StdMutex<Option<BindSnapshot>>> {
        Arc::clone(&self.bind_snapshot)
    }

    /// Actual texture dimensions reported by the renderer's most recent
    /// `BindBuffers`. Returns `(0, 0)` before the first BindBuffers.
    pub fn texture_size(&self) -> (u32, u32) {
        if let Ok(g) = self.bind_snapshot.lock() {
            if let Some(snap) = g.as_ref() {
                return (snap.width, snap.height);
            }
        }
        (0, 0)
    }

    /// Current placement flags from the latest `BindBuffers`, or 0 if
    /// no snapshot has arrived yet.
    pub fn current_flags(&self) -> u32 {
        self.bind_snapshot
            .lock()
            .ok()
            .and_then(|g| g.as_ref().map(|s| s.flags))
            .unwrap_or(0)
    }

    /// Whether a `ConfigureBuffers` request is currently in flight (sent
    /// to the renderer but not yet answered by BindBuffers).
    pub fn pending_configure(&self) -> Option<u32> {
        self.pending_configure.lock().ok().and_then(|g| *g)
    }

    /// Obtain a dup'd copy of the acquire sync_fd that arrived with
    /// `FrameReady` seq. Each caller gets an independent fd.
    pub fn clone_sync_fd(&self, seq: u64) -> Option<OwnedFd> {
        use std::os::fd::{AsRawFd, FromRawFd};
        let guard = self.sync_fds.lock().ok()?;
        let (_, fd) = guard.iter().find(|(s, _)| *s == seq)?;
        let dup_raw = nix::unistd::dup(fd.as_raw_fd()).ok()?;
        // SAFETY: nix::unistd::dup returned a fresh fd we now own.
        Some(unsafe { OwnedFd::from_raw_fd(dup_raw) })
    }

    /// Borrow a dup'd handle to the producer's release timeline
    /// syncobj fd. Returns `None` until ReleaseSyncobj arrives.
    pub fn clone_release_syncobj_fd(&self) -> Option<OwnedFd> {
        use std::os::fd::{AsRawFd, FromRawFd};
        let guard = self.release_syncobj.lock().ok()?;
        let fd = guard.as_ref()?;
        let dup_raw = nix::unistd::dup(fd.as_raw_fd()).ok()?;
        Some(unsafe { OwnedFd::from_raw_fd(dup_raw) })
    }

    /// Borrow a clone of the producer's declared modifier-negotiation
    /// capabilities. Returns `None` until FormatCaps arrives.
    pub fn format_caps(&self) -> Option<crate::dma::negotiate::PeerCaps> {
        self.format_caps.lock().ok().and_then(|g| g.clone())
    }

    /// Mutate the producer's blacklist with `(fourcc, modifier)`. The
    /// blacklist lives inside the producer's cached PeerCaps.
    pub fn blacklist_format(&self, fourcc: u32, modifier: u64) -> bool {
        let Ok(mut guard) = self.format_caps.lock() else {
            return false;
        };
        let Some(caps) = guard.as_mut() else {
            return false;
        };
        caps.blacklist.insert((fourcc, modifier))
    }

    /// Most recently dispatched [`crate::dma::negotiate::NegotiatedScheme`]
    /// for this renderer. `None` until a negotiation succeeds.
    pub fn current_scheme(&self) -> Option<crate::dma::negotiate::NegotiatedScheme> {
        self.last_dispatched_scheme.lock().ok().and_then(|g| *g)
    }

    /// True iff the renderer's most recent `BindBuffers` snapshot
    /// matches the most recently dispatched [`crate::dma::negotiate::NegotiatedScheme`]
    pub fn scheme_satisfied(&self) -> bool {
        let Some(scheme) = self.current_scheme() else {
            return false;
        };
        let snap = self.bind_snapshot();
        let Ok(guard) = snap.lock() else {
            return false;
        };
        match guard.as_ref() {
            Some(s) => s.fourcc == scheme.fourcc && s.modifier == scheme.modifier,
            None => false,
        }
    }

    /// Push a per-frame [`crate::sync::FrameRecord`] to the reaper.
    /// Called once per display consumer per frame.
    pub fn submit_frame_record(
        &self,
        record: crate::sync::FrameRecord,
    ) -> std::result::Result<(), &'static str> {
        let Some(tx) = self.frame_record_tx.as_ref() else {
            return Err("no reaper wired (test stub or unconfigured renderer)");
        };
        tx.send(record).map_err(|_| "reaper channel closed")
    }

    /// Renderer-published clear color (RGBA, 0..=1). Defaults to
    /// opaque black until the renderer reports state.
    pub fn clear_rgba(&self) -> [f32; 4] {
        self.clear_rgba
            .lock()
            .map(|g| *g)
            .unwrap_or([0.0, 0.0, 0.0, 1.0])
    }
}

// ---------------------------------------------------------------------------
// Manager

pub struct RendererManager {
    inner: TokioMutex<Inner>,
    /// Plugin registry mapping wallpaper types to renderer binaries.
    registry: RendererRegistry,
    /// Back-reference to the router, installed after construction via
    /// `attach_router`. Held weak to avoid a cycle with `Router::mgr`.
    router: OnceLock<StdWeak<Router>>,
    /// Cached `/dev/dri` enumeration from startup. Used at spawn time to
    /// translate `gpu_drm_dev` settings into render-node paths.
    gpus: OnceLock<Arc<Vec<crate::gpu::GpuInfo>>>,
    /// Dead-renderer signals queue here (from reader-thread exit or
    /// a send_control hitting EPIPE). One background task drains it.
    reap_tx: tokio::sync::mpsc::UnboundedSender<RendererId>,
    reap_rx: StdMutex<Option<tokio::sync::mpsc::UnboundedReceiver<RendererId>>>,
}

struct Inner {
    renderers: HashMap<RendererId, Arc<RendererHandle>>,
}

impl RendererManager {
    pub fn new(registry: RendererRegistry) -> Self {
        let (reap_tx, reap_rx) = tokio::sync::mpsc::unbounded_channel();
        Self {
            inner: TokioMutex::new(Inner {
                renderers: HashMap::new(),
            }),
            registry,
            router: OnceLock::new(),
            gpus: OnceLock::new(),
            reap_tx,
            reap_rx: StdMutex::new(Some(reap_rx)),
        }
    }

    /// Hand the manager the startup `/dev/dri` snapshot so spawn-time can
    /// resolve `gpu_drm_dev` selections into `render_node` paths.
    pub fn attach_gpus(&self, gpus: Arc<Vec<crate::gpu::GpuInfo>>) {
        let _ = self.gpus.set(gpus);
    }

    /// Wire the manager to the router. Must be called once after both
    /// sides have been constructed. Later calls are ignored.
    pub fn attach_router(&self, router: StdWeak<Router>) {
        let _ = self.router.set(router);
    }

    /// Start the background reaper task that drains `mark_dead`
    /// signals and runs async eviction.
    pub fn start_reaper(self: &Arc<Self>) {
        let rx = match self.reap_rx.lock() {
            Ok(mut g) => g.take(),
            Err(_) => return,
        };
        let Some(mut rx) = rx else { return };
        let this = Arc::clone(self);
        tokio::spawn(async move {
            while let Some(id) = rx.recv().await {
                this.evict(&id).await;
            }
        });
    }

    /// Test-only convenience: construct a manager whose registry has a
    /// single scene renderer when `$WAYWALLEN_RENDERER_BIN` is set.
    pub fn new_default() -> Self {
        let mut registry = RendererRegistry::new();
        if let Some(bin) = std::env::var_os("WAYWALLEN_RENDERER_BIN") {
            registry.register(RendererDef {
                name: "test-scene".to_string(),
                plugin_id: "test.plugin".to_string(),
                plugin_version: "0.0.0".to_string(),
                plugin_system: false,
                bin: PathBuf::from(bin),
                types: vec!["scene".to_string()],
                priority: 100,
                spawn_version: None,
                extras: Vec::new(),
                settings: Default::default(),
                events: Vec::new(),
            });
        }
        Self::new(registry)
    }

    /// Access the renderer registry (for HTTP introspection endpoints).
    pub fn registry(&self) -> &RendererRegistry {
        &self.registry
    }

    /// Spawn a fresh renderer-host subprocess, wait for its `Ready`
    /// event, and return its id. Cleans up the child on failure.
    pub async fn spawn(&self, mut req: SpawnRequest) -> Result<RendererId> {
        let id: RendererId = Uuid::new_v4().to_string();

        // Create a listening UDS at a temp path; the child connects to
        // it shortly after exec().
        let sock_path = temp_sock_path(&id);
        let _ = std::fs::remove_file(&sock_path);
        let listener = tokio::net::UnixListener::bind(&sock_path)
            .with_context(|| format!("bind {}", sock_path.display()))?;

        // Best-effort cleanup of the socket file at the end of spawn —
        // the connection survives unlink(2).
        let _cleanup = TempUnlink(sock_path.clone());

        let renderer_def = match req.renderer_name.as_deref() {
            Some(name) => self
                .registry
                .resolve_by_name(name)
                .ok_or_else(|| Error::RendererNotFound(name.to_string()))?
                .clone(),
            None => self
                .registry
                .resolve(&req.wp_type)
                .ok_or_else(|| Error::NoRendererForType(req.wp_type.clone()))?
                .clone(),
        };

        // Translate the user's GPU choice into a render-node path before
        // settings reach the subprocess.
        if let Some(raw) = req.settings.remove(crate::gpu::GPU_DRM_DEV_KEY) {
            if let Some((major, minor)) = crate::gpu::parse_drm_dev(&raw) {
                let resolved = self
                    .gpus
                    .get()
                    .and_then(|gs| gs.iter().find(|g| g.matches_render(major, minor)))
                    .and_then(|g| g.render_node.as_ref())
                    .and_then(|p| p.to_str().map(str::to_string));
                if let Some(path) = resolved {
                    req.settings
                        .insert(crate::gpu::RENDER_NODE_KEY.to_string(), path);
                } else {
                    log::warn!(
                        "spawn: gpu_drm_dev={raw} not in /dev/dri enumeration; \
                         dropping selection and letting renderer pick default"
                    );
                }
            } else {
                log::warn!("spawn: gpu_drm_dev={raw:?} not parseable as <major>:<minor>");
            }
        }

        // Build the Init message *before* spawning the child (no
        // orphan socket file lingers if validation fails here).
        let init_msg = build_init_msg(&req, &renderer_def);

        let mut cmd = Command::new(&renderer_def.bin);
        cmd.arg("--ipc").arg(&sock_path);
        // SPAWN_VERSION 3: extras (canonical `path` + plugin-specific
        // keys like `assets`/`workshop_id`) ride as `--<key> <value>`
        let mut extra_keys: Vec<&String> = req.extras.keys().collect();
        extra_keys.sort();
        for k in extra_keys {
            if k != "path" && !renderer_def.extras.iter().any(|w| w == k) {
                continue;
            }
            cmd.arg(format!("--{k}")).arg(&req.extras[k]);
        }
        cmd.kill_on_drop(true)
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());

        let mut child = cmd
            .spawn()
            .with_context(|| format!("spawn {}", renderer_def.bin.display()))?;
        let child_pid = child.id();

        // Accept, with a bound to avoid hanging forever on a broken host.
        let accept = listener.accept();
        let (tokio_stream, _addr) = tokio::time::timeout(Duration::from_secs(10), accept)
            .await
            .map_err(|_| {
                let _ = child.start_kill();
                Error::RendererSpawnFailed(
                    "timed out waiting for waywallen-renderer to connect back".into(),
                )
            })?
            .context("accept")?;

        // Convert to a blocking std UnixStream for the rest of the
        // lifecycle because ipc::uds uses blocking sendmsg/recvmsg.
        let std_stream = tokio_stream.into_std().context("UnixStream::into_std")?;
        std_stream
            .set_nonblocking(false)
            .context("clear O_NONBLOCK on accepted stream")?;

        // Emit typed Init right after accept; CLI extras only identify
        // launch resources now.
        let handshake_stream = std_stream
            .try_clone()
            .context("try_clone for Init handshake")?;
        let gpu =
            tokio::task::spawn_blocking(move || run_init_handshake(&handshake_stream, &init_msg))
                .await
                .context("init handshake join")?
                .map_err(|e| {
                    let _ = child.start_kill();
                    e
                })?;
        log::info!(
            "renderer {id}: Ready (drm_render={}:{})",
            gpu.major,
            gpu.minor
        );

        // Now wire up the permanent reader thread and store the handle.
        let (events_tx, _events_rx) = broadcast::channel::<EventMsg>(256);
        let bind_snapshot: Arc<StdMutex<Option<BindSnapshot>>> = Arc::new(StdMutex::new(None));
        let sync_fds: Arc<StdMutex<std::collections::VecDeque<(u64, OwnedFd)>>> =
            Arc::new(StdMutex::new(std::collections::VecDeque::new()));
        let latest_frame: Arc<StdMutex<Option<FrameSnapshot>>> = Arc::new(StdMutex::new(None));
        let release_syncobj: Arc<StdMutex<Option<OwnedFd>>> = Arc::new(StdMutex::new(None));
        let format_caps: Arc<StdMutex<Option<crate::dma::negotiate::PeerCaps>>> =
            Arc::new(StdMutex::new(None));
        let pending_configure: Arc<StdMutex<Option<u32>>> = Arc::new(StdMutex::new(None));
        let clear_rgba: Arc<StdMutex<[f32; 4]>> = Arc::new(StdMutex::new([0.0, 0.0, 0.0, 1.0]));

        let sock = Arc::new(StdMutex::new(std_stream));
        let reader_sock = sock.clone();
        let reader_events = events_tx.clone();
        let reader_snapshot = bind_snapshot.clone();
        let reader_sync_fds = sync_fds.clone();
        let reader_latest_frame = latest_frame.clone();
        let reader_release_syncobj = release_syncobj.clone();
        let reader_format_caps = format_caps.clone();
        let reader_pending = pending_configure.clone();
        let reader_clear_rgba = clear_rgba.clone();
        let reader_id = id.clone();
        let reader_reap_tx = self.reap_tx.clone();
        thread::spawn(move || {
            run_reader(
                reader_id,
                reader_sock,
                reader_events,
                reader_snapshot,
                reader_sync_fds,
                reader_latest_frame,
                reader_release_syncobj,
                reader_format_caps,
                reader_pending,
                reader_clear_rgba,
                reader_reap_tx,
            );
        });

        // Per-renderer reaper drains FrameRecords and transfers consumer
        // release fences onto the producer timeline.
        let (frame_tx, frame_rx) =
            tokio::sync::mpsc::unbounded_channel::<crate::sync::FrameRecord>();
        let frame_record_tx = match crate::sync::drm_device() {
            Ok(_) => Some(frame_tx),
            Err(e) => {
                log::warn!(
                    "renderer {id}: no DRM render node ({e}); release-syncobj reaper disabled"
                );
                None
            }
        };

        let handle = Arc::new(RendererHandle {
            id: id.clone(),
            wp_type: req.wp_type.clone(),
            extras: req.extras.clone(),
            name: renderer_def.name.clone(),
            pid: child_pid,
            gpu,
            sock,
            events: events_tx,
            bind_snapshot,
            sync_fds,
            latest_frame,
            release_syncobj,
            format_caps,
            last_dispatched_scheme: Arc::new(StdMutex::new(None)),
            frame_record_tx,
            pending_configure,
            child: Arc::new(TokioMutex::new(Some(child))),
            events_subscribed: Arc::new(renderer_def.events.clone()),
            clear_rgba,
        });

        if handle.frame_record_tx.is_some() {
            // SAFETY: drm_device() returned Ok above and is idempotent.
            let drm = crate::sync::drm_device().expect("checked above");
            // Pass only the renderer id and release_syncobj; the reaper
            // must not keep the whole RendererHandle alive.
            crate::sync::spawn_reaper(
                drm,
                id.clone(),
                Arc::clone(&handle.release_syncobj),
                frame_rx,
            );
        }

        {
            let mut inner = self.inner.lock().await;
            inner.renderers.insert(id.clone(), handle);
        }
        log::info!("spawned renderer {id} ({})", req.wp_type);
        Ok(id)
    }

    /// Find an already-running renderer whose **identity** matches
    /// `req`, ignoring runtime-tunable plugin settings.
    pub async fn find_reusable(&self, req: &SpawnRequest) -> Option<RendererId> {
        let def = match req.renderer_name.as_deref() {
            Some(name) => self.registry.resolve_by_name(name)?.clone(),
            None => self.registry.resolve(&req.wp_type)?.clone(),
        };

        let inner = self.inner.lock().await;
        for (id, h) in inner.renderers.iter() {
            if h.wp_type != req.wp_type || h.name != def.name {
                continue;
            }
            if h.extras != req.extras {
                continue;
            }
            return Some(id.clone());
        }
        None
    }

    pub async fn get(&self, id: &str) -> Option<Arc<RendererHandle>> {
        let inner = self.inner.lock().await;
        inner.renderers.get(id).cloned()
    }

    /// Locate a live renderer whose `extras["path"]` matches the given
    /// resource. Used to route property changes to the active renderer.
    pub async fn find_by_resource(&self, resource: &str) -> Option<Arc<RendererHandle>> {
        let inner = self.inner.lock().await;
        inner.renderers.values().find_map(|h| {
            (h.extras.get("path").map(String::as_str) == Some(resource)).then(|| h.clone())
        })
    }

    pub async fn list(&self) -> Vec<RendererId> {
        let inner = self.inner.lock().await;
        inner.renderers.keys().cloned().collect()
    }

    pub async fn wait_for_first_frame(&self, id: &str, timeout: Duration) -> Result<()> {
        let handle = self
            .get(id)
            .await
            .ok_or_else(|| Error::RendererNotFound(id.to_string()))?;
        if handle.frame_ready_seen() {
            return Ok(());
        }

        let mut events = handle.events();
        let deadline = tokio::time::sleep(timeout);
        tokio::pin!(deadline);
        let mut liveness = tokio::time::interval(Duration::from_millis(100));

        loop {
            if handle.frame_ready_seen() {
                return Ok(());
            }

            tokio::select! {
                _ = &mut deadline => {
                    return Err(Error::RendererFrameFailed(format!(
                        "timed out after {}s waiting for renderer '{id}' to send its first frame",
                        timeout.as_secs()
                    )));
                }
                _ = liveness.tick() => {
                    if self.get(id).await.is_none() {
                        return Err(Error::RendererFrameFailed(format!(
                            "renderer '{id}' exited before its first frame"
                        )));
                    }

                    let mut child_guard = handle.child.lock().await;
                    if let Some(child) = child_guard.as_mut() {
                        match child.try_wait() {
                            Ok(Some(status)) => {
                                self.mark_dead(id);
                                return Err(Error::RendererFrameFailed(format!(
                                    "renderer '{id}' exited before its first frame: {status}"
                                )));
                            }
                            Ok(None) => {}
                            Err(e) => {
                                return Err(Error::RendererFrameFailed(format!(
                                    "failed to check renderer '{id}' liveness: {e}"
                                )));
                            }
                        }
                    }
                }
                recv = events.recv() => {
                    match recv {
                        Ok(EventMsg::FrameReady { .. }) => return Ok(()),
                        Ok(_) => {}
                        Err(broadcast::error::RecvError::Lagged(_)) => {
                            if handle.frame_ready_seen() {
                                return Ok(());
                            }
                        }
                        Err(broadcast::error::RecvError::Closed) => {
                            return Err(Error::RendererFrameFailed(format!(
                                "renderer '{id}' event stream closed before its first frame"
                            )));
                        }
                    }
                }
            }
        }
    }

    /// Fire-and-forget control send. Returns an error if the renderer
    /// is unknown or the underlying socket write fails.
    pub async fn send_control(&self, id: &str, msg: ControlMsg) -> Result<()> {
        let handle = self
            .get(id)
            .await
            .ok_or_else(|| Error::RendererNotFound(id.to_string()))?;
        let sock = handle.sock.clone();
        let codec_res: Result<std::result::Result<(), CodecError>> =
            tokio::task::spawn_blocking(move || {
                let guard = sock.lock().map_err(|e| {
                    Error::RendererControlFailed(format!("sock mutex poisoned: {e}"))
                })?;
                Ok(send_control(&*guard, &msg, &[]))
            })
            .await
            .context("send_control join")?;
        match codec_res? {
            Ok(()) => Ok(()),
            Err(e) => {
                if is_peer_gone(&e) {
                    log::warn!("renderer {id}: peer gone on send_control ({e}), evicting");
                    self.mark_dead(id);
                }
                Err(Error::RendererControlFailed(format!("send_control: {e}")))
            }
        }
    }

    /// Modifier-negotiation v2 dispatch — replaces the deleted
    /// `send_configure_buffers`.
    pub async fn send_negotiate_buffers(
        &self,
        id: &str,
        scheme: crate::dma::negotiate::NegotiatedScheme,
    ) -> Result<()> {
        let handle = self
            .get(id)
            .await
            .ok_or_else(|| Error::RendererNotFound(id.to_string()))?;
        // Idempotence: skip if we've already dispatched this exact scheme.
        if let Ok(guard) = handle.last_dispatched_scheme.lock() {
            if guard.as_ref() == Some(&scheme) {
                return Ok(());
            }
        }
        log::info!(
            "renderer {id}: NegotiateBuffers fourcc=0x{:08x} modifier=0x{:x} \
             plane_count={} sync=0x{:x} color=0x{:x} mem_hint=0x{:x} \
             count={} path={:?} mem_source={:?}",
            scheme.fourcc,
            scheme.modifier,
            scheme.plane_count,
            scheme.sync_mode,
            scheme.color,
            scheme.mem_hint,
            scheme.count,
            scheme.path,
            scheme.mem_source,
        );
        let msg = ControlMsg::NegotiateBuffers {
            fourcc: scheme.fourcc,
            modifier: scheme.modifier,
            plane_count: scheme.plane_count,
            sync_mode: scheme.sync_mode,
            color: scheme.color,
            mem_hint: scheme.mem_hint,
            count: scheme.count,
            path: scheme.path.as_u32(),
            mem_source: scheme.mem_source.as_u32(),
        };
        self.send_control(id, msg).await?;
        if let Ok(mut guard) = handle.last_dispatched_scheme.lock() {
            *guard = Some(scheme);
        }
        Ok(())
    }

    /// Push a `setting_changed` event to a live renderer. `settings` is
    /// the caller-filtered runtime delta.
    pub async fn send_setting_changed(
        &self,
        id: &str,
        settings: Vec<(String, String)>,
        fps: Option<u32>,
    ) -> Result<()> {
        let handle = self
            .get(id)
            .await
            .ok_or_else(|| Error::RendererNotFound(id.to_string()))?;
        // setting_changed is a pure kv list. fps is just one of the kv
        // keys (when the manifest declares it), not a typed scalar.
        let mut settings = settings;
        if let Some(f) = fps {
            if f != 0 {
                settings.retain(|(k, _)| k != "fps");
                settings.push(("fps".to_string(), f.to_string()));
            }
        }
        let msg = ControlMsg::SettingChanged {
            settings: settings.clone(),
        };
        log::info!(
            "renderer {id}: setting_changed keys={:?}",
            settings.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>(),
        );
        self.send_control(id, msg).await?;
        let _ = handle;
        Ok(())
    }

    /// Forward a pointer-motion event to a live renderer. Silently
    /// drops when the renderer did not subscribe to pointer events.
    pub async fn send_pointer_motion(
        &self,
        id: &str,
        x: f32,
        y: f32,
        timestamp_us: u64,
        modifiers: u32,
    ) -> Result<()> {
        if !self.subscribed_to(id, EVENT_KIND_POINTER).await {
            return Ok(());
        }
        self.send_control(
            id,
            ControlMsg::PointerMotion {
                x,
                y,
                timestamp_us,
                modifiers,
            },
        )
        .await
    }

    /// Forward a pointer-button event. Same gating as
    /// [`Self::send_pointer_motion`].
    pub async fn send_pointer_button(
        &self,
        id: &str,
        x: f32,
        y: f32,
        button: u32,
        state: u32,
        timestamp_us: u64,
        modifiers: u32,
    ) -> Result<()> {
        if !self.subscribed_to(id, EVENT_KIND_POINTER).await {
            return Ok(());
        }
        self.send_control(
            id,
            ControlMsg::PointerButton {
                x,
                y,
                button,
                state,
                timestamp_us,
                modifiers,
            },
        )
        .await
    }

    /// Forward a pointer-axis (scroll) event. Same gating as
    /// [`Self::send_pointer_motion`].
    pub async fn send_pointer_axis(
        &self,
        id: &str,
        x: f32,
        y: f32,
        delta_x: f32,
        delta_y: f32,
        source: u32,
        timestamp_us: u64,
        modifiers: u32,
    ) -> Result<()> {
        if !self.subscribed_to(id, EVENT_KIND_POINTER).await {
            return Ok(());
        }
        self.send_control(
            id,
            ControlMsg::PointerAxis {
                x,
                y,
                delta_x,
                delta_y,
                source,
                timestamp_us,
                modifiers,
            },
        )
        .await
    }

    /// Forward an MPRIS media snapshot to a live renderer. Silently
    /// drops when the renderer did not subscribe to MPRIS events.
    pub async fn send_mpris(&self, id: &str, snapshot: MprisSnapshot) -> Result<()> {
        if !self.subscribed_to(id, EVENT_KIND_MPRIS).await {
            return Ok(());
        }
        self.send_control(
            id,
            ControlMsg::Mpris {
                state: snapshot.state,
                title: snapshot.title,
                artist: snapshot.artist,
                album: snapshot.album,
                album_artist: snapshot.album_artist,
                art_url: snapshot.art_url,
                previous_art_url: snapshot.previous_art_url,
            },
        )
        .await
    }

    /// Returns `true` when the renderer is alive and its manifest
    /// declared `events = [..., kind, ...]`. Unknown id ⇒ `false`
    async fn subscribed_to(&self, id: &str, kind: &str) -> bool {
        match self.get(id).await {
            Some(h) => h.events_subscribed.iter().any(|e| e == kind),
            None => false,
        }
    }

    /// Enqueue a renderer for eviction. Synchronous (cheap channel
    /// send); cleanup happens on the reaper task.
    pub fn mark_dead(&self, id: &str) {
        if self.reap_tx.send(id.to_string()).is_err() {
            log::warn!("renderer {id}: mark_dead dropped (reaper channel closed)");
        }
    }

    /// Actual eviction: remove from map, unregister from router, kill
    /// child. Called only by the reaper task and is idempotent.
    async fn evict(self: &Arc<Self>, id: &str) {
        let handle = {
            let mut inner = self.inner.lock().await;
            inner.renderers.remove(id)
        };
        let Some(handle) = handle else { return };
        log::warn!("renderer {id}: evicting");

        if let Some(router) = self.router.get().and_then(|w| w.upgrade()) {
            router.unregister_renderer(id).await;
        }

        let mut child_guard = handle.child.lock().await;
        if let Some(mut child) = child_guard.take() {
            let _ = child.start_kill();
            let _ = tokio::time::timeout(Duration::from_secs(2), child.wait()).await;
        }
    }

    /// Send Shutdown, wait for the child to exit gracefully, escalate
    /// to SIGKILL only if it doesn't. Removes from the map.
    pub async fn kill(&self, id: &str) -> Result<()> {
        let handle = {
            let mut inner = self.inner.lock().await;
            inner.renderers.remove(id)
        }
        .ok_or_else(|| Error::RendererNotFound(id.to_string()))?;

        // Send Shutdown over the bridge socket.
        let sock = handle.sock.clone();
        let _ = tokio::task::spawn_blocking(move || {
            if let Ok(guard) = sock.lock() {
                let _ = send_control(&*guard, &ControlMsg::Shutdown, &[]);
            }
        })
        .await;

        let mut child_guard = handle.child.lock().await;
        if let Some(mut child) = child_guard.take() {
            // 5 s: comfortably above any plausible vkDeviceWaitIdle
            // under load; image is usually microseconds, mpv/wescene slower.
            match tokio::time::timeout(Duration::from_secs(5), child.wait()).await {
                Ok(_) => {
                    log::info!("renderer {id}: graceful shutdown");
                }
                Err(_) => {
                    log::warn!("renderer {id}: Shutdown timeout (5s), escalating to SIGKILL");
                    let _ = child.start_kill();
                    let _ = tokio::time::timeout(Duration::from_secs(1), child.wait()).await;
                }
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Reader thread

fn run_reader(
    id: RendererId,
    sock: Arc<StdMutex<StdUnixStream>>,
    events: broadcast::Sender<EventMsg>,
    bind_snapshot: Arc<StdMutex<Option<BindSnapshot>>>,
    sync_fds: Arc<StdMutex<std::collections::VecDeque<(u64, OwnedFd)>>>,
    latest_frame: Arc<StdMutex<Option<FrameSnapshot>>>,
    release_syncobj: Arc<StdMutex<Option<OwnedFd>>>,
    format_caps: Arc<StdMutex<Option<crate::dma::negotiate::PeerCaps>>>,
    pending_configure: Arc<StdMutex<Option<u32>>>,
    clear_rgba: Arc<StdMutex<[f32; 4]>>,
    reap_tx: tokio::sync::mpsc::UnboundedSender<RendererId>,
) {
    // Any reader exit enqueues renderer eviction so stale ids do not remain
    // registered after EOF, recvmsg error, or panic.
    let _reap = ReaperOnDrop {
        id: id.clone(),
        tx: reap_tx,
    };

    // Hold the stream by dup'ing the raw fd so the blocking recv is not
    // contending with sends on the same mutex.
    let read_stream = {
        let guard = match sock.lock() {
            Ok(g) => g,
            Err(_) => {
                log::error!("renderer {id}: sock mutex poisoned, reader exiting");
                return;
            }
        };
        match guard.try_clone() {
            Ok(s) => s,
            Err(e) => {
                log::error!("renderer {id}: try_clone failed: {e}");
                return;
            }
        }
    };

    loop {
        let received = match recv_event(&read_stream) {
            Ok(ok) => ok,
            Err(e) => {
                log::info!("renderer {id}: reader exit: {e}");
                return;
            }
        };
        let (msg, fds) = received;

        // Cache each BindBuffers snapshot with its fds; later generations
        // replace earlier ones.
        if let EventMsg::BindBuffers {
            generation,
            flags,
            count,
            fourcc,
            width,
            height,
            modifier,
            planes_per_buffer,
            ref stride,
            ref plane_offset,
            ref size,
        } = msg
        {
            // Validate parallel arrays up front so all per-plane fields
            // stay index-aligned.
            let expected = (count as usize) * (planes_per_buffer as usize);
            if stride.len() != expected
                || plane_offset.len() != expected
                || size.len() != expected
                || fds.len() != expected
            {
                log::warn!(
                    "renderer {id}: BindBuffers length mismatch \
                     count={count} planes={planes_per_buffer} expected={expected} \
                     stride={} offset={} size={} fds={}; dropping",
                    stride.len(),
                    plane_offset.len(),
                    size.len(),
                    fds.len()
                );
            } else if fds.is_empty() {
                log::warn!("renderer {id}: BindBuffers arrived without fds");
            } else {
                let prev_gen = bind_snapshot
                    .lock()
                    .ok()
                    .and_then(|g| g.as_ref().map(|s| s.generation));
                if let Some(prev) = prev_gen {
                    if generation <= prev {
                        log::warn!(
                            "renderer {id}: BindBuffers gen={generation} not > prev {prev}; \
                             accepting anyway but display protocol expects monotonicity"
                        );
                    }
                }
                let snap = BindSnapshot {
                    generation,
                    flags,
                    count,
                    fourcc,
                    width,
                    height,
                    modifier,
                    planes_per_buffer,
                    stride: stride.clone(),
                    plane_offset: plane_offset.clone(),
                    size: size.clone(),
                    fds,
                };
                if let Ok(mut guard) = bind_snapshot.lock() {
                    *guard = Some(snap);
                    log::info!(
                        "renderer {id}: BindBuffers cached (gen={generation}, flags=0x{flags:x})"
                    );
                }
                // A rebind retires acquire fences from the previous
                // buffer_generation.
                if let Ok(mut guard) = sync_fds.lock() {
                    guard.clear();
                }
                if let Ok(mut guard) = latest_frame.lock() {
                    *guard = None;
                }
                // Clear any in-flight ConfigureBuffers, warning if the
                // renderer answered with different flags.
                if let Ok(mut guard) = pending_configure.lock() {
                    if let Some(want) = guard.take() {
                        if want != flags {
                            log::warn!(
                                "renderer {id}: ConfigureBuffers asked for \
                                 flags=0x{want:x} but renderer answered \
                                 with flags=0x{flags:x}; accepting"
                            );
                        }
                    }
                }
            }
        } else if let EventMsg::FrameReady {
            image_index,
            seq,
            release_point,
            ..
        } = msg
        {
            // frame_ready always carries exactly one sync_fd: the codec
            // enforced expected_fds() == 1 before handing us `fds`.
            let mut taken = fds;
            let fd = taken.remove(0);
            if let Ok(mut guard) = sync_fds.lock() {
                while guard.len() >= SYNC_FD_RETENTION {
                    guard.pop_front();
                }
                guard.push_back((seq, fd));
            }
            let gen = bind_snapshot
                .lock()
                .ok()
                .and_then(|g| g.as_ref().map(|s| s.generation));
            if let Some(buffer_generation) = gen {
                if let Ok(mut guard) = latest_frame.lock() {
                    *guard = Some(FrameSnapshot {
                        buffer_generation,
                        buffer_index: image_index,
                        seq,
                        release_point,
                    });
                }
            }
        } else if let EventMsg::ReleaseSyncobj = msg {
            // Producer's exported timeline drm_syncobj. Exactly one fd;
            // the codec enforced expected_fds() == 1.
            let mut taken = fds;
            let fd = taken.remove(0);
            if let Ok(mut guard) = release_syncobj.lock() {
                if guard.is_some() {
                    log::warn!(
                        "renderer {id}: ReleaseSyncobj received twice; \
                         replacing previous fd"
                    );
                }
                *guard = Some(fd);
                log::info!("renderer {id}: ReleaseSyncobj imported");
            }
        } else if let EventMsg::FormatCaps {
            ref fourccs,
            ref mod_counts,
            ref modifiers,
            ref plane_counts,
            ref device_uuid,
            ref driver_uuid,
            drm_render_major,
            drm_render_minor,
            mem_hints,
            sync_caps,
            color_caps,
            extent_max_w,
            extent_max_h,
        } = msg
        {
            let drm = DrmNode {
                major: drm_render_major,
                minor: drm_render_minor,
            };
            match crate::dma::negotiate::unflatten_caps(
                fourccs,
                mod_counts,
                modifiers,
                plane_counts,
                device_uuid,
                driver_uuid,
                drm,
                sync_caps,
                color_caps,
                mem_hints,
                (extent_max_w, extent_max_h),
            ) {
                Ok(caps) => {
                    if let Ok(mut guard) = format_caps.lock() {
                        if guard.is_some() {
                            log::warn!(
                                "renderer {id}: FormatCaps received twice; \
                                 replacing previous caps"
                            );
                        }
                        let prefix = format!("renderer {id}: format_caps");
                        log::info!(
                            "{prefix}: imported {} fourcc{}",
                            caps.formats.by_fourcc.len(),
                            if caps.formats.by_fourcc.len() == 1 {
                                ""
                            } else {
                                "s"
                            },
                        );
                        caps.log_dump(&prefix);
                        *guard = Some(caps);
                    }
                }
                Err(e) => {
                    log::warn!("renderer {id}: FormatCaps malformed: {e:?}");
                }
            }
        } else if let EventMsg::BindFailed {
            fourcc,
            modifier,
            reason,
            ref message,
        } = msg
        {
            // Renderer-side bind failure is surfaced for debugging; router
            // retry paths handle consumer-side failures.
            log::warn!(
                "renderer {id}: BindFailed fourcc=0x{fourcc:08x} \
                 modifier=0x{modifier:x} reason={reason} msg={message:?}"
            );
        } else if let EventMsg::ReportState { ref state } = msg {
            // Recognised keys are stashed on the handle; unknown keys
            // are ignored. Currently only `clear_color` is consumed.
            for (k, v) in state.iter() {
                if k == "clear_color" {
                    if let Some(rgba) = parse_clear_color(v) {
                        if let Ok(mut g) = clear_rgba.lock() {
                            *g = rgba;
                        }
                    } else {
                        log::warn!(
                            "renderer {id}: ReportState clear_color={v:?} unparseable, ignored"
                        );
                    }
                }
            }
        } else if !fds.is_empty() {
            log::warn!("renderer {id}: unexpected fds on event {msg:?}, dropping");
        }

        // Broadcast to any subscribers. No subscribers means no error:
        // SendError is only returned when receivers drop, which is fine.
        let _ = events.send(msg);
    }
}

// ---------------------------------------------------------------------------
// Helpers

/// True when a `send_control` / `recv_event` error indicates the peer
/// is gone, so callers can evict the renderer.
fn is_peer_gone(err: &CodecError) -> bool {
    use nix::errno::Errno;
    matches!(
        err,
        CodecError::PeerClosed
            | CodecError::Nix(Errno::EPIPE | Errno::ECONNRESET | Errno::ENOTCONN)
    )
}

/// RAII guard that enqueues the renderer for eviction when the reader
/// thread drops on any exit path.
struct ReaperOnDrop {
    id: RendererId,
    tx: tokio::sync::mpsc::UnboundedSender<RendererId>,
}

impl Drop for ReaperOnDrop {
    fn drop(&mut self) {
        let id = std::mem::take(&mut self.id);
        let _ = self.tx.send(id);
    }
}

fn temp_sock_path(id: &str) -> PathBuf {
    let runtime_dir = std::env::var("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir());
    let dir = runtime_dir.join("waywallen");
    let _ = std::fs::create_dir_all(&dir);
    dir.join(format!("renderer-{id}.sock"))
}

struct TempUnlink(PathBuf);
impl Drop for TempUnlink {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Build the typed `Init` control message the daemon emits right
/// after a renderer subprocess connects back.
pub(crate) fn build_init_msg(req: &SpawnRequest, def: &RendererDef) -> ControlMsg {
    let spawn_version = def.spawn_version.unwrap_or(SPAWN_VERSION);

    let mut settings_kv: HashMap<String, String> = req.settings.clone();

    if def.settings.contains_key("test_pattern") && req.test_pattern {
        settings_kv.insert("test_pattern".to_string(), "1".to_string());
    }

    let mut settings: Vec<(String, String)> = settings_kv.into_iter().collect();
    settings.sort_by(|a, b| a.0.cmp(&b.0));

    ControlMsg::Init {
        spawn_version,
        settings,
        user_properties: req.user_properties_json.clone().unwrap_or_default(),
    }
}

/// Run the post-accept handshake on a blocking std `UnixStream`:
/// send typed Init, then read exactly one event.
pub(crate) fn run_init_handshake(sock: &StdUnixStream, init: &ControlMsg) -> Result<DrmNode> {
    send_control(sock, init, &[])
        .map_err(|e| Error::RendererSpawnFailed(format!("send Init: {e}")))?;
    let (evt, fds) =
        recv_event(sock).map_err(|e| Error::RendererSpawnFailed(format!("recv Ready: {e}")))?;
    match evt {
        EventMsg::Ready {
            drm_render_major,
            drm_render_minor,
        } => {
            if !fds.is_empty() {
                log::warn!("Ready unexpectedly carried {} fds; dropping", fds.len());
            }
            Ok(DrmNode {
                major: drm_render_major,
                minor: drm_render_minor,
            })
        }
        EventMsg::InitNack {
            received_spawn_version,
            supported_spawn_version,
            reason,
        } => Err(Error::RendererSpawnFailed(format!(
            "renderer rejected Init: {reason} (received spawn_version={received_spawn_version}, \
             supported={supported_spawn_version})"
        ))),
        other => Err(Error::RendererSpawnFailed(format!(
            "host emitted {other:?} before Ready; aborting spawn"
        ))),
    }
}

#[allow(dead_code)]
fn _assert_path_ok<P: AsRef<std::path::Path>>(_p: P) {} // compile-time shim

/// Parse a `"r,g,b,a"` clear-color value. Components clamped to
/// `[0, 1]`; malformed strings return `None`.
fn parse_clear_color(s: &str) -> Option<[f32; 4]> {
    let parts: Vec<&str> = s.split(',').map(str::trim).collect();
    if parts.len() != 4 {
        return None;
    }
    let mut out = [0.0f32; 4];
    for (i, p) in parts.iter().enumerate() {
        let v: f32 = p.parse().ok()?;
        if !v.is_finite() {
            return None;
        }
        out[i] = v.clamp(0.0, 1.0);
    }
    Some(out)
}

// ---------------------------------------------------------------------------
// Test stubs

#[cfg(test)]
impl RendererHandle {
    /// Test-only: inject a `PeerCaps` so router-level negotiation
    /// tests can pretend the renderer shipped a `FormatCaps` event.
    pub fn test_set_format_caps(&self, caps: crate::dma::negotiate::PeerCaps) {
        if let Ok(mut g) = self.format_caps.lock() {
            *g = Some(caps);
        }
    }

    /// Test-only: read the producer's blacklist length. Lets a
    /// router-side test assert bind-failure blacklist mutation.
    pub fn test_blacklist_len(&self) -> usize {
        self.format_caps
            .lock()
            .ok()
            .and_then(|g| g.as_ref().map(|c| c.blacklist.len()))
            .unwrap_or(0)
    }

    pub fn test_set_latest_frame(&self, frame: FrameSnapshot) {
        use nix::sys::memfd::{memfd_create, MemFdCreateFlag};
        use std::ffi::CString;

        let name = CString::new("waywallen-frame-test").unwrap();
        let fd = memfd_create(&name, MemFdCreateFlag::MFD_CLOEXEC).unwrap();
        if let Ok(mut guard) = self.sync_fds.lock() {
            guard.push_back((frame.seq, fd));
        }
        if let Ok(mut guard) = self.latest_frame.lock() {
            *guard = Some(frame);
        }
    }
}

impl RendererHandle {
    /// Construct a `RendererHandle` with no running child process.
    /// Used by routing-table unit tests.
    pub fn test_stub(id: &str, wp_type: &str) -> Arc<Self> {
        let (handle, _peer) = Self::test_stub_with_peer_inner(id, wp_type);
        handle
    }

    #[cfg(test)]
    pub fn test_stub_with_peer(id: &str, wp_type: &str) -> (Arc<Self>, StdUnixStream) {
        Self::test_stub_with_peer_inner(id, wp_type)
    }

    fn test_stub_with_peer_inner(id: &str, wp_type: &str) -> (Arc<Self>, StdUnixStream) {
        let (a, b) = StdUnixStream::pair().expect("UnixStream pair");
        let (events_tx, _) = broadcast::channel::<EventMsg>(8);
        let handle = Arc::new(Self {
            id: id.into(),
            wp_type: wp_type.into(),
            extras: HashMap::new(),
            name: "test-stub".into(),
            pid: None,
            gpu: DrmNode::UNKNOWN,
            sock: Arc::new(StdMutex::new(a)),
            events: events_tx,
            bind_snapshot: Arc::new(StdMutex::new(None)),
            sync_fds: Arc::new(StdMutex::new(std::collections::VecDeque::new())),
            latest_frame: Arc::new(StdMutex::new(None)),
            release_syncobj: Arc::new(StdMutex::new(None)),
            format_caps: Arc::new(StdMutex::new(None)),
            last_dispatched_scheme: Arc::new(StdMutex::new(None)),
            frame_record_tx: None,
            pending_configure: Arc::new(StdMutex::new(None)),
            child: Arc::new(TokioMutex::new(None)),
            events_subscribed: Arc::new(Vec::new()),
            clear_rgba: Arc::new(StdMutex::new([0.0, 0.0, 0.0, 1.0])),
        });
        (handle, b)
    }
}

impl RendererManager {
    /// Insert a pre-built handle into the manager's map without
    /// spawning a child process. Used by routing-table unit tests.
    pub async fn register_test_handle(&self, handle: Arc<RendererHandle>) {
        let mut inner = self.inner.lock().await;
        inner.renderers.insert(handle.id.clone(), handle);
    }
}

#[cfg(test)]
mod init_handshake_tests {
    use super::*;
    use crate::ipc::uds::send_event;
    use crate::plugin::renderer_registry::{SettingDef, SettingType};
    use std::path::PathBuf;
    use std::thread;

    fn def_legacy(name: &str) -> RendererDef {
        // Legacy (no-schema) manifest: build_init_msg falls back to
        // the hard-coded primary-key priority list.
        RendererDef {
            name: name.to_string(),
            plugin_id: "test.plugin".to_string(),
            plugin_version: "0.0.0".to_string(),
            plugin_system: false,
            bin: PathBuf::from("/dev/null"),
            types: vec!["scene".to_string()],
            priority: 100,
            spawn_version: None,
            extras: Vec::new(),
            settings: Default::default(),
            events: Vec::new(),
        }
    }

    fn def_scene_schema() -> RendererDef {
        RendererDef {
            name: "wescene-renderer".into(),
            plugin_id: "test.plugin".to_string(),
            plugin_version: "0.0.0".to_string(),
            plugin_system: false,
            bin: PathBuf::from("/dev/null"),
            types: vec!["scene".into()],
            priority: 100,
            spawn_version: Some(1),
            extras: vec!["assets".into(), "workshop_id".into()],
            settings: Default::default(),
            events: Vec::new(),
        }
    }

    fn def_mpv_schema() -> RendererDef {
        let mut ps = HashMap::new();
        ps.insert(
            "loop_file".to_string(),
            SettingDef::new(
                SettingType::String,
                toml::Value::String("inf".into()),
                false,
            ),
        );
        RendererDef {
            name: "waywallen-mpv".into(),
            plugin_id: "test.plugin".to_string(),
            plugin_version: "0.0.0".to_string(),
            plugin_system: false,
            bin: PathBuf::from("/dev/null"),
            types: vec!["video".into()],
            priority: 100,
            spawn_version: Some(1),
            extras: Vec::new(),
            settings: ps,
            events: Vec::new(),
        }
    }

    // Legacy Init-shape tests were removed after Init became plain settings
    // plus user_properties.

    #[test]
    fn slim_init_carries_extent_and_settings_kv() {
        // Init carries settings kv verbatim; callers own sourcing them from
        // the settings store.
        let mut settings_in = HashMap::new();
        settings_in.insert("loop_file".to_string(), "inf".to_string());
        let req = SpawnRequest {
            extras: HashMap::new(),
            wp_type: "video".into(),
            settings: settings_in,
            test_pattern: false,
            renderer_name: None,
            user_properties_json: None,
        };
        let msg = build_init_msg(&req, &def_mpv_schema());
        match msg {
            ControlMsg::Init {
                spawn_version,
                settings,
                user_properties,
            } => {
                assert_eq!(spawn_version, 1); // pulled from def_mpv_schema
                assert_eq!(settings, vec![("loop_file".to_string(), "inf".to_string())]);
                assert_eq!(user_properties, "");
            }
            other => panic!("expected ControlMsg::Init, got {other:?}"),
        }
    }

    #[test]
    fn spawn_handshake_init_nack_aborts() {
        // Drive the daemon side over a socketpair while the peer replies
        // with InitNack.
        let (daemon, renderer) = StdUnixStream::pair().expect("UnixStream::pair");
        daemon
            .set_nonblocking(false)
            .expect("set_nonblocking(false) on daemon side");
        renderer
            .set_nonblocking(false)
            .expect("set_nonblocking(false) on renderer side");

        let peer = thread::spawn(move || {
            // Receive the Init then immediately reply with InitNack.
            let (got, _fds) = crate::ipc::uds::recv_control(&renderer).expect("renderer recv Init");
            assert!(matches!(got, ControlMsg::Init { .. }));
            send_event(
                &renderer,
                &EventMsg::InitNack {
                    received_spawn_version: 999,
                    supported_spawn_version: SPAWN_VERSION,
                    reason: "unsupported spawn_version".into(),
                },
                &[],
            )
            .expect("renderer send InitNack");
        });

        let mut settings = HashMap::new();
        settings.insert("scene".to_string(), "/tmp/scene.pkg".to_string());
        let req = SpawnRequest {
            extras: HashMap::new(),
            wp_type: "scene".into(),
            settings,
            test_pattern: false,
            renderer_name: None,
            user_properties_json: None,
        };
        let init = build_init_msg(&req, &def_legacy("wescene-renderer"));
        let err =
            run_init_handshake(&daemon, &init).expect_err("InitNack must abort the handshake");
        let s = err.to_string();
        assert!(
            s.contains("renderer rejected Init"),
            "unexpected error: {s}"
        );
        assert!(
            s.contains("unsupported spawn_version"),
            "unexpected error: {s}"
        );

        peer.join().expect("peer thread");
    }
}

#[cfg(test)]
mod reuse_tests {
    use super::*;
    use crate::plugin::renderer_registry::{
        RendererDef, RendererRegistry, SettingDef, SettingType,
    };
    use std::path::PathBuf;

    fn def_mpv() -> RendererDef {
        let mut ps = HashMap::new();
        ps.insert(
            "loop_file".to_string(),
            SettingDef::new(
                SettingType::String,
                toml::Value::String("inf".into()),
                false,
            ),
        );
        ps.insert(
            "hwdec".to_string(),
            SettingDef::new(
                SettingType::String,
                toml::Value::String("auto".into()),
                false,
            ),
        );
        RendererDef {
            name: "waywallen-mpv".into(),
            plugin_id: "test.plugin".to_string(),
            plugin_version: "0.0.0".to_string(),
            plugin_system: false,
            bin: PathBuf::from("/dev/null"),
            types: vec!["video".into()],
            priority: 100,
            spawn_version: Some(1),
            extras: Vec::new(),
            settings: ps,
            events: Vec::new(),
        }
    }

    /// Construct a live mpv handle stub with the given extras dict.
    /// Mirrors `RendererHandle::test_stub` but lets tests pin extras.
    fn live_mpv_handle(id: &str, extras: HashMap<String, String>) -> Arc<RendererHandle> {
        let (a, _b) = std::os::unix::net::UnixStream::pair().unwrap();
        let (events_tx, _) = tokio::sync::broadcast::channel::<EventMsg>(8);
        Arc::new(RendererHandle {
            id: id.into(),
            wp_type: "video".into(),
            extras,
            name: "waywallen-mpv".into(),
            pid: None,
            gpu: DrmNode::UNKNOWN,
            sock: Arc::new(StdMutex::new(a)),
            events: events_tx,
            bind_snapshot: Arc::new(StdMutex::new(None)),
            sync_fds: Arc::new(StdMutex::new(std::collections::VecDeque::new())),
            latest_frame: Arc::new(StdMutex::new(None)),
            release_syncobj: Arc::new(StdMutex::new(None)),
            format_caps: Arc::new(StdMutex::new(None)),
            last_dispatched_scheme: Arc::new(StdMutex::new(None)),
            frame_record_tx: None,
            pending_configure: Arc::new(StdMutex::new(None)),
            child: Arc::new(TokioMutex::new(None)),
            events_subscribed: Arc::new(Vec::new()),
            clear_rgba: Arc::new(StdMutex::new([0.0, 0.0, 0.0, 1.0])),
        })
    }

    fn req_with_extras(extras: HashMap<String, String>) -> SpawnRequest {
        SpawnRequest {
            extras,
            wp_type: "video".into(),
            settings: HashMap::new(),
            test_pattern: false,
            renderer_name: None,
            user_properties_json: None,
        }
    }

    #[tokio::test]
    async fn find_reusable_hits_when_extras_match() {
        let mut registry = RendererRegistry::new();
        registry.register(def_mpv());
        let mgr = RendererManager::new(registry);

        let mut extras = HashMap::new();
        extras.insert("path".into(), "/clip.mp4".into());
        let h = live_mpv_handle("h1", extras.clone());
        mgr.register_test_handle(h).await;

        let req = req_with_extras(extras);
        let id = mgr.find_reusable(&req).await.expect("reuse hit expected");
        assert_eq!(id, "h1");
    }

    #[tokio::test]
    async fn find_reusable_misses_on_different_path() {
        let mut registry = RendererRegistry::new();
        registry.register(def_mpv());
        let mgr = RendererManager::new(registry);

        let mut h_extras = HashMap::new();
        h_extras.insert("path".into(), "/clip.mp4".into());
        mgr.register_test_handle(live_mpv_handle("h1", h_extras))
            .await;

        let mut req_extras = HashMap::new();
        req_extras.insert("path".into(), "/other.mp4".into());
        let req = req_with_extras(req_extras);
        assert!(
            mgr.find_reusable(&req).await.is_none(),
            "different path must miss reuse",
        );
    }

    #[tokio::test]
    async fn send_setting_changed_writes_wire_and_updates_cache() {
        // Direct end-to-end: wire a socketpair into a RendererHandle and
        // drain the setting_changed control message from the peer side.
        let mut registry = RendererRegistry::new();
        registry.register(def_mpv());
        let mgr = RendererManager::new(registry);

        let (daemon_side, renderer_side) = std::os::unix::net::UnixStream::pair().unwrap();
        daemon_side.set_nonblocking(false).unwrap();
        renderer_side.set_nonblocking(false).unwrap();

        let (events_tx, _) = tokio::sync::broadcast::channel::<EventMsg>(8);
        let h = Arc::new(RendererHandle {
            id: "h1".into(),
            wp_type: "video".into(),
            extras: HashMap::new(),
            name: "waywallen-mpv".into(),
            pid: None,
            gpu: DrmNode::UNKNOWN,
            sock: Arc::new(StdMutex::new(daemon_side)),
            events: events_tx,
            bind_snapshot: Arc::new(StdMutex::new(None)),
            sync_fds: Arc::new(StdMutex::new(std::collections::VecDeque::new())),
            latest_frame: Arc::new(StdMutex::new(None)),
            release_syncobj: Arc::new(StdMutex::new(None)),
            format_caps: Arc::new(StdMutex::new(None)),
            last_dispatched_scheme: Arc::new(StdMutex::new(None)),
            frame_record_tx: None,
            pending_configure: Arc::new(StdMutex::new(None)),
            child: Arc::new(TokioMutex::new(None)),
            events_subscribed: Arc::new(Vec::new()),
            clear_rgba: Arc::new(StdMutex::new([0.0, 0.0, 0.0, 1.0])),
        });
        mgr.register_test_handle(Arc::clone(&h)).await;

        // Renderer-side reader running in a thread to drain the wire.
        let peer = std::thread::spawn(move || {
            let (req, _fds) = crate::ipc::uds::recv_control(&renderer_side).expect("recv");
            req
        });

        mgr.send_setting_changed("h1", vec![("loop_file".into(), "no".into())], None)
            .await
            .expect("send_setting_changed ok");

        let got = peer.join().expect("peer joined");
        match got {
            ControlMsg::SettingChanged { settings } => {
                assert_eq!(settings, vec![("loop_file".into(), "no".into())]);
            }
            other => panic!("expected ApplySettings, got {other:?}"),
        }
    }
}
