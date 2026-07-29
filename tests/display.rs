#[path = "common/mod.rs"]
mod common;

mod handshake {
    #[allow(unused_imports)]
    use super::common;
    // End-to-end smoke test for the `waywallen-display-v1` handshake.
    //

    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::time::Duration;

    use waywallen::display::endpoint;
    use waywallen::display::proto::{
        codec, error_code, Event, Request, PROTOCOL_NAME, PROTOCOL_VERSION,
    };
    use waywallen::events::GlobalEvent;
    use waywallen::renderer_manager::RendererManager;
    use waywallen::routing::Router;

    async fn start_display_endpoint(
        sock_name: &str,
    ) -> (
        PathBuf,
        tokio::task::JoinHandle<()>,
        tokio::sync::broadcast::Receiver<GlobalEvent>,
    ) {
        let sock = common::tmp_sock(sock_name);
        let _ = std::fs::remove_file(&sock);

        let mgr = Arc::new(RendererManager::new_default());
        let router = Router::new(Arc::clone(&mgr));
        let sock_for_task = sock.clone();
        let (events_tx, events_rx) = tokio::sync::broadcast::channel(8);
        let server_task = tokio::spawn({
            let router = Arc::clone(&router);
            async move {
                let _ = endpoint::serve(&sock_for_task, router, events_tx).await;
            }
        });

        assert!(
            common::wait_for_sock_bind(&sock, Duration::from_secs(2)).await,
            "display endpoint did not bind {}",
            sock.display()
        );

        (sock, server_task, events_rx)
    }

    fn drive_display_registration(
        sock: &Path,
        client_protocol_version: u32,
    ) -> anyhow::Result<u64> {
        use std::os::unix::net::UnixStream;

        let stream = UnixStream::connect(sock)?;
        codec::send_request(
            &stream,
            &Request::Hello {
                protocol: PROTOCOL_NAME.to_string(),
                client_name: "handshake-test".to_string(),
                client_version: "0.0.1".to_string(),
                client_protocol_version,
            },
            &[],
        )
        .map_err(|e| anyhow::anyhow!("send hello: {e}"))?;

        let (welcome, _fds) =
            codec::recv_event(&stream).map_err(|e| anyhow::anyhow!("recv welcome: {e}"))?;
        match welcome {
            Event::Welcome {
                server_version,
                features,
            } => {
                assert!(
                    server_version.starts_with("waywallen "),
                    "server_version={server_version}"
                );
                assert!(
                    features.iter().any(|s| s == "explicit_sync_fd"),
                    "explicit_sync_fd not in features={features:?}"
                );
            }
            other => panic!("expected welcome, got opcode {}", other.opcode()),
        }

        codec::send_request(
            &stream,
            &Request::RegisterDisplay {
                name: "DP-test".to_string(),
                instance_id: String::new(),
                width: 1920,
                height: 1080,
                refresh_mhz: 60_000,
                drm_render_major: 0,
                drm_render_minor: 0,
                properties: Vec::new(),
            },
            &[],
        )
        .map_err(|e| anyhow::anyhow!("send register_display: {e}"))?;

        let (accepted, _fds) = codec::recv_event(&stream)
            .map_err(|e| anyhow::anyhow!("recv display_accepted: {e}"))?;
        match accepted {
            Event::DisplayAccepted { display_id } => Ok(display_id),
            other => panic!("expected display_accepted, got opcode {}", other.opcode()),
        }
    }

    fn expect_version_reject(sock: &Path, probe: u32) -> anyhow::Result<()> {
        use std::os::unix::net::UnixStream;

        let stream = UnixStream::connect(sock)?;
        codec::send_request(
            &stream,
            &Request::Hello {
                protocol: PROTOCOL_NAME.to_string(),
                client_name: "version-probe".to_string(),
                client_version: "0.0.1".to_string(),
                client_protocol_version: probe,
            },
            &[],
        )
        .map_err(|e| anyhow::anyhow!("send: {e}"))?;

        match codec::recv_event(&stream) {
            Ok((Event::Error { code, message }, _)) => anyhow::ensure!(
                code == error_code::VERSION_UNSUPPORTED,
                "expected VERSION_UNSUPPORTED ({}), got code={code} msg={message:?}",
                error_code::VERSION_UNSUPPORTED,
            ),
            Ok((other, _)) => {
                panic!("expected Error event, got opcode {}", other.opcode())
            }
            Err(e) => panic!("expected Error event, got recv err: {e}"),
        }

        Ok(())
    }

    #[tokio::test]
    async fn handshake_up_to_display_accepted() {
        let (sock, server_task, _events_rx) = start_display_endpoint("display-handshake").await;

        let sock_for_client = sock.clone();
        let client_handle = tokio::task::spawn_blocking(move || {
            drive_display_registration(&sock_for_client, PROTOCOL_VERSION)
        });

        let display_id = client_handle
            .await
            .expect("client join")
            .expect("client flow");
        assert!(display_id >= 1, "display_id={display_id}");

        // Ensure the server still exists (hasn't panicked); then clean up.
        assert!(!server_task.is_finished(), "server task exited prematurely");
        server_task.abort();
        let _ = std::fs::remove_file(&sock);
    }

    #[tokio::test]
    async fn accepts_legacy_protocol_versions() {
        let (sock, server_task, _events_rx) =
            start_display_endpoint("display-legacy-versions").await;

        for version in 6..PROTOCOL_VERSION {
            let sock_for_client = sock.clone();
            let display_id = tokio::task::spawn_blocking(move || {
                drive_display_registration(&sock_for_client, version)
            })
            .await
            .expect("client join")
            .expect("legacy client flow");
            assert!(display_id >= 1, "display_id={display_id}");
        }

        assert!(!server_task.is_finished(), "server task exited prematurely");
        server_task.abort();
        let _ = std::fs::remove_file(&sock);
    }

    #[tokio::test]
    async fn rejects_wrong_protocol_string() {
        let (sock, server_task, _events_rx) = start_display_endpoint("display-bad-proto").await;

        let sock_for_client = sock.clone();
        let got_error = tokio::task::spawn_blocking(move || -> anyhow::Result<bool> {
            use std::os::unix::net::UnixStream;
            let stream = UnixStream::connect(&sock_for_client)?;
            codec::send_request(
                &stream,
                &Request::Hello {
                    protocol: "nope-v0".to_string(),
                    client_name: "bad".to_string(),
                    client_version: "0".to_string(),
                    client_protocol_version: PROTOCOL_VERSION,
                },
                &[],
            )
            .map_err(|e| anyhow::anyhow!("send: {e}"))?;
            // Expect either an Error event or EOF.
            match codec::recv_event(&stream) {
                Ok((Event::Error { .. }, _)) => Ok(true),
                Ok((other, _)) => panic!("unexpected event {:?}", other.opcode()),
                Err(_) => Ok(true), // PeerClosed also acceptable
            }
        })
        .await
        .expect("client join")
        .expect("client flow");

        assert!(got_error, "server must reject bad protocol string");
        server_task.abort();
        let _ = std::fs::remove_file(&sock);
    }

    /// `client_protocol_version` outside the daemon's supported range
    /// must produce `error{code = VERSION_UNSUPPORTED}` followed by close.
    #[tokio::test]
    async fn rejects_unsupported_client_protocol_version() {
        let (sock, server_task, mut events_rx) =
            start_display_endpoint("display-bad-version").await;

        let mut probes = vec![PROTOCOL_VERSION.saturating_add(99)];
        if let Some(low_probe) = endpoint::MIN_SUPPORTED_CLIENT_VERSION.checked_sub(1) {
            probes.push(low_probe);
        }

        for probe in probes {
            let sock_for_client = sock.clone();
            tokio::task::spawn_blocking(move || expect_version_reject(&sock_for_client, probe))
                .await
                .expect("client join")
                .expect("client flow");

            let event = tokio::time::timeout(Duration::from_secs(1), events_rx.recv())
                .await
                .expect("display failure event timeout")
                .expect("display failure event channel closed");
            match event {
                GlobalEvent::DisplayConnectionFailed {
                    client_name,
                    client_protocol_version,
                    error_code: code,
                    reason,
                } => {
                    assert_eq!(client_name, "version-probe");
                    assert_eq!(client_protocol_version, probe);
                    assert_eq!(code, error_code::VERSION_UNSUPPORTED);
                    assert!(reason.contains("not supported"), "reason={reason:?}");
                }
                other => panic!("unexpected global event: {other:?}"),
            }
        }

        server_task.abort();
        let _ = std::fs::remove_file(&sock);
    }
}

mod sync_fd_fanout {
    #[allow(unused_imports)]
    use super::common;
    // Multi-display sync_fd fan-out test: verify that TWO concurrent
    // display clients, each subscribed to the same renderer, both

    use std::os::fd::AsRawFd;
    use std::os::unix::net::UnixStream;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::Duration;

    use waywallen::display::endpoint;
    use waywallen::display::proto::{codec, Event, Request, PROTOCOL_NAME, PROTOCOL_VERSION};
    use waywallen::renderer_manager::{RendererManager, SpawnRequest};
    use waywallen::routing::Router;

    /// Drive a single display client through handshake + N frames.
    /// Returns the count of real `anon_inode:sync_file` fds received.
    fn run_client(sock: &PathBuf, name: &str, n_frames: usize) -> anyhow::Result<usize> {
        let stream = UnixStream::connect(sock)?;
        stream.set_read_timeout(Some(Duration::from_secs(10)))?;

        // hello
        codec::send_request(
            &stream,
            &Request::Hello {
                protocol: PROTOCOL_NAME.to_string(),
                client_name: name.to_string(),
                client_version: "0.0.1".to_string(),
                client_protocol_version: PROTOCOL_VERSION,
            },
            &[],
        )?;
        let (welcome, _) = codec::recv_event(&stream)?;
        anyhow::ensure!(matches!(welcome, Event::Welcome { .. }));

        // register
        codec::send_request(
            &stream,
            &Request::RegisterDisplay {
                name: name.to_string(),
                instance_id: String::new(),
                width: 640,
                height: 480,
                refresh_mhz: 60_000,
                drm_render_major: 0,
                drm_render_minor: 0,
                properties: Vec::new(),
            },
            &[],
        )?;
        let (accepted, _) = codec::recv_event(&stream)?;
        anyhow::ensure!(matches!(accepted, Event::DisplayAccepted { .. }));

        // bind_buffers — the daemon may rebind mid-stream when it promotes
        // the renderer to HOST_VISIBLE, so track the *latest* generation
        let (bind, bind_fds) = codec::recv_event(&stream)?;
        let mut buffer_generation = match bind {
            Event::BindBuffers {
                buffer_generation, ..
            } => buffer_generation,
            _ => anyhow::bail!("{name}: expected bind_buffers"),
        };
        drop(bind_fds);

        // set_config
        let (cfg, _) = codec::recv_event(&stream)?;
        anyhow::ensure!(matches!(cfg, Event::SetConfig { .. }));

        // drain frames
        let mut real_count = 0usize;
        let mut frames = 0usize;
        while frames < n_frames {
            let (evt, fds) = codec::recv_event(&stream)?;
            match evt {
                Event::FrameReady {
                    buffer_generation: g,
                    buffer_index,
                    seq,
                } => {
                    anyhow::ensure!(g == buffer_generation);
                    anyhow::ensure!(fds.len() == 2);
                    let link = std::fs::read_link(format!("/proc/self/fd/{}", fds[0].as_raw_fd()))
                        .unwrap_or_default();
                    if link.to_string_lossy().contains("sync_file") {
                        real_count += 1;
                    }
                    drop(fds);
                    let _ = (g, buffer_index, seq);
                    frames += 1;
                }
                // Unbind/Bind/SetConfig may happen mid-stream when the
                // daemon promotes the renderer to HOST_VISIBLE.
                Event::BindBuffers {
                    buffer_generation: g,
                    ..
                } => {
                    buffer_generation = g;
                }
                Event::SetConfig { .. } | Event::Unbind { .. } => {}
                other => anyhow::bail!("{name}: unexpected {other:?}"),
            }
        }
        codec::send_request(&stream, &Request::Bye, &[])?;
        Ok(real_count)
    }

    #[tokio::test]
    async fn two_displays_both_get_real_sync_fds() {
        if !common::have_vulkan_device() {
            eprintln!("skip: no /dev/dri");
            return;
        }

        let renderer_bin = env!("CARGO_BIN_EXE_waywallen_renderer");
        std::env::set_var("WAYWALLEN_RENDERER_BIN", renderer_bin);

        let mgr = Arc::new(RendererManager::new_default());
        let router = Router::new(Arc::clone(&mgr));
        let sock = common::tmp_sock("sync-fd-fanout");
        let _ = std::fs::remove_file(&sock);

        let sock2 = sock.clone();
        let router2 = Arc::clone(&router);
        let (events_tx, _) = tokio::sync::broadcast::channel(8);
        let server = tokio::spawn(async move {
            let _ = endpoint::serve(&sock2, router2, events_tx).await;
        });

        assert!(
            common::wait_for_sock_bind(&sock, Duration::from_secs(2)).await,
            "display endpoint did not bind"
        );

        let spawn_res = mgr
            .spawn(SpawnRequest {
                wp_type: "scene".into(),
                extras: std::collections::HashMap::new(),
                settings: std::collections::HashMap::new(),
                test_pattern: false,
                renderer_name: None,
                user_properties_json: None,
            })
            .await;
        let renderer_id = match spawn_res {
            Ok(id) => id,
            Err(e) => {
                eprintln!("skip: renderer spawn: {e:#}");
                server.abort();
                let _ = std::fs::remove_file(&sock);
                return;
            }
        };

        if let Some(handle) = mgr.get(&renderer_id).await {
            router.register_renderer(handle).await;
        }

        tokio::time::sleep(Duration::from_millis(500)).await;

        // Spawn two display clients concurrently.
        let sock_a = sock.clone();
        let sock_b = sock.clone();
        let client_a = tokio::task::spawn_blocking(move || run_client(&sock_a, "display-A", 3));
        let client_b = tokio::task::spawn_blocking(move || run_client(&sock_b, "display-B", 3));

        let real_a = client_a.await.expect("A join").expect("A flow");
        let real_b = client_b.await.expect("B join").expect("B flow");

        eprintln!("display-A: {real_a}/3 real sync_files");
        eprintln!("display-B: {real_b}/3 real sync_files");

        // Both must have gotten at least 1 real sync_file (proving the
        // dup fan-out works). In practice we expect 3/3 for each.
        assert!(
            real_a >= 1,
            "display-A got no real sync_files; clone_sync_fd fan-out broken"
        );
        assert!(
            real_b >= 1,
            "display-B got no real sync_files; clone_sync_fd fan-out broken"
        );

        server.abort();
        let _ = std::fs::remove_file(&sock);
    }
}

mod sync_fd_single {
    #[allow(unused_imports)]
    use super::common;
    // End-to-end smoke test: a real Vulkan `waywallen_renderer` subprocess
    // produces real `dma_fence` sync_fds on every `FrameReady`, those fds

    use std::os::fd::AsRawFd;
    use std::sync::Arc;
    use std::time::Duration;

    use waywallen::display::endpoint;
    use waywallen::display::proto::{codec, Event, Request, PROTOCOL_NAME, PROTOCOL_VERSION};
    use waywallen::renderer_manager::{RendererManager, SpawnRequest};
    use waywallen::routing::Router;

    #[tokio::test]
    async fn renderer_produces_real_sync_fds() {
        if !common::have_vulkan_device() {
            eprintln!("skip: no /dev/dri on this host");
            return;
        }

        // Resolve the renderer binary path via cargo's CARGO_BIN_EXE
        // convention so the test doesn't rely on PATH.
        let renderer_bin = env!("CARGO_BIN_EXE_waywallen_renderer");
        std::env::set_var("WAYWALLEN_RENDERER_BIN", renderer_bin);

        // ---- Rig: manager + router + display endpoint ----
        let mgr = Arc::new(RendererManager::new_default());
        let router = Router::new(Arc::clone(&mgr));
        let sock = common::tmp_sock("sync-fd-single");
        let _ = std::fs::remove_file(&sock);

        let sock_for_task = sock.clone();
        let router_for_task = Arc::clone(&router);
        let (events_tx, _) = tokio::sync::broadcast::channel(8);
        let server = tokio::spawn(async move {
            let _ = endpoint::serve(&sock_for_task, router_for_task, events_tx).await;
        });

        assert!(
            common::wait_for_sock_bind(&sock, Duration::from_secs(2)).await,
            "display endpoint did not bind"
        );

        // ---- Spawn a real renderer ----
        let spawn_res = mgr
            .spawn(SpawnRequest {
                wp_type: "scene".into(),
                extras: std::collections::HashMap::new(),
                settings: std::collections::HashMap::new(),
                test_pattern: false,
                renderer_name: None,
                user_properties_json: None,
            })
            .await;
        let renderer_id = match spawn_res {
            Ok(id) => id,
            Err(e) => {
                eprintln!("skip: could not spawn waywallen_renderer: {e:#}");
                server.abort();
                let _ = std::fs::remove_file(&sock);
                return;
            }
        };

        // Wire the renderer into the router — production code does this via
        // `control::apply_entry`; the test rig has to do it explicitly.
        if let Some(handle) = mgr.get(&renderer_id).await {
            router.register_renderer(handle).await;
        }

        // Give the renderer a moment to emit its first BindBuffers.
        tokio::time::sleep(Duration::from_millis(500)).await;

        // ---- Connect a display client and drive the full flow ----
        let sock_for_client = sock.clone();
        let client = tokio::task::spawn_blocking(move || -> anyhow::Result<usize> {
            use std::os::unix::net::UnixStream;
            let stream = UnixStream::connect(&sock_for_client)?;
            stream.set_read_timeout(Some(Duration::from_secs(10)))?;

            // hello / welcome
            codec::send_request(
                &stream,
                &Request::Hello {
                    protocol: PROTOCOL_NAME.to_string(),
                    client_name: "phase3b-e2e".to_string(),
                    client_version: "0.0.1".to_string(),
                    client_protocol_version: PROTOCOL_VERSION,
                },
                &[],
            )?;
            let (welcome, _) = codec::recv_event(&stream)?;
            anyhow::ensure!(
                matches!(welcome, Event::Welcome { .. }),
                "expected welcome, got {welcome:?}"
            );

            // register / accepted
            codec::send_request(
                &stream,
                &Request::RegisterDisplay {
                    name: "e2e-display".to_string(),
                    instance_id: String::new(),
                    width: 640,
                    height: 480,
                    refresh_mhz: 60_000,
                    drm_render_major: 0,
                    drm_render_minor: 0,
                    properties: Vec::new(),
                },
                &[],
            )?;
            let (accepted, _) = codec::recv_event(&stream)?;
            anyhow::ensure!(
                matches!(accepted, Event::DisplayAccepted { .. }),
                "expected display_accepted, got {accepted:?}"
            );

            // bind_buffers carries real dma-buf fds from the renderer.
            // The daemon may rebind mid-stream during promotion.
            let (bind, bind_fds) = codec::recv_event(&stream)?;
            let Event::BindBuffers {
                buffer_generation: initial_gen,
                count,
                planes_per_buffer,
                ..
            } = bind
            else {
                anyhow::bail!("expected bind_buffers");
            };
            let mut buffer_generation = initial_gen;
            let expected_fds = (count * planes_per_buffer) as usize;
            anyhow::ensure!(
                bind_fds.len() == expected_fds,
                "bind_buffers fd count {} != expected {}",
                bind_fds.len(),
                expected_fds
            );
            for (i, fd) in bind_fds.iter().enumerate() {
                // Sanity: must be a valid fd the kernel handed us.
                anyhow::ensure!(fd.as_raw_fd() >= 0, "invalid dma-buf fd #{i}");
            }
            drop(bind_fds);

            // set_config
            let (cfg, _) = codec::recv_event(&stream)?;
            anyhow::ensure!(
                matches!(cfg, Event::SetConfig { .. }),
                "expected set_config"
            );

            // Drain at least 3 frames and verify each carries a live sync fd.
            let mut real_fence_count = 0usize;
            let mut frames_seen = 0usize;
            while frames_seen < 3 {
                let (evt, fds) = codec::recv_event(&stream)?;
                match evt {
                    Event::FrameReady {
                        buffer_generation: g,
                        buffer_index,
                        seq,
                    } => {
                        anyhow::ensure!(
                            g == buffer_generation,
                            "frame_ready gen={g} != bind gen={buffer_generation}"
                        );
                        anyhow::ensure!(
                            fds.len() == 2,
                            "frame_ready expected 2 fds (acquire + release_syncobj), got {}",
                            fds.len()
                        );
                        let acquire_fd = &fds[0];
                        let release_fd = &fds[1];
                        anyhow::ensure!(acquire_fd.as_raw_fd() >= 0, "invalid acquire fd");
                        anyhow::ensure!(release_fd.as_raw_fd() >= 0, "invalid release fd");

                        // Distinguish a real dma_fence sync_file from our
                        // eventfd placeholder by inspecting the proc fd link.
                        let link =
                            std::fs::read_link(format!("/proc/self/fd/{}", acquire_fd.as_raw_fd()))
                                .unwrap_or_default();
                        let link_str = link.to_string_lossy();
                        if link_str.contains("sync_file") {
                            real_fence_count += 1;
                        }
                        eprintln!(
                            "frame #{frames_seen} idx={buffer_index} seq={seq} \
                         acquire_fd={} kind={link_str} release_fd={}",
                            acquire_fd.as_raw_fd(),
                            release_fd.as_raw_fd()
                        );

                        // Release path: v1 dropped the BufferRelease request.
                        // The release_syncobj is signaled by the consumer's
                        drop(fds);
                        let _ = (g, buffer_index, seq);
                        frames_seen += 1;
                    }
                    Event::BindBuffers {
                        buffer_generation: g,
                        ..
                    } => {
                        // The daemon promoted the renderer to HOST_VISIBLE
                        // for cross-GPU; track the new generation.
                        buffer_generation = g;
                    }
                    Event::SetConfig { .. } | Event::Unbind { .. } => {
                        // config update / pre-rebind retire of old gen — fine, drop
                    }
                    other => anyhow::bail!("unexpected event: {other:?}"),
                }
            }

            // Send bye to let the server clean up cleanly.
            codec::send_request(&stream, &Request::Bye, &[])?;
            Ok(real_fence_count)
        });

        let result = client.await.expect("client join");
        let real_fence_count = match result {
            Ok(n) => n,
            Err(e) => {
                eprintln!("client flow failed: {e:#}");
                server.abort();
                let _ = std::fs::remove_file(&sock);
                panic!("Phase 3b e2e failed: {e:#}");
            }
        };

        eprintln!("received {real_fence_count} real dma_fence sync_files out of 3 frames");
        // Acceptance: at least 1 of 3 must be a real sync_file, proving
        // the producer-to-consumer sync_fd path works end-to-end. We do
        assert!(
            real_fence_count >= 1,
            "no real dma_fence sync_files observed; sync_fd path is broken"
        );

        server.abort();
        let _ = std::fs::remove_file(&sock);
    }
}
