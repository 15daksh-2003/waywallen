use anyhow::anyhow;
use std::collections::HashMap;
use std::os::fd::{AsRawFd, OwnedFd, RawFd};
use std::os::unix::net::UnixStream as StdUnixStream;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::display::proto::generated::{self as wire, Rect};
use crate::display::proto::{
    codec, error_code, opcode, Event, Request, PROTOCOL_NAME, PROTOCOL_VERSION,
};
use crate::events::GlobalEvent;
// Display-protocol failures are daemon-internal; this layer talks to
// display consumers over a UDS, not public WS or D-Bus surfaces.
use crate::display::layout::display_point_to_texture;
use crate::error::{Error, Result, ResultExt};
use crate::renderer_manager::{PublishedPool, RendererHandle};
use crate::routing::{
    DisplayConsumptionPermit, DisplayHandle, DisplayOutEvent, DisplayRegistration,
    PresentationDynamicConfig, PresentationSnapshot, Router,
};
use crate::scheduler::ProjectedConfig;
use crate::sync::drm_device;

/// Server version string advertised in `welcome.server_version`.
/// Free-form, informational; consumers do not gate on this.
pub const SERVER_VERSION: &str = concat!("waywallen ", env!("CARGO_PKG_VERSION"));

/// Inclusive range of `client_protocol_version` values this daemon
/// accepts. Unsupported versions are rejected during handshake.
pub const MIN_SUPPORTED_CLIENT_VERSION: u32 = PROTOCOL_VERSION;
pub const MAX_SUPPORTED_CLIENT_VERSION: u32 = PROTOCOL_VERSION;

/// Advertised in `welcome.features`. Advisory in v3+ — clients MUST
/// NOT gate on these; negotiated protocol version is authoritative.
const ADVERTISED_FEATURES: &[&str] = &[
    "explicit_sync_fd",
    "drm_syncobj_release",
    "modifier_negotiation_v1",
];

// ---------------------------------------------------------------------------
// Public entry point

pub fn default_socket_path() -> PathBuf {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    let dir = runtime.join("waywallen");
    let _ = std::fs::create_dir_all(&dir);
    dir.join("display.sock")
}

/// Back-compat 2-arg entry point used by integration tests that
/// don't care about daemon-level shutdown.
pub async fn serve(
    sock_path: &Path,
    router: Arc<Router>,
    events_tx: tokio::sync::broadcast::Sender<GlobalEvent>,
) -> Result<()> {
    // Holding `_never_tx` in scope keeps `wait_for` parked on `Pending`
    // instead of making subscribers observe `RecvError::Closed`.
    let (_never_tx, rx) = tokio::sync::watch::channel(false);
    let res = serve_with_shutdown(sock_path, router, events_tx, rx).await;
    drop(_never_tx);
    res
}

pub async fn serve_with_shutdown(
    sock_path: &Path,
    router: Arc<Router>,
    events_tx: tokio::sync::broadcast::Sender<GlobalEvent>,
    mut shutdown_rx: tokio::sync::watch::Receiver<bool>,
) -> Result<()> {
    let _ = std::fs::remove_file(sock_path);
    if let Some(parent) = sock_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let listener = tokio::net::UnixListener::bind(sock_path)
        .with_context(|| format!("bind display socket at {}", sock_path.display()))?;
    log::info!("display endpoint listening on {}", sock_path.display());

    loop {
        let accepted = tokio::select! {
            biased;
            _ = wait_shutdown(&mut shutdown_rx) => {
                log::info!("display endpoint: shutdown received, ceasing accept");
                return Ok(());
            }
            res = listener.accept() => res,
        };
        let (stream, _addr) = match accepted {
            Ok(x) => x,
            Err(e) => {
                log::warn!("display accept failed: {e}");
                continue;
            }
        };
        let std_stream = match stream
            .into_std()
            .and_then(|s| s.set_nonblocking(false).map(|_| s))
        {
            Ok(s) => s,
            Err(e) => {
                log::warn!("display into_std failed: {e}");
                continue;
            }
        };
        let router = Arc::clone(&router);
        let client_shutdown_rx = shutdown_rx.clone();
        let client_events_tx = events_tx.clone();
        tokio::spawn(async move {
            if let Err(e) =
                handle_client(std_stream, router, client_events_tx, client_shutdown_rx).await
            {
                log::info!("display client closed: {e}");
            }
        });
    }
}

// ---------------------------------------------------------------------------
// Per-client state machine

async fn handle_client(
    stream: StdUnixStream,
    router: Arc<Router>,
    events_tx: tokio::sync::broadcast::Sender<GlobalEvent>,
    mut shutdown_rx: tokio::sync::watch::Receiver<bool>,
) -> Result<()> {
    log::info!("display client connected; performing handshake");
    let registration = do_handshake(&stream, &events_tx, &mut shutdown_rx).await?;
    let DisplayHandle {
        id: display_id,
        session_id,
        presentation,
        rx,
    } = router.register_display(registration).await;
    log::info!("display {display_id} registered with router");

    let send_ack_stream = stream.try_clone().context("clone for accepted")?;
    tokio::task::spawn_blocking(move || {
        codec::send_event(
            &send_ack_stream,
            &Event::DisplayAccepted {
                display_id,
                presentation: presentation_to_wire(presentation),
            },
            &[],
        )
    })
    .await
    .context("accepted join")?
    .map_err(|e| Error::Internal(anyhow!("send display_accepted: {e}")))?;

    let result = run_frame_loop(
        stream,
        router.clone(),
        display_id,
        session_id,
        rx,
        shutdown_rx,
    )
    .await;
    router.unregister_display(display_id).await;
    result
}

// ---------------------------------------------------------------------------
// Handshake

async fn do_handshake(
    stream: &StdUnixStream,
    events_tx: &tokio::sync::broadcast::Sender<GlobalEvent>,
    shutdown_rx: &mut tokio::sync::watch::Receiver<bool>,
) -> Result<DisplayRegistration> {
    let (hello, _fds): (Request, _) = recv_request_cancellable(stream, shutdown_rx)
        .await
        .context("recv hello")?;
    let Request::Hello {
        protocol,
        client_name,
        client_version,
        client_protocol_version,
    } = hello
    else {
        return Err(Error::Internal(anyhow!(
            "expected hello, got opcode {}",
            hello.opcode()
        )));
    };
    if protocol != PROTOCOL_NAME {
        let s = stream.try_clone().context("clone for error")?;
        let msg = format!("unsupported protocol: {protocol:?} (expected {PROTOCOL_NAME:?})");
        let err_msg = msg.clone();
        let _ = tokio::task::spawn_blocking(move || {
            codec::send_event(
                &s,
                &Event::Error {
                    code: error_code::PROTO_NAME_MISMATCH,
                    message: err_msg,
                },
                &[],
            )
        })
        .await;
        let _ = events_tx.send(GlobalEvent::DisplayConnectionFailed {
            client_name: client_name.clone(),
            client_protocol_version,
            error_code: error_code::PROTO_NAME_MISMATCH,
            reason: msg.clone(),
        });
        return Err(Error::Internal(anyhow!("bad protocol string: {msg}")));
    }
    if !(MIN_SUPPORTED_CLIENT_VERSION..=MAX_SUPPORTED_CLIENT_VERSION)
        .contains(&client_protocol_version)
    {
        let s = stream.try_clone().context("clone for error")?;
        let msg = format!(
            "client protocol v{client_protocol_version} not supported; \
             daemon accepts [{MIN_SUPPORTED_CLIENT_VERSION}..={MAX_SUPPORTED_CLIENT_VERSION}]"
        );
        let err_msg = msg.clone();
        let _ = tokio::task::spawn_blocking(move || {
            codec::send_event(
                &s,
                &Event::Error {
                    code: error_code::VERSION_UNSUPPORTED,
                    message: err_msg,
                },
                &[],
            )
        })
        .await;
        let _ = events_tx.send(GlobalEvent::DisplayConnectionFailed {
            client_name: client_name.clone(),
            client_protocol_version,
            error_code: error_code::VERSION_UNSUPPORTED,
            reason: msg.clone(),
        });
        return Err(Error::Internal(anyhow!("version mismatch: {msg}")));
    }
    log::info!("display hello: {client_name} v{client_version} (proto v{client_protocol_version})");

    let welcome_stream = stream.try_clone().context("clone for welcome")?;
    tokio::task::spawn_blocking(move || {
        codec::send_event(
            &welcome_stream,
            &Event::Welcome {
                server_version: SERVER_VERSION.to_string(),
                features: ADVERTISED_FEATURES.iter().map(|s| s.to_string()).collect(),
            },
            &[],
        )
    })
    .await
    .context("welcome join")?
    .map_err(|e| Error::Internal(anyhow!("send welcome: {e}")))?;

    let (reg, _fds): (Request, _) = recv_request_cancellable(stream, shutdown_rx)
        .await
        .context("recv register_display")?;
    let Request::RegisterDisplay {
        name,
        instance_id,
        width,
        height,
        refresh_mhz,
        drm_render_major,
        drm_render_minor,
        properties,
        presentation_caps,
    } = reg
    else {
        return Err(Error::Internal(anyhow!(
            "expected register_display, got opcode {}",
            reg.opcode()
        )));
    };
    let instance_id = if instance_id.is_empty() {
        None
    } else {
        Some(instance_id)
    };
    log::info!(
        "display register: {name} (instance_id={}) {width}x{height}@{refresh_mhz}mHz drm_render={drm_render_major}:{drm_render_minor}",
        instance_id.as_deref().unwrap_or("<none>")
    );
    Ok(DisplayRegistration {
        name,
        instance_id,
        width,
        height,
        refresh_mhz,
        gpu: crate::renderer_manager::DrmNode {
            major: drm_render_major,
            minor: drm_render_minor,
        },
        properties,
        presentation_caps: presentation_caps.flags,
        // consumer_caps arrives ASYNCHRONOUSLY in the frame loop's
        // request handler; registration does not block on it.
        consumer_caps: None,
    })
}

// ---------------------------------------------------------------------------
// Frame loop — translate DisplayOutEvent → wire Event

async fn run_frame_loop(
    stream: StdUnixStream,
    router: Arc<Router>,
    display_id: crate::scheduler::DisplayId,
    display_session_id: crate::sync::DisplaySessionId,
    mut rx: tokio::sync::mpsc::UnboundedReceiver<DisplayOutEvent>,
    mut shutdown_rx: tokio::sync::watch::Receiver<bool>,
) -> Result<()> {
    // Spawn the blocking reader half (client→server requests).
    let read_stream = stream.try_clone().context("clone for reader")?;
    let (req_tx, mut req_rx) =
        tokio::sync::mpsc::unbounded_channel::<codec::CodecResult<Request>>();
    let reader_handle = tokio::task::spawn_blocking(move || loop {
        let res = codec::recv_request(&read_stream);
        let is_err = res.is_err();
        let _ = req_tx.send(res.map(|(r, _fds)| r));
        if is_err {
            return;
        }
    });

    // Most-recently-bound renderer, kept so inbound pointer events can
    // be forwarded without a routing-table walk.
    let mut bound_renderer: Option<Arc<RendererHandle>> = None;
    // Latest SetConfig pushed to this display. Used to inverse-map
    // pointer coords from display pixels into renderer texture pixels.
    let mut latest_config: Option<ProjectedConfig> = None;
    let mut pending_arms = HashMap::<(u64, u64), crate::sync::FrameConsumerArm>::new();
    let mut release_sessions = HashMap::<String, crate::sync::FrameConsumerSession>::new();

    let result = loop {
        tokio::select! {
            _ = wait_shutdown(&mut shutdown_rx) => {
                log::info!("display {display_id}: shutdown signalled");
                break Ok(());
            }
            evt = rx.recv() => match evt {
                None => {
                    log::info!("display {display_id}: router rx closed");
                    break Ok(());
                }
                Some(DisplayOutEvent::Bind { renderer, pool, buffer_generation }) => {
                    bound_renderer = Some(Arc::clone(&renderer));
                    if let Err(e) = send_bind(&stream, &pool, buffer_generation).await {
                        break Err(e);
                    }
                }
                Some(DisplayOutEvent::Unbind { buffer_generation }) => {
                    bound_renderer = None;
                    latest_config = None;
                    if let Err(e) = send_unbind(&stream, buffer_generation).await {
                        break Err(e);
                    }
                }
                Some(DisplayOutEvent::SetConfig(cfg)) => {
                    latest_config = Some(cfg.clone());
                    if let Err(e) = send_set_config(&stream, &cfg).await {
                        break Err(e);
                    }
                }
                Some(DisplayOutEvent::SetPresentationConfig(presentation)) => {
                    if let Err(e) = send_presentation_config(&stream, presentation).await {
                        break Err(e);
                    }
                }
                Some(DisplayOutEvent::SetPresentationDynamicConfig(dynamic_config)) => {
                    if let Err(e) = send_presentation_dynamic_config(&stream, dynamic_config).await {
                        break Err(e);
                    }
                }
                Some(DisplayOutEvent::Frame {
                    renderer, buffer_generation, buffer_index, seq,
                    consumption, member,
                }) => {
                    match forward_frame_ready(
                        &stream, &renderer, buffer_generation, buffer_index, seq,
                        consumption, member, true,
                    ).await {
                        Ok(Some(forwarded)) => {
                            release_sessions
                                .entry(forwarded.session.renderer_id().to_string())
                                .or_insert(forwarded.session);
                            if let Some(arm) = forwarded.arm {
                                pending_arms.insert((buffer_generation, seq), arm);
                            }
                        }
                        Ok(None) => {}
                        Err(e) => break Err(e),
                    }
                }
            },
            maybe_req = req_rx.recv() => match maybe_req {
                Some(Ok(Request::UpdateDisplay { width, height, properties: _ })) => {
                    router.update_display_size(display_id, width, height).await;
                    log::info!("display {display_id}: resized to {width}x{height}");
                }
                Some(Ok(Request::WindowState { flags })) => {
                    log::debug!(
                        "display {display_id}: window_state flags=0x{flags:x}"
                    );
                    router.update_display_window_state(display_id, flags).await;
                }
                Some(Ok(Request::FrameArmed { buffer_generation, seq })) => {
                    match pending_arms.remove(&(buffer_generation, seq)) {
                        Some(arm) => arm.arm(),
                        None => log::warn!(
                            "display {display_id} session {display_session_id}: stale or unknown frame_armed gen={buffer_generation} seq={seq}"
                        ),
                    }
                }
                Some(Ok(Request::ConsumerCaps {
                    fourccs, mod_counts, modifiers, plane_counts,
                    device_uuid, driver_uuid, drm_render_major, drm_render_minor,
                    mem_hints, sync_caps, color_caps, extent_max_w, extent_max_h,
                })) => {
                    let drm = crate::renderer_manager::DrmNode {
                        major: drm_render_major, minor: drm_render_minor,
                    };
                    match crate::dma::negotiate::unflatten_caps(
                        &fourccs, &mod_counts, &modifiers, &plane_counts,
                        &device_uuid, &driver_uuid, drm,
                        sync_caps, color_caps, mem_hints,
                        (extent_max_w, extent_max_h),
                    ) {
                        Ok(caps) => {
                            let prefix = format!("display {display_id}: consumer_caps");
                            log::info!(
                                "{prefix}: imported {} fourcc{}",
                                caps.formats.by_fourcc.len(),
                                if caps.formats.by_fourcc.len() == 1 { "" } else { "s" },
                            );
                            caps.log_dump(&prefix);
                            router.set_consumer_caps(display_id, caps).await;
                        }
                        Err(e) => {
                            log::warn!(
                                "display {display_id}: ConsumerCaps malformed: {e:?}"
                            );
                        }
                    }
                }
                Some(Ok(Request::BindFailed { fourcc, modifier, reason, message })) => {
                    log::warn!(
                        "display {display_id}: BindFailed fourcc=0x{fourcc:08x} \
                         modifier=0x{modifier:x} reason={reason} msg={message:?}"
                    );
                    router.on_consumer_bind_failed(display_id, fourcc, modifier).await;
                }
                Some(Ok(Request::Bye)) => {
                    log::info!("display {display_id}: bye");
                    break Ok(());
                }
                Some(Ok(Request::UnbindDone { buffer_generation })) => {
                    log::debug!(
                        "display {display_id}: unbind_done gen={buffer_generation}"
                    );
                    router.record_unbind_done(display_id, buffer_generation).await;
                }
                Some(Ok(Request::PointerMotion { x, y, timestamp_us, modifiers })) => {
                    if let (Some(r), Some(cfg)) = (bound_renderer.as_ref(), latest_config.as_ref()) {
                        if let Some((tx, ty)) = display_point_to_texture(x, y, cfg) {
                            // RendererManager.send_pointer_motion gates on the
                            // renderer's manifest events list.
                            if let Err(e) = router.renderer_manager()
                                .send_pointer_motion(&r.id, tx, ty, timestamp_us, modifiers).await
                            {
                                log::debug!("display {display_id}: pointer_motion forward failed: {e}");
                            }
                        }
                    }
                }
                Some(Ok(Request::PointerButton { x, y, button, state, timestamp_us, modifiers })) => {
                    if let (Some(r), Some(cfg)) = (bound_renderer.as_ref(), latest_config.as_ref()) {
                        if let Some((tx, ty)) = display_point_to_texture(x, y, cfg) {
                            if let Err(e) = router.renderer_manager()
                                .send_pointer_button(&r.id, tx, ty, button, state, timestamp_us, modifiers).await
                            {
                                log::debug!("display {display_id}: pointer_button forward failed: {e}");
                            }
                        }
                    }
                }
                Some(Ok(Request::PointerAxis { x, y, delta_x, delta_y, source, timestamp_us, modifiers })) => {
                    if let (Some(r), Some(cfg)) = (bound_renderer.as_ref(), latest_config.as_ref()) {
                        if let Some((tx, ty)) = display_point_to_texture(x, y, cfg) {
                            // delta_x/delta_y are scroll quantities, not
                            // spatial; forward unchanged.
                            if let Err(e) = router.renderer_manager()
                                .send_pointer_axis(&r.id, tx, ty, delta_x, delta_y, source, timestamp_us, modifiers).await
                            {
                                log::debug!("display {display_id}: pointer_axis forward failed: {e}");
                            }
                        }
                    }
                }
                Some(Ok(other)) => {
                    log::warn!(
                        "display {display_id}: unexpected request opcode {}",
                        other.opcode()
                    );
                }
                Some(Err(e)) => {
                    log::info!("display {display_id}: client recv error: {e}");
                    break Ok(());
                }
                None => {
                    log::info!("display {display_id}: reader task ended");
                    break Ok(());
                }
            },
        }
    };
    // Force the blocking reader out of its parked `recvmsg`. `shutdown`
    // operates on the socket itself, so all dup'd handles observe it.
    let _ = stream.shutdown(std::net::Shutdown::Both);
    let _ = reader_handle.await;
    pending_arms.clear();
    for (_, session) in release_sessions {
        session.close();
    }
    result
}

/// Run `codec::recv_request` on the blocking pool but tear down the
/// wait if `shutdown_rx` flips to `true`.
async fn recv_request_cancellable(
    stream: &StdUnixStream,
    shutdown_rx: &mut tokio::sync::watch::Receiver<bool>,
) -> Result<(Request, Vec<OwnedFd>)> {
    let blocking_stream = stream.try_clone().context("clone for recv")?;
    let shutdown_stream = stream.try_clone().context("clone for shutdown-kick")?;
    let mut handle = tokio::task::spawn_blocking(move || codec::recv_request(&blocking_stream));
    tokio::select! {
        biased;
        res = &mut handle => match res {
            Ok(r) => r.map_err(|e| Error::Internal(anyhow!("recv: {e}"))),
            Err(e) => Err(Error::Internal(anyhow!("recv join: {e}"))),
        },
        _ = wait_shutdown(shutdown_rx) => {
            let _ = shutdown_stream.shutdown(std::net::Shutdown::Both);
            let _ = handle.await;
            Err(Error::Internal(anyhow!("shutdown during recv")))
        }
    }
}

/// Resolve once the daemon flips the shutdown flag.
async fn wait_shutdown(rx: &mut tokio::sync::watch::Receiver<bool>) {
    let _ = rx.wait_for(|v| *v).await;
}

// ---------------------------------------------------------------------------
// Wire-event senders

fn presentation_to_wire(snapshot: PresentationSnapshot) -> wire::PresentationSnapshot {
    let kind = match snapshot.config.pause_effect.kind {
        crate::settings::PauseEffectKind::None => wire::PauseEffectKind::None,
        crate::settings::PauseEffectKind::Blur => wire::PauseEffectKind::Blur,
    };
    wire::PresentationSnapshot {
        config: wire::PresentationConfig {
            generation: snapshot.config.generation,
            pause_effect: wire::PauseEffectConfig {
                kind,
                blur: wire::BlurEffectConfig {
                    radius: snapshot.config.pause_effect.blur.radius,
                },
            },
        },
        dynamic_config: presentation_dynamic_to_wire(snapshot.dynamic_config),
    }
}

fn presentation_dynamic_to_wire(
    config: PresentationDynamicConfig,
) -> wire::PresentationDynamicConfig {
    wire::PresentationDynamicConfig {
        generation: config.generation,
        config_generation: config.config_generation,
        pause_effect: wire::PauseEffectDynamicConfig {
            active: config.pause_effect.active,
        },
    }
}

async fn send_presentation_config(
    stream: &StdUnixStream,
    presentation: PresentationSnapshot,
) -> Result<()> {
    let evt = Event::SetPresentationConfig {
        presentation: presentation_to_wire(presentation),
    };
    let s = stream
        .try_clone()
        .context("clone for set_presentation_config")?;
    tokio::task::spawn_blocking(move || codec::send_event(&s, &evt, &[]))
        .await
        .context("set_presentation_config join")?
        .map_err(|e| Error::Internal(anyhow!("send set_presentation_config: {e}")))?;
    Ok(())
}

async fn send_presentation_dynamic_config(
    stream: &StdUnixStream,
    dynamic_config: PresentationDynamicConfig,
) -> Result<()> {
    let evt = Event::SetPresentationDynamicConfig {
        dynamic_config: presentation_dynamic_to_wire(dynamic_config),
    };
    let s = stream
        .try_clone()
        .context("clone for set_presentation_dynamic_config")?;
    tokio::task::spawn_blocking(move || codec::send_event(&s, &evt, &[]))
        .await
        .context("set_presentation_dynamic_config join")?
        .map_err(|e| Error::Internal(anyhow!("send set_presentation_dynamic_config: {e}")))?;
    Ok(())
}

async fn send_unbind(stream: &StdUnixStream, buffer_generation: u64) -> Result<()> {
    let evt = Event::Unbind { buffer_generation };
    let s = stream.try_clone().context("clone for unbind")?;
    tokio::task::spawn_blocking(move || codec::send_event(&s, &evt, &[]))
        .await
        .context("unbind join")?
        .map_err(|e| Error::Internal(anyhow!("send unbind: {e}")))?;
    Ok(())
}

async fn send_set_config(stream: &StdUnixStream, cfg: &ProjectedConfig) -> Result<()> {
    let evt = Event::SetConfig {
        config_generation: cfg.config_generation,
        source_rect: Rect {
            x: cfg.source_x,
            y: cfg.source_y,
            w: cfg.source_w,
            h: cfg.source_h,
        },
        dest_rect: Rect {
            x: cfg.dest_x,
            y: cfg.dest_y,
            w: cfg.dest_w,
            h: cfg.dest_h,
        },
        transform: cfg.transform,
        clear_r: cfg.clear_rgba[0],
        clear_g: cfg.clear_rgba[1],
        clear_b: cfg.clear_rgba[2],
        clear_a: cfg.clear_rgba[3],
    };
    let s = stream.try_clone().context("clone for set_config")?;
    tokio::task::spawn_blocking(move || codec::send_event(&s, &evt, &[]))
        .await
        .context("set_config join")?
        .map_err(|e| Error::Internal(anyhow!("send set_config: {e}")))?;
    Ok(())
}

async fn send_bind(
    stream: &StdUnixStream,
    pool: &PublishedPool,
    buffer_generation: u64,
) -> Result<()> {
    let (event, dup_fds) = build_bind_event(pool, buffer_generation)?;
    let s = stream.try_clone().context("clone for bind")?;
    let event_for_send = event.clone();
    let dup_for_send = dup_fds.clone();
    tokio::task::spawn_blocking(move || {
        let result = codec::send_event(&s, &event_for_send, &dup_for_send);
        for fd in dup_for_send {
            unsafe { libc::close(fd) };
        }
        result
    })
    .await
    .context("bind send join")?
    .map_err(|e| Error::Internal(anyhow!("send bind_buffers: {e}")))?;
    Ok(())
}

/// Translate one immutable renderer publication into a `BindBuffers`
/// event. Both schemas use flattened parallel arrays per plane.
fn build_bind_event(pool: &PublishedPool, buffer_generation: u64) -> Result<(Event, Vec<RawFd>)> {
    let count = pool.count;
    let planes_per_buffer = pool.planes_per_buffer;
    let n = (count as usize) * (planes_per_buffer as usize);

    if pool.stride.len() != n
        || pool.plane_offset.len() != n
        || pool.size.len() != n
        || pool.fds.len() != n
    {
        return Err(Error::Internal(anyhow!(
            "PublishedPool parallel arrays inconsistent: count={} planes={} expected={} \
             stride={} offset={} size={} fds={}",
            count,
            planes_per_buffer,
            n,
            pool.stride.len(),
            pool.plane_offset.len(),
            pool.size.len(),
            pool.fds.len()
        )));
    }

    let mut dup_fds: Vec<RawFd> = Vec::with_capacity(n);
    for fd in &pool.fds {
        let raw = nix::unistd::dup(fd.as_raw_fd())
            .map_err(|e| Error::Internal(anyhow!("dup dma-buf fd: {e}")))?;
        dup_fds.push(raw);
    }

    let event = Event::BindBuffers {
        buffer_generation,
        count,
        width: pool.width,
        height: pool.height,
        fourcc: pool.fourcc,
        modifier: pool.modifier,
        planes_per_buffer,
        stride: pool.stride.clone(),
        plane_offset: pool.plane_offset.clone(),
        size: pool.size.clone(),
    };
    log::debug!(
        "display::endpoint: build_bind_event gen={} count={} planes={} {}x{} \
         fourcc=0x{:08x} mod=0x{:016x}",
        buffer_generation,
        count,
        planes_per_buffer,
        pool.width,
        pool.height,
        pool.fourcc,
        pool.modifier,
    );
    for i in 0..n {
        let bi = i / (planes_per_buffer as usize).max(1);
        let pi = i % (planes_per_buffer as usize).max(1);
        log::debug!(
            "  buf[{}].plane[{}] dup_fd={} stride={} plane_offset={} size={}",
            bi,
            pi,
            dup_fds[i],
            pool.stride[i],
            pool.plane_offset[i],
            pool.size[i],
        );
    }
    Ok((event, dup_fds))
}

// ---------------------------------------------------------------------------
// Frame forwarding (with sync fence)

fn acquire_sync_fd(renderer: &Arc<RendererHandle>, seq: u64) -> Result<OwnedFd> {
    renderer.clone_sync_fd(seq).ok_or_else(|| {
        Error::Internal(anyhow!(
            "acquire sync_fd for seq={seq} missing (evicted or never arrived)"
        ))
    })
}

async fn forward_frame_ready(
    stream: &StdUnixStream,
    renderer: &Arc<RendererHandle>,
    buffer_generation: u64,
    buffer_index: u32,
    seq: u64,
    consumption: DisplayConsumptionPermit,
    member: Option<crate::sync::FrameConsumerMember>,
    requires_arm: bool,
) -> Result<Option<ForwardedFrame>> {
    if !consumption.is_current() {
        if let Some(member) = member {
            member.skip();
        }
        return Ok(None);
    }
    let fence = acquire_sync_fd(renderer, seq)?;
    // Allocate a fresh BINARY drm_syncobj for this consumer and frame.
    // The handle stays in the daemon and is handed off to the reaper.
    let dev = drm_device().context("open DRM render node for release_syncobj")?;
    let consumer_handle = dev
        .create_binary_syncobj()
        .context("create binary release_syncobj")?;
    let release_fd = dev
        .handle_to_fd(&consumer_handle)
        .context("export release_syncobj fd")?;

    let fence_raw = fence.as_raw_fd();
    let release_raw = release_fd.as_raw_fd();
    let send_stream = stream.try_clone().context("clone for frame_ready")?;
    let evt = Event::FrameReady {
        buffer_generation,
        buffer_index,
        seq,
    };
    let send_result = tokio::task::spawn_blocking(move || {
        codec::send_event(&send_stream, &evt, &[fence_raw, release_raw])
    })
    .await
    .context("frame_ready send join")?;
    drop(fence);
    drop(release_fd);
    send_result.map_err(|e| Error::Internal(anyhow!("send frame_ready: {e}")))?;

    let forwarded = member.map(|member| {
        let session = member.session();
        let arm = member.delivered(consumer_handle, requires_arm);
        ForwardedFrame { arm, session }
    });
    Ok(forwarded)
}

struct ForwardedFrame {
    arm: Option<crate::sync::FrameConsumerArm>,
    session: crate::sync::FrameConsumerSession,
}

// ---------------------------------------------------------------------------
// Tests

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_bind_event_uses_display_generation() {
        use nix::sys::memfd::{memfd_create, MemFdCreateFlag};
        use std::ffi::CString;
        use std::os::fd::FromRawFd;
        let name = CString::new("waywallen-display-endpoint-test").unwrap();
        let fd1 = memfd_create(&name, MemFdCreateFlag::MFD_CLOEXEC).unwrap();
        let fd2 = memfd_create(&name, MemFdCreateFlag::MFD_CLOEXEC).unwrap();

        let pool = PublishedPool {
            generation: 7,
            flags: 0,
            count: 2,
            fourcc: 0x34325258,
            width: 800,
            height: 600,
            modifier: 0,
            planes_per_buffer: 1,
            stride: vec![3200, 3200],
            plane_offset: vec![0, 0],
            size: vec![1_920_000, 1_920_000],
            fds: vec![fd1, fd2],
        };

        let (event, dup_fds) = build_bind_event(&pool, 11).unwrap();
        assert_eq!(dup_fds.len(), 2);
        match event {
            Event::BindBuffers {
                buffer_generation,
                count,
                width,
                height,
                fourcc,
                modifier,
                planes_per_buffer,
                stride,
                plane_offset,
                size,
            } => {
                assert_eq!(buffer_generation, 11);
                assert_eq!(count, 2);
                assert_eq!(width, 800);
                assert_eq!(height, 600);
                assert_eq!(fourcc, 0x34325258);
                assert_eq!(modifier, 0);
                assert_eq!(planes_per_buffer, 1);
                assert_eq!(stride, vec![3200, 3200]);
                assert_eq!(plane_offset, vec![0, 0]);
                assert_eq!(size, vec![1_920_000, 1_920_000]);
            }
            _ => panic!("expected BindBuffers"),
        }
        for raw in dup_fds {
            let _ = unsafe { std::fs::File::from_raw_fd(raw) };
        }
    }
}

#[allow(dead_code)]
const _OPCODE_MOD_KEEPALIVE: fn() = || {
    let _ = opcode::request::HELLO;
};
