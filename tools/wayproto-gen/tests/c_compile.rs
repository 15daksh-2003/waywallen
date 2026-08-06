use std::path::PathBuf;
use std::process::Command;

fn gcc_available() -> bool {
    Command::new("gcc")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn protocol_xml() -> PathBuf {
    // tools/wayproto-gen/ → ../../protocol/...
    manifest_dir()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("protocol/waywallen_display_v1.xml")
}

#[test]
fn generated_c_compiles_cleanly() {
    if !gcc_available() {
        eprintln!("skipping: gcc not found on PATH");
        return;
    }
    let xml_path = protocol_xml();
    let xml = std::fs::read_to_string(&xml_path)
        .unwrap_or_else(|e| panic!("read {}: {e}", xml_path.display()));
    let header = wayproto_gen::emit_c_header_from_xml(&xml).expect("codegen header");
    let source = wayproto_gen::emit_c_source_from_xml(&xml).expect("codegen source");

    let tmp = std::env::temp_dir().join(format!("wayproto-gen-c-test-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).unwrap();
    let h_path = tmp.join("ww_proto.h");
    let c_path = tmp.join("ww_proto.c");
    let o_path = tmp.join("ww_proto.o");
    std::fs::write(&h_path, header).unwrap();
    std::fs::write(&c_path, source).unwrap();

    let out = Command::new("gcc")
        .args([
            "-Wall",
            "-Wextra",
            "-Werror",
            "-Wpedantic",
            "-Wconversion",
            "-Wsign-conversion",
            "-std=c11",
            "-I",
        ])
        .arg(&tmp)
        .arg("-c")
        .arg(&c_path)
        .arg("-o")
        .arg(&o_path)
        .output()
        .expect("gcc failed to spawn");

    if !out.status.success() {
        panic!(
            "gcc failed:\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    }

    // Clean up artefacts.
    let _ = std::fs::remove_file(&o_path);
}

#[test]
fn roundtrip_hello_and_bind_buffers() {
    if !gcc_available() {
        eprintln!("skipping: gcc not found on PATH");
        return;
    }
    let xml_path = protocol_xml();
    let xml = std::fs::read_to_string(&xml_path).unwrap();
    let header = wayproto_gen::emit_c_header_from_xml(&xml).expect("codegen header");
    let source = wayproto_gen::emit_c_source_from_xml(&xml).expect("codegen source");

    let tmp = std::env::temp_dir().join(format!("wayproto-gen-c-rt-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).unwrap();
    let h_path = tmp.join("ww_proto.h");
    let c_path = tmp.join("ww_proto.c");
    let rt_c_path = tmp.join("roundtrip.c");
    let bin_path = tmp.join("roundtrip");
    std::fs::write(&h_path, header).unwrap();
    std::fs::write(&c_path, source).unwrap();

    let rt_src = r#"
#define _POSIX_C_SOURCE 200809L
#include "ww_proto.h"
#include <assert.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static void test_hello(void) {
    ww_req_hello_t in = {0};
    in.client_name = strdup("rt-test");
    in.client_version = strdup("0.0.1");
    in.protocol_version = 8;

    ww_buf_t buf;
    ww_buf_init(&buf);
    int rc = ww_req_hello_encode(&in, &buf);
    assert(rc == WW_OK);

    ww_req_hello_t out;
    rc = ww_req_hello_decode(buf.data, buf.len, &out);
    assert(rc == WW_OK);
    assert(strcmp(out.client_name, "rt-test") == 0);
    assert(strcmp(out.client_version, "0.0.1") == 0);
    assert(out.protocol_version == 8);

    ww_req_hello_free(&out);
    ww_req_hello_free(&in);
    ww_buf_free(&buf);
}

static void test_bind_buffers(void) {
    ww_evt_bind_buffers_t in;
    memset(&in, 0, sizeof(in));
    in.buffer_generation = 42;
    in.count = 3;
    in.width = 1920;
    in.height = 1080;
    in.fourcc = 0x34325258; /* XR24 */
    in.modifier = 0x0100000000000001ULL;
    in.planes_per_buffer = 1;

    in.stride.count = 3;
    in.stride.data = calloc(3, sizeof(uint32_t));
    in.stride.data[0] = 7680;
    in.stride.data[1] = 7680;
    in.stride.data[2] = 7680;

    in.plane_offset.count = 3;
    in.plane_offset.data = calloc(3, sizeof(uint32_t));

    in.size.count = 3;
    in.size.data = calloc(3, sizeof(uint64_t));
    for (int i = 0; i < 3; i++) in.size.data[i] = 8294400ULL;

    in.initial_config.generation = 7;
    in.initial_config.buffer_generation = 42;
    in.initial_config.source_rect.w = 1920.f;
    in.initial_config.source_rect.h = 1080.f;
    in.initial_config.dest_rect.x = 10.f;
    in.initial_config.dest_rect.y = 20.f;
    in.initial_config.dest_rect.w = 1900.f;
    in.initial_config.dest_rect.h = 1060.f;
    in.initial_config.clear_color.a = 1.f;

    uint32_t fds = ww_evt_bind_buffers_expected_fds(&in);
    assert(fds == 3);

    ww_buf_t buf;
    ww_buf_init(&buf);
    int rc = ww_evt_bind_buffers_encode(&in, &buf);
    assert(rc == WW_OK);

    ww_evt_bind_buffers_t out;
    rc = ww_evt_bind_buffers_decode(buf.data, buf.len, &out);
    assert(rc == WW_OK);
    assert(out.buffer_generation == 42);
    assert(out.count == 3);
    assert(out.width == 1920);
    assert(out.height == 1080);
    assert(out.fourcc == 0x34325258);
    assert(out.modifier == 0x0100000000000001ULL);
    assert(out.planes_per_buffer == 1);
    assert(out.stride.count == 3 && out.stride.data[0] == 7680);
    assert(out.size.count == 3 && out.size.data[0] == 8294400ULL);
    assert(out.initial_config.generation == 7);
    assert(out.initial_config.buffer_generation == 42);
    assert(out.initial_config.dest_rect.x == 10.f);
    assert(out.initial_config.clear_color.a == 1.f);

    ww_evt_bind_buffers_free(&out);
    ww_evt_bind_buffers_free(&in);
    ww_buf_free(&buf);
}

static void test_register_display_with_owned_struct(void) {
    ww_req_register_display_t in;
    memset(&in, 0, sizeof(in));
    in.name = strdup("DP-1");
    in.instance_id = strdup("connector-DP-1");
    in.metrics.width = 2560;
    in.metrics.height = 1440;
    in.metrics.refresh_mhz = 144000;

    in.consumer_caps.fourccs.count = 1;
    in.consumer_caps.fourccs.data = calloc(1, sizeof(uint32_t));
    in.consumer_caps.fourccs.data[0] = 0x34325258;
    in.consumer_caps.mod_counts.count = 1;
    in.consumer_caps.mod_counts.data = calloc(1, sizeof(uint32_t));
    in.consumer_caps.mod_counts.data[0] = 1;
    in.consumer_caps.modifiers.count = 1;
    in.consumer_caps.modifiers.data = calloc(1, sizeof(uint64_t));
    in.consumer_caps.modifiers.data[0] = 0;
    in.consumer_caps.plane_counts.count = 1;
    in.consumer_caps.plane_counts.data = calloc(1, sizeof(uint32_t));
    in.consumer_caps.plane_counts.data[0] = 1;
    in.consumer_caps.device_uuid.count = 4;
    in.consumer_caps.device_uuid.data = calloc(4, sizeof(uint32_t));
    in.consumer_caps.device_uuid.data[0] = 0x01020304;
    in.consumer_caps.driver_uuid.count = 4;
    in.consumer_caps.driver_uuid.data = calloc(4, sizeof(uint32_t));
    in.consumer_caps.driver_uuid.data[0] = 0x05060708;
    in.consumer_caps.drm_render_major = 226;
    in.consumer_caps.drm_render_minor = 128;
    in.consumer_caps.mem_hints = 3;
    in.consumer_caps.sync_caps = 1;
    in.consumer_caps.color_caps = 1;
    in.consumer_caps.extent_max_w = 7680;
    in.consumer_caps.extent_max_h = 4320;
    in.presentation_caps.flags = 1;
    in.window_state_flags = 8;

    ww_buf_t buf;
    ww_buf_init(&buf);
    assert(ww_req_register_display_encode(&in, &buf) == WW_OK);

    ww_req_register_display_t out;
    assert(ww_req_register_display_decode(buf.data, buf.len, &out) == WW_OK);
    assert(strcmp(out.name, "DP-1") == 0);
    assert(strcmp(out.instance_id, "connector-DP-1") == 0);
    assert(out.metrics.width == 2560);
    assert(out.metrics.refresh_mhz == 144000);
    assert(out.consumer_caps.fourccs.count == 1);
    assert(out.consumer_caps.fourccs.data[0] == 0x34325258);
    assert(out.consumer_caps.modifiers.count == 1);
    assert(out.consumer_caps.modifiers.data[0] == 0);
    assert(out.consumer_caps.device_uuid.count == 4);
    assert(out.consumer_caps.device_uuid.data[0] == 0x01020304);
    assert(out.consumer_caps.drm_render_major == 226);
    assert(out.presentation_caps.flags == 1);
    assert(out.window_state_flags == 8);

    ww_req_register_display_free(&out);
    ww_req_register_display_free(&in);
    ww_buf_free(&buf);
}

static void test_set_composition_config(void) {
    ww_evt_set_composition_config_t in = {0};
    in.config.generation = 7;
    in.config.buffer_generation = 42;
    in.config.source_rect.w = 1920.f;
    in.config.source_rect.h = 1080.f;
    in.config.dest_rect.x = 10.f;
    in.config.dest_rect.y = 20.f;
    in.config.dest_rect.w = 1900.f;
    in.config.dest_rect.h = 1060.f;
    in.config.clear_color.a = 1.f;

    ww_buf_t buf;
    ww_buf_init(&buf);
    assert(ww_evt_set_composition_config_encode(&in, &buf) == WW_OK);

    ww_evt_set_composition_config_t out;
    assert(ww_evt_set_composition_config_decode(buf.data, buf.len, &out) == WW_OK);
    assert(out.config.generation == 7);
    assert(out.config.buffer_generation == 42);
    assert(out.config.source_rect.w == 1920.f);
    assert(out.config.dest_rect.x == 10.f);
    assert(out.config.clear_color.a == 1.f);

    ww_evt_set_composition_config_free(&out);
    ww_buf_free(&buf);
}

static void test_presentation_bool(void) {
    ww_evt_set_presentation_state_t in = {0};
    in.state.generation = 5;
    in.state.config_generation = 3;
    in.state.pause_effect.active = true;

    ww_buf_t buf;
    ww_buf_init(&buf);
    assert(ww_evt_set_presentation_state_encode(&in, &buf) == WW_OK);

    ww_evt_set_presentation_state_t out;
    assert(ww_evt_set_presentation_state_decode(buf.data, buf.len, &out) == WW_OK);
    assert(out.state.generation == 5);
    assert(out.state.config_generation == 3);
    assert(out.state.pause_effect.active);
    ww_evt_set_presentation_state_free(&out);

    assert(buf.len == 20);
    buf.data[16] = 2;
    buf.data[17] = 0;
    buf.data[18] = 0;
    buf.data[19] = 0;
    assert(ww_evt_set_presentation_state_decode(buf.data, buf.len, &out) == WW_ERR_BAD_BOOL);
    ww_buf_free(&buf);
}

static void test_presentation_enum(void) {
    ww_evt_set_presentation_snapshot_t in = {0};
    in.presentation.config.generation = 2;
    in.presentation.config.pause_effect.kind = WAYWALLEN_PAUSE_EFFECT_KIND_BLUR;
    in.presentation.config.pause_effect.blur.radius = 30;
    in.presentation.state.generation = 2;
    in.presentation.state.config_generation = 2;
    in.presentation.state.pause_effect.active = true;

    ww_buf_t buf;
    ww_buf_init(&buf);
    assert(ww_evt_set_presentation_snapshot_encode(&in, &buf) == WW_OK);

    ww_evt_set_presentation_snapshot_t out;
    assert(ww_evt_set_presentation_snapshot_decode(buf.data, buf.len, &out) == WW_OK);
    assert(out.presentation.config.pause_effect.kind == WAYWALLEN_PAUSE_EFFECT_KIND_BLUR);
    ww_evt_set_presentation_snapshot_free(&out);

    buf.data[8] = 7;
    buf.data[9] = 0;
    buf.data[10] = 0;
    buf.data[11] = 0;
    assert(ww_evt_set_presentation_snapshot_decode(buf.data, buf.len, &out) == WW_ERR_BAD_ENUM);
    ww_buf_free(&buf);
}

static void test_buffer_import_failure_enum(void) {
    ww_req_buffer_import_failed_t in = {0};
    in.buffer_generation = 42;
    in.kind = WAYWALLEN_BUFFER_IMPORT_FAILURE_KIND_UNSUPPORTED;
    in.message = strdup("unsupported modifier");
    ww_buf_t buf;
    ww_buf_init(&buf);
    assert(ww_req_buffer_import_failed_encode(&in, &buf) == WW_OK);

    ww_req_buffer_import_failed_t out;
    assert(ww_req_buffer_import_failed_decode(buf.data, buf.len, &out) == WW_OK);
    assert(out.buffer_generation == 42);
    assert(out.kind == WAYWALLEN_BUFFER_IMPORT_FAILURE_KIND_UNSUPPORTED);
    assert(strcmp(out.message, "unsupported modifier") == 0);
    ww_req_buffer_import_failed_free(&out);

    buf.data[8] = 7;
    buf.data[9] = 0;
    buf.data[10] = 0;
    buf.data[11] = 0;
    assert(ww_req_buffer_import_failed_decode(buf.data, buf.len, &out) == WW_ERR_BAD_ENUM);
    ww_req_buffer_import_failed_free(&in);
    ww_buf_free(&buf);
}

int main(void) {
    test_hello();
    test_bind_buffers();
    test_register_display_with_owned_struct();
    test_set_composition_config();
    test_presentation_bool();
    test_presentation_enum();
    test_buffer_import_failure_enum();
    printf("wayproto-gen C roundtrip: OK\n");
    return 0;
}
"#;
    std::fs::write(&rt_c_path, rt_src).unwrap();

    let out = Command::new("gcc")
        .args([
            "-Wall",
            "-Wextra",
            "-Werror",
            "-Wpedantic",
            "-std=c11",
            "-I",
        ])
        .arg(&tmp)
        .arg(&c_path)
        .arg(&rt_c_path)
        .arg("-o")
        .arg(&bin_path)
        .output()
        .expect("gcc spawn");
    if !out.status.success() {
        panic!(
            "gcc failed building roundtrip:\n{}\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    }

    let run = Command::new(&bin_path).output().expect("run roundtrip");
    if !run.status.success() {
        panic!(
            "roundtrip test failed: {}\nstdout: {}\nstderr: {}",
            run.status,
            String::from_utf8_lossy(&run.stdout),
            String::from_utf8_lossy(&run.stderr)
        );
    }
    let stdout = String::from_utf8_lossy(&run.stdout);
    assert!(
        stdout.contains("wayproto-gen C roundtrip: OK"),
        "unexpected stdout: {stdout}"
    );

    // Clean up.
    let _ = std::fs::remove_file(&bin_path);
}
