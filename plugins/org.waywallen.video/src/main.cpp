// waywallen-video-renderer — Iter 2 GPU YUV→RGB pipeline.
//
// Iter 0/1: sw decode → CPU swscale to RGBA → staging upload of RGBA.
// Iter 2 (this file): sw decode → CPU swscale to NV12 → GPU NV12→RGBA
// via a compute shader (`wavsen::video::YuvToRgba`). NV12 upload is
// 1.5 bytes/pixel vs RGBA's 4, so PCIe bandwidth drops ~60%; the YUV→RGB
// math also moves off the CPU. Iter 4 swaps the sw-decode front end for
// FFmpeg's vulkan hwdevice, after which the pipeline is end-to-end GPU.
//
// IPC plumbing (Init handshake, reader thread, negotiate handoff) is
// unchanged from Iter 0.

#include <atomic>
#include <chrono>
import rstd.cppstd;
import rstd.log;
import wavsen.video;
import wavsen.audio;
import nlohmann.json;

#include <rstd/macro.hpp>

#include <waywallen-bridge/bridge.h>
#include <waywallen-bridge/ipc_v1.h>
#include <waywallen-bridge/pool.h>
#include <waywallen-bridge/probe_vk.h>
#include <waywallen-bridge/resolution.h>

#include <cmath>
#include <errno.h>
#include <signal.h>
#include <string.h>

#include <ctype.h>
#include <stdlib.h>

#include <sys/prctl.h>
#include <sys/socket.h>
#include <unistd.h>

namespace
{

constexpr uint32_t SLOT_COUNT = 3;

struct Options {
    std::string ipc_path;
    std::string video_path;
    std::string render_node; // e.g. "/dev/dri/renderD128"; empty → auto-pick
    std::string hwdec;       // selftest: "auto" | "vulkan" | "vaapi" | "none"
    uint32_t    width { 1920 };
    uint32_t    height { 1080 };
    bool        loop_file { true };
    bool        selftest { false };
};

struct ClearColor {
    float r { 0.0f };
    float g { 0.0f };
    float b { 0.0f };
    float a { 1.0f };
};

constexpr const char* kSchemeColorKey = "waywallen.scheme_color";
constexpr const char* kEnableAudioKey = "waywallen.enable_audio";

[[noreturn]] void die(const std::string& msg) {
    rstd_error("waywallen-video-renderer: {}", msg);
    std::exit(1);
}

float clamp01(float v) {
    if (v < 0.0f) return 0.0f;
    if (v > 1.0f) return 1.0f;
    return v;
}

bool parse_color_wire(const char* raw, ClearColor& out) {
    if (! raw || ! *raw) return false;
    std::string s = raw;
    for (char& ch : s) {
        if (ch == ',') ch = ' ';
    }

    float       values[4] = {};
    int         count     = 0;
    const char* p         = s.c_str();
    while (*p) {
        while (*p && std::isspace(static_cast<unsigned char>(*p))) ++p;
        if (! *p) break;
        if (count >= 4) return false;
        errno     = 0;
        char* end = nullptr;
        float v   = std::strtof(p, &end);
        if (end == p || errno == ERANGE || ! std::isfinite(v)) return false;
        values[count++] = clamp01(v);
        p               = end;
    }
    if (count < 3) return false;
    out = ClearColor {
        .r = values[0],
        .g = values[1],
        .b = values[2],
        .a = count >= 4 ? values[3] : 1.0f,
    };
    return true;
}

bool parse_bool_wire(const char* raw, bool& out) {
    if (! raw) return false;
    std::string s     = raw;
    const auto  first = s.find_first_not_of(" \t\r\n");
    if (first == std::string::npos) return false;
    const auto last = s.find_last_not_of(" \t\r\n");
    s               = s.substr(first, last - first + 1);
    for (char& ch : s) {
        if (ch >= 'A' && ch <= 'Z') ch = static_cast<char>(ch - 'A' + 'a');
    }
    if (s == "true" || s == "1" || s == "yes" || s == "on") {
        out = true;
        return true;
    }
    if (s == "false" || s == "0" || s == "no" || s == "off") {
        out = false;
        return true;
    }
    return false;
}

// SPAWN_VERSION 3: video path arrives via `--path`; everything else
// (loop_file, hwdec, render_node, fps, volume) rides on Init.settings
// kv. Keep `--no-loop` / `--render-node` as standalone-debug escape
// hatches (set them before init; daemon doesn't emit them).
Options parse_args(int argc, char** argv) {
    Options o;
    for (int i = 1; i < argc; ++i) {
        std::string a    = argv[i];
        auto        next = [&]() -> std::string {
            if (i + 1 >= argc) return {};
            return argv[++i];
        };
        if (a == "--ipc")
            o.ipc_path = next();
        else if (a == "--path")
            o.video_path = next();
        else if (a == "--no-loop")
            o.loop_file = false;
        else if (a == "--render-node")
            o.render_node = next();
        else if (a == "--hwdec")
            o.hwdec = next();
        else if (a == "--selftest") {
            o.selftest   = true;
            o.video_path = next();
        }
        // Tolerate other `--key value` extras by skipping the value.
        else if (a.size() >= 2 && a[0] == '-' && a[1] == '-' && i + 1 < argc) {
            std::string nxt = argv[i + 1];
            if (! (nxt.size() >= 2 && nxt[0] == '-' && nxt[1] == '-')) ++i;
        }
    }
    return o;
}

const char* kv_get(const ww_kv_list_t& kv, const char* key) {
    for (uint32_t i = 0; i < kv.count; ++i) {
        if (kv.data[i].key && std::strcmp(kv.data[i].key, key) == 0) return kv.data[i].value;
    }
    return nullptr;
}

struct HostState {
    int               sock { -1 };
    ww_pool_t*        pool { nullptr };
    std::atomic<bool> shutdown { false };
    std::atomic<bool> negotiated { false };
    std::atomic<bool> paused { false };
    std::atomic<bool> muted { false };
    std::atomic<bool> settings_enable_audio { true };
    std::atomic<bool> property_enable_audio { true };

    std::mutex              neg_mu;
    std::condition_variable neg_cv;
    bool                    neg_pending { false };
    ww_pool_directive_t     neg_directive {};
    std::mutex              send_mu;

    std::atomic<bool> loop_pending { false };
    std::atomic<bool> loop_value { true };

    /* hwdec changes are applied at the next file/loop boundary, not
     * mid-stream — store the pending value here. */
    std::mutex  hwdec_mu;
    std::string pending_hwdec;
    bool        hwdec_pending { false };

    /* Audio runtime settings — applied to AvPlayer atomically when
     * pending flag is set; no decoder rebuild. */
    std::atomic<uint32_t> pending_volume { 100 };
    std::atomic<bool>     volume_pending { false };
    std::atomic<bool>     enable_audio_pending { false };
    std::atomic<uint32_t> mute_fade_ms { 0 };
    std::atomic<uint32_t> pause_fade_ms { 0 };
    ClearColor            scheme_color {};
};

bool effective_audio_enabled(const HostState& host) {
    return host.settings_enable_audio.load(std::memory_order_acquire) &&
           host.property_enable_audio.load(std::memory_order_acquire);
}

using Clock = std::chrono::steady_clock;

struct AudioRuntime {
    uint32_t          volume_pct { 100 };
    bool              enabled { true };
    bool              paused { false };
    bool              muted { false };
    bool              device_muted { false };
    bool              pause_pending { false };
    Clock::time_point pause_at {};
    float             target_scale { 1.0f };
};

wavsen::video::HwAccel parse_hwdec(const char* v) {
    if (! v || ! *v) return wavsen::video::HwAccel::Auto;
    if (std::strcmp(v, "vulkan") == 0) return wavsen::video::HwAccel::Vulkan;
    if (std::strcmp(v, "vaapi") == 0) return wavsen::video::HwAccel::Vaapi;
    if (std::strcmp(v, "none") == 0) return wavsen::video::HwAccel::None;
    return wavsen::video::HwAccel::Auto;
}

const char* hwdec_label(wavsen::video::HwAccel h) {
    switch (h) {
    case wavsen::video::HwAccel::Auto: return "auto";
    case wavsen::video::HwAccel::Vulkan: return "vulkan";
    case wavsen::video::HwAccel::Vaapi: return "vaapi";
    case wavsen::video::HwAccel::None: return "none";
    }
    return "?";
}

const char* kind_label(wavsen::video::FrameKind k) {
    switch (k) {
    case wavsen::video::FrameKind::Sw: return "sw";
    case wavsen::video::FrameKind::VulkanShared: return "vulkan-shared";
    case wavsen::video::FrameKind::VaapiDrm: return "vaapi-drm";
    }
    return "?";
}

void signal_shutdown(HostState& s) {
    s.shutdown.store(true, std::memory_order_release);
    s.neg_cv.notify_all();
}

void publish_clear_color(HostState& host, const ClearColor& c) {
    std::lock_guard<std::mutex> send_lk(host.send_mu);
    if (int rc = ww_bridge_send_report_state_clear_color(host.sock, c.r, c.g, c.b, c.a); rc != 0) {
        rstd_warn("waywallen-video-renderer: report_state(clear_color) failed ({})", rc);
    }
}

void set_scheme_color(HostState& host, const char* value, bool publish) {
    ClearColor next {};
    if (value && *value && ! parse_color_wire(value, next)) {
        rstd_warn("waywallen-video-renderer: invalid {} value '{}'; ignoring",
                  static_cast<const char*>(kSchemeColorKey),
                  static_cast<const char*>(value));
        return;
    }
    host.scheme_color = next;
    if (publish) publish_clear_color(host, host.scheme_color);
}

void apply_user_properties(HostState& host, const char* json) {
    if (! json || ! *json) return;
    auto parsed = nlohmann::json::parse(json,
                                        /*cb*/ nullptr,
                                        /*allow_exceptions*/ false,
                                        /*ignore_comments*/ true);
    if (! parsed.is_object()) {
        if (! parsed.is_discarded()) {
            rstd_warn("waywallen-video-renderer: init.user_properties is not an object; ignored");
        }
        return;
    }
    const auto scheme_it = parsed.find(kSchemeColorKey);
    if (scheme_it != parsed.end()) {
        if (! scheme_it->is_string()) {
            rstd_warn("waywallen-video-renderer: {} is not a string; ignored",
                      static_cast<const char*>(kSchemeColorKey));
        } else {
            const auto value = scheme_it->get<std::string>();
            set_scheme_color(host, value.c_str(), false);
        }
    }

    const auto audio_it = parsed.find(kEnableAudioKey);
    if (audio_it == parsed.end()) return;
    bool enabled = true;
    if (audio_it->is_boolean()) {
        enabled = audio_it->get<bool>();
    } else if (audio_it->is_string()) {
        const auto value = audio_it->get<std::string>();
        if (! parse_bool_wire(value.c_str(), enabled)) {
            rstd_warn("waywallen-video-renderer: invalid {} value '{}'; ignoring",
                      static_cast<const char*>(kEnableAudioKey),
                      value.c_str());
            return;
        }
    } else {
        rstd_warn("waywallen-video-renderer: {} is not a bool/string; ignored",
                  static_cast<const char*>(kEnableAudioKey));
        return;
    }
    host.property_enable_audio.store(enabled, std::memory_order_release);
}

void set_property_enable_audio(HostState& host, const char* value) {
    bool enabled = true;
    if (! parse_bool_wire(value, enabled)) {
        rstd_warn("waywallen-video-renderer: invalid {} value '{}'; ignoring",
                  static_cast<const char*>(kEnableAudioKey),
                  static_cast<const char*>(value ? value : ""));
        return;
    }
    host.property_enable_audio.store(enabled, std::memory_order_release);
    host.enable_audio_pending.store(true, std::memory_order_release);
}

void apply_control(HostState& host, ww_bridge_control_t& c) {
    switch (c.op) {
    case WW_EVT_IN_INIT:
        rstd_warn("waywallen-video-renderer: unexpected late Init; ignoring");
        break;
    case WW_EVT_IN_PLAY:
        host.pause_fade_ms.store(c.u.play.fade_ms, std::memory_order_release);
        host.paused.store(false, std::memory_order_release);
        host.neg_cv.notify_all();
        break;
    case WW_EVT_IN_PAUSE:
        host.pause_fade_ms.store(c.u.pause.fade_ms, std::memory_order_release);
        host.paused.store(true, std::memory_order_release);
        host.neg_cv.notify_all();
        break;
    case WW_EVT_IN_UNMUTE:
        host.mute_fade_ms.store(c.u.unmute.fade_ms, std::memory_order_release);
        host.muted.store(false, std::memory_order_release);
        break;
    case WW_EVT_IN_MUTE:
        host.mute_fade_ms.store(c.u.mute.fade_ms, std::memory_order_release);
        host.muted.store(true, std::memory_order_release);
        break;
    case WW_EVT_IN_SET_FPS:
    case WW_EVT_IN_POINTER_MOTION:
    case WW_EVT_IN_POINTER_BUTTON:
    case WW_EVT_IN_POINTER_AXIS: break;
    case WW_EVT_IN_SETTING_CHANGED: {
        ww_bridge_setting_changed_t as {};
        if (ww_bridge_setting_changed_from_control(&c, &as) != 0) break;
        for (uint32_t i = 0; i < as.settings.count; ++i) {
            const char* key = as.settings.data[i].key;
            const char* val = as.settings.data[i].value;
            if (! key || ! val) continue;
            if (std::strcmp(key, "loop_file") == 0) {
                bool enabled = ! (std::strcmp(val, "no") == 0);
                host.loop_value.store(enabled, std::memory_order_release);
                host.loop_pending.store(true, std::memory_order_release);
            } else if (std::strcmp(key, "hwdec") == 0) {
                std::lock_guard<std::mutex> lk(host.hwdec_mu);
                host.pending_hwdec = val;
                host.hwdec_pending = true;
            } else if (std::strcmp(key, "volume") == 0) {
                int n = std::atoi(val);
                if (n < 0) n = 0;
                if (n > 100) n = 100;
                host.pending_volume.store(static_cast<uint32_t>(n), std::memory_order_release);
                host.volume_pending.store(true, std::memory_order_release);
            } else if (std::strcmp(key, "enable_audio") == 0) {
                bool v = true;
                if (! parse_bool_wire(val, v)) {
                    rstd_warn("waywallen-video-renderer: invalid enable_audio value '{}'; ignoring",
                              static_cast<const char*>(val));
                    continue;
                }
                host.settings_enable_audio.store(v, std::memory_order_release);
                host.enable_audio_pending.store(true, std::memory_order_release);
            } else if (std::strcmp(key, kSchemeColorKey) == 0) {
                set_scheme_color(host, val, true);
            } else if (std::strcmp(key, kEnableAudioKey) == 0) {
                set_property_enable_audio(host, val);
            } else {
                rstd_warn("waywallen-video-renderer: ApplySettings: unknown key '{}'; ignoring",
                          static_cast<const char*>(key));
            }
        }
        ww_bridge_setting_changed_free(&as);
        host.neg_cv.notify_all();
        break;
    }
    case WW_EVT_IN_SHUTDOWN: signal_shutdown(host); break;
    case WW_EVT_IN_RELEASE_RESOLVED: {
        const auto& release = c.u.release_resolved;
        int         rc      = release.outcome <= WW_POOL_RELEASE_FORCED
                                  ? ww_bridge_pool_report_release(
                                        host.pool,
                                        release.buffer_generation,
                                        release.buffer_index,
                                        release.release_point,
                                        static_cast<ww_pool_release_outcome_t>(release.outcome))
                                  : -EPROTO;
        if (rc != 0) {
            rstd_warn("waywallen-video-renderer: release_resolved rejected: {}", rc);
            if (rc == -EPIPE) signal_shutdown(host);
        }
        break;
    }
    case WW_EVT_IN_NEGOTIATE_BUFFERS: {
        const auto&         nb = c.u.negotiate_buffers;
        ww_pool_directive_t d {};
        d.category    = nb.path;
        d.mem_source  = nb.mem_source;
        d.fourcc      = nb.fourcc;
        d.modifier    = nb.modifier;
        d.plane_count = nb.plane_count;
        d.sync_mode   = nb.sync_mode;
        d.color       = nb.color;
        d.mem_hint    = nb.mem_hint;
        d.count       = nb.count > 0 ? nb.count : SLOT_COUNT;
        if (d.count > SLOT_COUNT) d.count = SLOT_COUNT;
        {
            std::lock_guard<std::mutex> lk(host.neg_mu);
            host.neg_directive = d;
            host.neg_pending   = true;
        }
        host.neg_cv.notify_all();
        break;
    }
    default:
        rstd_warn("waywallen-video-renderer: unknown control op {}", static_cast<int>(c.op));
        break;
    }
}

void apply_audio_scale(wavsen::audio::AvPlayer* av_player, AudioRuntime& audio, float scale,
                       uint32_t fade_ms, bool force = false) {
    if (! av_player) return;
    scale = clamp01(scale);
    if (! force && audio.target_scale == scale) return;
    audio.target_scale = scale;
    av_player->set_volume_scale(scale, fade_ms);
}

bool set_audio_device_enabled(wavsen::audio::AvPlayer* av_player, AudioRuntime& audio, bool enabled,
                              double seek_seconds, uint32_t volume_pct) {
    if (! av_player) {
        audio.enabled       = enabled;
        audio.pause_pending = false;
        return false;
    }
    if (! enabled) {
        av_player->close_device();
        audio.enabled       = false;
        audio.pause_pending = false;
        audio.device_muted  = false;
        audio.target_scale  = -1.0f;
        return true;
    }
    if (! av_player->is_device_open()) {
        if (seek_seconds >= 0.0 && std::isfinite(seek_seconds)) {
            av_player->seek_to(seek_seconds);
        }
        if (! av_player->open_device()) {
            rstd_warn("waywallen-video-renderer: audio device open failed");
            audio.enabled       = false;
            audio.pause_pending = false;
            audio.device_muted  = false;
            audio.target_scale  = -1.0f;
            return false;
        }
        av_player->set_volume(static_cast<float>(volume_pct) / 100.0f);
        audio.target_scale = -1.0f;
    }
    audio.enabled = true;
    return true;
}

void sync_audio_state(wavsen::audio::AvPlayer* av_player, AudioRuntime& audio, bool paused,
                      bool muted, uint32_t pause_fade_ms, uint32_t mute_fade_ms,
                      Clock::time_point now) {
    if (! av_player) {
        audio.paused        = paused;
        audio.muted         = muted;
        audio.pause_pending = false;
        return;
    }
    if (! audio.enabled) {
        av_player->close_device();
        audio.paused        = paused;
        audio.muted         = muted;
        audio.pause_pending = false;
        audio.device_muted  = false;
        return;
    }

    const bool was_audible   = ! audio.paused && ! audio.muted;
    const bool will_audible  = ! paused && ! muted;
    const bool pause_changed = audio.paused != paused;
    const bool mute_changed  = audio.muted != muted;
    uint32_t   fade_ms       = 0;
    if (was_audible != will_audible) {
        if (pause_changed) {
            fade_ms = pause_fade_ms;
        } else if (mute_changed) {
            fade_ms = mute_fade_ms;
        }
    }

    if (audio.device_muted) {
        av_player->set_muted(false);
        audio.device_muted = false;
    }

    if (! paused && av_player->is_paused()) av_player->play();

    audio.paused = paused;
    audio.muted  = muted;
    apply_audio_scale(av_player, audio, will_audible ? 1.0f : 0.0f, fade_ms);

    if (paused) {
        if (! av_player->is_paused()) {
            if (audio.pause_pending) {
                if (now >= audio.pause_at) {
                    av_player->pause();
                    audio.pause_pending = false;
                }
            } else if (was_audible && fade_ms > 0) {
                audio.pause_pending = true;
                audio.pause_at      = now + std::chrono::milliseconds(fade_ms);
            } else {
                av_player->pause();
                audio.pause_pending = false;
            }
        } else {
            audio.pause_pending = false;
        }
    } else {
        audio.pause_pending = false;
    }
}

// --selftest: open a video, decode one frame, run YuvToRgba against a
// throw-away VkImage we allocate ourselves. Validates that the GPU
// pipeline (device, shader compile/load, descriptor set, queue submit)
// works on this box before relying on the renderer in a real shell. No
// IPC, no daemon — strictly local. Returns 0 on success.
int run_selftest(const Options& opt) {
    if (opt.video_path.empty()) {
        rstd_error("waywallen-video-renderer: --selftest needs a video path");
        return 1;
    }

    uint32_t even_w = opt.width + (opt.width & 1u);
    uint32_t even_h = opt.height + (opt.height & 1u);

    auto producer_res =
        wavsen::video::Producer::create_with_render_node(even_w, even_h, opt.render_node);
    if (producer_res.is_err()) {
        rstd_error("selftest vk: {}", std::move(producer_res).unwrap_err().message);
        return 1;
    }
    auto producer = std::move(producer_res).unwrap();

    auto yuv_res = wavsen::video::YuvToRgba::create(producer->instance(),
                                                    producer->physical_device(),
                                                    producer->device(),
                                                    producer->queue_family_index(),
                                                    producer->queue(),
                                                    even_w,
                                                    even_h);
    if (yuv_res.is_err()) {
        rstd_error("selftest yuv: {}", std::move(yuv_res).unwrap_err().message);
        return 1;
    }
    auto yuv = std::move(yuv_res).unwrap();

    wavsen::video::OpenOpts dec_opts {
        parse_hwdec(opt.hwdec.empty() ? nullptr : opt.hwdec.c_str()),
        opt.render_node,
    };
    auto decoder_res = wavsen::video::VideoDecoder::open_with_vk(
        opt.video_path, even_w, even_h, /*loop=*/false, *producer, dec_opts);
    if (decoder_res.is_err()) {
        rstd_error("selftest decode: {}", std::move(decoder_res).unwrap_err().message);
        return 1;
    }
    auto decoder = std::move(decoder_res).unwrap();
    rstd_info("selftest: hwdec={}, decoder kind={}",
              hwdec_label(dec_opts.hwaccel),
              kind_label(decoder->kind()));

    /* Allocate a private RGBA8 VkImage to convert into. */
    VkImage        dst_img = VK_NULL_HANDLE;
    VkDeviceMemory dst_mem = VK_NULL_HANDLE;
    {
        VkImageCreateInfo ici {};
        ici.sType         = VK_STRUCTURE_TYPE_IMAGE_CREATE_INFO;
        ici.imageType     = VK_IMAGE_TYPE_2D;
        ici.format        = VK_FORMAT_R8G8B8A8_UNORM;
        ici.extent        = { even_w, even_h, 1 };
        ici.mipLevels     = 1;
        ici.arrayLayers   = 1;
        ici.samples       = VK_SAMPLE_COUNT_1_BIT;
        ici.tiling        = VK_IMAGE_TILING_OPTIMAL;
        ici.usage         = VK_IMAGE_USAGE_STORAGE_BIT | VK_IMAGE_USAGE_TRANSFER_SRC_BIT;
        ici.sharingMode   = VK_SHARING_MODE_EXCLUSIVE;
        ici.initialLayout = VK_IMAGE_LAYOUT_UNDEFINED;
        if (vkCreateImage(producer->device(), &ici, nullptr, &dst_img) != VK_SUCCESS) {
            rstd_error("selftest vkCreateImage failed");
            return 1;
        }
        VkMemoryRequirements mr {};
        vkGetImageMemoryRequirements(producer->device(), dst_img, &mr);
        VkPhysicalDeviceMemoryProperties mp {};
        vkGetPhysicalDeviceMemoryProperties(producer->physical_device(), &mp);
        uint32_t type = UINT32_MAX;
        for (uint32_t i = 0; i < mp.memoryTypeCount; ++i) {
            if ((mr.memoryTypeBits & (1u << i)) &&
                (mp.memoryTypes[i].propertyFlags & VK_MEMORY_PROPERTY_DEVICE_LOCAL_BIT)) {
                type = i;
                break;
            }
        }
        if (type == UINT32_MAX) {
            rstd_error("selftest no DEVICE_LOCAL memory");
            return 1;
        }
        VkMemoryAllocateInfo mai {};
        mai.sType           = VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO;
        mai.allocationSize  = mr.size;
        mai.memoryTypeIndex = type;
        if (vkAllocateMemory(producer->device(), &mai, nullptr, &dst_mem) != VK_SUCCESS ||
            vkBindImageMemory(producer->device(), dst_img, dst_mem, 0) != VK_SUCCESS) {
            rstd_error("selftest vkAllocateMemory/Bind failed");
            return 1;
        }
    }

    /* Decode one frame, convert it. */
    int        sync_fd = -1;
    const auto kind    = decoder->kind();
    if (kind == wavsen::video::FrameKind::VulkanShared) {
        wavsen::video::VkFrameView vkv {};
        auto                       fs_res = decoder->next_vk_frame(vkv);
        if (fs_res.is_err()) {
            rstd_error("selftest next_vk_frame: {}", std::move(fs_res).unwrap_err().message);
            return 1;
        }
        if (std::move(fs_res).unwrap() != wavsen::video::NextFrame::Ok) return 1;
        const auto cm = wavsen::video::make_color_matrix(
            static_cast<wavsen::video::ColorSpace>(vkv.colorspace),
            static_cast<wavsen::video::ColorRange>(vkv.color_range));
        wavsen::video::YuvToRgba::VkFrameImports im {};
        im.y_image           = vkv.img[0];
        im.uv_image          = vkv.plane_count > 1 ? vkv.img[1] : VK_NULL_HANDLE;
        im.y_sem             = vkv.sem[0];
        im.uv_sem            = vkv.plane_count > 1 ? vkv.sem[1] : vkv.sem[0];
        im.y_sem_val_in_out  = &vkv.sem_value[0];
        im.uv_sem_val_in_out = vkv.plane_count > 1 ? &vkv.sem_value[1] : &vkv.sem_value[0];
        im.y_layout_in_out   = &vkv.layout[0];
        im.uv_layout_in_out  = vkv.plane_count > 1 ? &vkv.layout[1] : &vkv.layout[0];
        im.y_qf_in_out       = &vkv.queue_family[0];
        im.uv_qf_in_out      = vkv.plane_count > 1 ? &vkv.queue_family[1] : &vkv.queue_family[0];
        im.src_w             = vkv.width;
        im.src_h             = vkv.height;
        im.bit_depth         = vkv.bit_depth;
        auto cv_res          = yuv->convert_av_vk_frame(im, dst_img, even_w, even_h, cm);
        if (cv_res.is_err()) {
            rstd_error("selftest convert: {}", std::move(cv_res).unwrap_err().message);
            sync_fd = -1;
        } else {
            sync_fd = std::move(cv_res).unwrap();
        }
    } else if (kind == wavsen::video::FrameKind::VaapiDrm) {
        wavsen::video::DrmFrameView drmv {};
        auto                        fs_res = decoder->next_drm_frame(drmv);
        if (fs_res.is_err()) {
            rstd_error("selftest next_drm_frame: {}", std::move(fs_res).unwrap_err().message);
            return 1;
        }
        if (std::move(fs_res).unwrap() != wavsen::video::NextFrame::Ok) return 1;
        rstd_info("selftest drm_prime: {}x{}, modifier=0x{:x}, objects={}, layers={}",
                  drmv.width,
                  drmv.height,
                  drmv.objects[0].format_modifier,
                  drmv.object_count,
                  drmv.layer_count);
        const auto cm = wavsen::video::make_color_matrix(
            static_cast<wavsen::video::ColorSpace>(drmv.colorspace),
            static_cast<wavsen::video::ColorRange>(drmv.color_range));
        auto cv_res = yuv->convert_drm_prime(drmv, dst_img, even_w, even_h, cm);
        if (cv_res.is_err()) {
            rstd_error("selftest convert (drm): {}", std::move(cv_res).unwrap_err().message);
            sync_fd = -1;
        } else {
            sync_fd = std::move(cv_res).unwrap();
        }
    } else {
        wavsen::video::Nv12Frame frame;
        auto                     fs_res = decoder->next_frame(frame);
        if (fs_res.is_err()) {
            rstd_error("selftest next_frame: {}", std::move(fs_res).unwrap_err().message);
            return 1;
        }
        if (std::move(fs_res).unwrap() != wavsen::video::NextFrame::Ok) return 1;
        const auto cm = wavsen::video::make_color_matrix(
            static_cast<wavsen::video::ColorSpace>(frame.colorspace),
            static_cast<wavsen::video::ColorRange>(frame.color_range));
        auto cv_res =
            yuv->convert_nv12(dst_img, even_w, even_h, frame.data.data(), frame.data.size(), cm);
        if (cv_res.is_err()) {
            rstd_error("selftest convert: {}", std::move(cv_res).unwrap_err().message);
            sync_fd = -1;
        } else {
            sync_fd = std::move(cv_res).unwrap();
        }
    }

    /* Wait for the conversion to complete via the sync_fd. We can poll
     * the fd with poll(2) since SYNC_FD is a kernel sync_file. */
    if (sync_fd >= 0) {
        ::close(sync_fd);
    }
    vkDeviceWaitIdle(producer->device());
    vkDestroyImage(producer->device(), dst_img, nullptr);
    vkFreeMemory(producer->device(), dst_mem, nullptr);

    if (sync_fd < 0) return 1;
    rstd_info("waywallen-video-renderer: --selftest ok (kind={}, {}x{})",
              kind_label(decoder->kind()),
              even_w,
              even_h);
    return 0;
}

void reader_loop(HostState& host) {
    while (! host.shutdown.load(std::memory_order_acquire)) {
        ww_bridge_control_t msg {};
        int                 rc = ww_bridge_recv_control(host.sock, &msg);
        if (rc != 0) {
            if (! host.shutdown.load(std::memory_order_acquire)) {
                rstd_error("waywallen-video-renderer: recv_control failed: {}", rc);
            }
            signal_shutdown(host);
            return;
        }
        apply_control(host, msg);
        ww_bridge_control_free(&msg);
    }
}

} // namespace

int main(int argc, char** argv) {
    static rstd::log::EnvLogger _logger;
    rstd::log::set_logger(_logger);
    rstd::log::set_max_level(_logger.filter());

    ww_bridge_set_log_callback(
        [](ww_bridge_log_level_t level, const char* msg, void*) {
            constexpr rstd::log::Level kMap[4] = {
                rstd::log::Level::Debug,
                rstd::log::Level::Info,
                rstd::log::Level::Warn,
                rstd::log::Level::Error,
            };
            auto              lvl  = kMap[(unsigned)level <= 3u ? (unsigned)level : 3u];
            auto              args = rstd::fmt::Arguments::make("{}", msg);
            rstd::log::Record rec {
                rstd::log::Metadata { lvl, {} },
                args,
            };
            rstd::log::log(rec);
        },
        nullptr);

    Options opt = parse_args(argc, argv);
    if (opt.selftest) return run_selftest(opt);
    if (opt.ipc_path.empty()) die("--ipc <socket_path> is required");

    ::prctl(PR_SET_PDEATHSIG, SIGTERM);

    HostState host;
    host.sock = ww_bridge_connect(opt.ipc_path.c_str());
    if (host.sock < 0) die("ww_bridge_connect: " + std::string(::strerror(-host.sock)));

    ww_bridge_init_t init {};
    if (int rc = ww_bridge_recv_init(host.sock, &init); rc < 0) {
        const char* reason = (rc == -EPROTO) ? "init: protocol error or unsupported spawn_version"
                                             : "init: recv failed";
        ww_bridge_send_init_nack(
            host.sock, init.spawn_version, WW_BRIDGE_SUPPORTED_SPAWN_VERSION, reason);
        ww_bridge_init_free(&init);
        die(std::string(reason) + " rc=" + std::to_string(rc));
    }
    // Video path arrives via CLI argv `--path` (already in
    // opt.video_path). Init carries only the resolved settings kv.
    if (const char* v = kv_get(init.settings, "loop_file")) {
        opt.loop_file = ! (std::strcmp(v, "no") == 0);
    }
    if (opt.render_node.empty()) {
        if (const char* v = kv_get(init.settings, "render_node"); v && *v) {
            opt.render_node = v;
        }
    }
    wavsen::video::HwAccel hwaccel = wavsen::video::HwAccel::Auto;
    if (const char* v = kv_get(init.settings, "hwdec")) {
        hwaccel = parse_hwdec(v);
    }
    bool     enable_audio = true;
    uint32_t volume_pct   = 100;
    if (const char* v = kv_get(init.settings, "enable_audio")) {
        if (! parse_bool_wire(v, enable_audio)) {
            rstd_warn("waywallen-video-renderer: invalid enable_audio setting '{}'; using true",
                      static_cast<const char*>(v));
            enable_audio = true;
        }
    }
    if (const char* v = kv_get(init.settings, "volume")) {
        int n = std::atoi(v);
        if (n < 0) n = 0;
        if (n > 100) n = 100;
        volume_pct = static_cast<uint32_t>(n);
    }
    host.pending_volume.store(volume_pct, std::memory_order_release);
    host.settings_enable_audio.store(enable_audio, std::memory_order_release);
    int32_t resolution = static_cast<int32_t>(WW_RESOLUTION_ORIGIN);
    if (const char* v = kv_get(init.settings, "resolution"); v && *v) {
        char* end  = nullptr;
        long  n    = std::strtol(v, &end, 10);
        resolution = (end != v) ? ww_resolution_sanitize(static_cast<int32_t>(n))
                                : static_cast<int32_t>(WW_RESOLUTION_1080P);
    }
    apply_user_properties(host, init.user_properties);
    ww_bridge_init_free(&init);
    if (opt.video_path.empty()) die("--path <video-file> is required");

    /* Probe the file's native dimensions. `Producer::create_with_render_node`
     * needs the final size up front, so this has to happen here in
     * main, not inside VideoDecoder. */
    uint32_t native_w = 0, native_h = 0;
    {
        auto probe_res = wavsen::video::VideoDecoder::probe_native(opt.video_path);
        if (probe_res.is_err()) {
            die("probe_native " + opt.video_path + ": " +
                std::move(probe_res).unwrap_err().message);
        }
        auto probe = std::move(probe_res).unwrap();
        native_w   = probe.width;
        native_h   = probe.height;
    }
    opt.width  = native_w;
    opt.height = native_h;
    ww_resolution_apply_cap(resolution, WW_RESOLUTION_CAP_DEFAULT, &opt.width, &opt.height);

    /* NV12 chroma is 4:2:0 → both extents must be even. The decoder
     * rounds up internally too; do it here so all our state agrees. */
    uint32_t even_w = opt.width + (opt.width & 1u);
    uint32_t even_h = opt.height + (opt.height & 1u);

    /* --- Vulkan device first, so the decoder can share it --- */
    auto producer_res =
        wavsen::video::Producer::create_with_render_node(even_w, even_h, opt.render_node);
    if (producer_res.is_err()) {
        die("vk producer: " + std::move(producer_res).unwrap_err().message);
    }
    auto producer = std::move(producer_res).unwrap();

    /* --- Decoder: hwaccel chain per the `hwdec` setting (Auto =
     *   Vulkan → VAAPI → SW). VAAPI takes the render_node path; on any
     *   per-frame mapping failure we fall through to sw via the helper. */
    wavsen::video::OpenOpts dec_opts {
        hwaccel,
        opt.render_node,
    };
    auto decoder_res = wavsen::video::VideoDecoder::open_with_vk(
        opt.video_path, even_w, even_h, opt.loop_file, *producer, dec_opts);
    if (decoder_res.is_err()) {
        die("decode " + opt.video_path + ": " + std::move(decoder_res).unwrap_err().message);
    }
    auto decoder = std::move(decoder_res).unwrap();
    host.loop_value.store(opt.loop_file, std::memory_order_release);
    rstd_info("waywallen-video-renderer: hwdec={}, decoder kind={}",
              hwdec_label(hwaccel),
              kind_label(decoder->kind()));

    /* --- Audio: open same file via PosixFile, attach AvPlayer.
     *   Failure (missing audio stream, unsupported codec, no audio device)
     *   is non-fatal: log and continue without audio (presenter falls
     *   back to wall-clock pacing). */
    std::unique_ptr<wavsen::audio::AvPlayer> av_player;
    {
        auto audio_file_res = wavsen::audio::PosixFile::open(opt.video_path);
        if (audio_file_res.is_err()) {
            rstd_warn("waywallen-video-renderer: audio file open failed");
        } else {
            std::shared_ptr<wavsen::audio::IByteStream> src = std::move(audio_file_res).unwrap();
            auto p_res = wavsen::audio::AvPlayer::open(std::move(src), false);
            if (p_res.is_err()) {
                rstd_warn("waywallen-video-renderer: audio open failed: {}",
                          std::move(p_res).unwrap_err().message);
            } else {
                av_player = std::move(p_res).unwrap();
                av_player->set_volume(volume_pct / 100.0f);
                rstd_info("waywallen-video-renderer: audio decoder attached (volume={}%)",
                          volume_pct);
            }
        }
    }
    AudioRuntime audio_runtime {
        .volume_pct   = volume_pct,
        .enabled      = false,
        .paused       = false,
        .muted        = false,
        .device_muted = false,
    };
    set_audio_device_enabled(
        av_player.get(), audio_runtime, effective_audio_enabled(host), -1.0, volume_pct);

    ww_bridge_vk_dt_t vdt {};
    ww_bridge_vk_dt_load(&vdt, vkGetInstanceProcAddr, producer->instance());
    ww_bridge_vk_log_gpu_info("waywallen-video-renderer", &vdt, producer->physical_device());

    auto yuv_res = wavsen::video::YuvToRgba::create(producer->instance(),
                                                    producer->physical_device(),
                                                    producer->device(),
                                                    producer->queue_family_index(),
                                                    producer->queue(),
                                                    even_w,
                                                    even_h);
    if (yuv_res.is_err()) {
        die("yuv_to_rgba: " + std::move(yuv_res).unwrap_err().message);
    }
    auto yuv = std::move(yuv_res).unwrap();

    /* --- Bridge pool --- */
    ww_pool_vulkan_init_t pool_init {};
    pool_init.instance           = producer->instance();
    pool_init.physical_device    = producer->physical_device();
    pool_init.device             = producer->device();
    pool_init.queue              = producer->queue();
    pool_init.queue_family_index = producer->queue_family_index();
    pool_init.get_instance_proc_addr =
        reinterpret_cast<void* (*)(void*, const char*)>(vkGetInstanceProcAddr);
    pool_init.device_uuid = producer->device_uuid();
    pool_init.driver_uuid = producer->driver_uuid();
    {
        ww_bridge_vk_dt_t dt {};
        ww_bridge_vk_dt_load(&dt, vkGetInstanceProcAddr, producer->instance());
        if (int rc = ww_bridge_vk_query_render_node(&dt,
                                                    producer->physical_device(),
                                                    &pool_init.drm_render_major,
                                                    &pool_init.drm_render_minor);
            rc != 0) {
            rstd_warn("waywallen-video-renderer: drm render-node query failed ({}); "
                      "topology will be unknown to daemon",
                      rc);
        }
    }
    pool_init.drm_render_fd = producer->drm_render_fd();
    /* The bridge's slot VkImage will be the dst of our compute shader's
     * storage-image binding, so it needs STORAGE usage in addition to
     * the default TRANSFER_DST. The modifier filter mirrors the
     * required features. */
    pool_init.image_usage_flags = VK_IMAGE_USAGE_STORAGE_BIT | VK_IMAGE_USAGE_TRANSFER_DST_BIT;
    pool_init.format_feature_flags =
        VK_FORMAT_FEATURE_STORAGE_IMAGE_BIT | VK_FORMAT_FEATURE_TRANSFER_DST_BIT;

    if (int rc = ww_bridge_pool_create(WW_POOL_BACKEND_VULKAN, &pool_init, &host.pool); rc != 0)
        die("ww_bridge_pool_create failed: " + std::to_string(rc));

    if (int rc = ww_bridge_pool_advertise_caps(host.pool,
                                               host.sock,
                                               opt.width,
                                               opt.height,
                                               WW_MEM_HINT_DEVICE_LOCAL | WW_MEM_HINT_HOST_VISIBLE);
        rc != 0)
        die("ww_bridge_pool_advertise_caps failed: " + std::to_string(rc));

    publish_clear_color(host, host.scheme_color);
    rstd_info("waywallen-video-renderer: ready ({}x{}, loop={}, GPU YUV→RGB), "
              "waiting for NegotiateBuffers",
              even_w,
              even_h,
              opt.loop_file ? 1 : 0);

    std::thread reader([&]() {
        reader_loop(host);
    });

    /* Block until first NegotiateBuffers. */
    {
        std::unique_lock<std::mutex> lk(host.neg_mu);
        host.neg_cv.wait(lk, [&] {
            return host.neg_pending || host.shutdown.load(std::memory_order_acquire);
        });
        if (host.neg_pending && ! host.shutdown.load(std::memory_order_acquire)) {
            ww_pool_directive_t d = host.neg_directive;
            host.neg_pending      = false;
            lk.unlock();
            int rc = 0;
            {
                std::lock_guard<std::mutex> send_lk(host.send_mu);
                rc = ww_bridge_pool_apply_directive(host.pool, host.sock, &d);
            }
            if (rc != 0) {
                rstd_error("waywallen-video-renderer: pool_apply_directive (initial) rc={}", rc);
                signal_shutdown(host);
            } else {
                host.negotiated.store(true, std::memory_order_release);
            }
        }
    }

    rstd_info("waywallen-video-renderer: decoder mode = {}", kind_label(decoder->kind()));

    /* --- Main loop ----------------------------------------------------- */
    uint32_t                 slot = 0;
    wavsen::video::Presenter presenter; // Iter 3: PTS-driven pacing.
    if (av_player) {
        presenter.set_external_clock([p = av_player.get()] {
            return p->current_time_seconds();
        });
    }
    wavsen::video::Nv12Frame    frame;
    wavsen::video::VkFrameView  vkv {};
    wavsen::video::DrmFrameView drmv {};
    double                      prev_pts = -1.0; // for loop-boundary detection (PTS regression)
    uint32_t stall_warn_counter          = 0;    // throttle ETIME log spam during backpressure
    bool     submitted_since_negotiate   = false;

    while (! host.shutdown.load(std::memory_order_acquire)) {
        {
            std::unique_lock<std::mutex> lk(host.neg_mu);
            if (host.neg_pending) {
                ww_pool_directive_t d = host.neg_directive;
                host.neg_pending      = false;
                lk.unlock();
                int rc = 0;
                {
                    std::lock_guard<std::mutex> send_lk(host.send_mu);
                    rc = ww_bridge_pool_apply_directive(host.pool, host.sock, &d);
                }
                if (rc != 0) {
                    rstd_error("waywallen-video-renderer: pool_apply_directive (re) rc={}", rc);
                    if (rc > 0) {
                        signal_shutdown(host);
                        break;
                    }
                } else {
                    submitted_since_negotiate = false;
                }
                slot = 0;
            }
        }

        if (host.loop_pending.exchange(false, std::memory_order_acq_rel)) {
            decoder->set_loop(host.loop_value.load(std::memory_order_acquire));
            // Loop toggled — let the presenter re-baseline on next frame.
            presenter.reset();
        }

        /* Audio settings — applied to AvPlayer atomically without rebuild. */
        if (host.volume_pending.exchange(false, std::memory_order_acq_rel)) {
            audio_runtime.volume_pct = host.pending_volume.load(std::memory_order_acquire);
            if (av_player) {
                av_player->set_volume(static_cast<float>(audio_runtime.volume_pct) / 100.0f);
            }
        }
        if (host.enable_audio_pending.exchange(false, std::memory_order_acq_rel)) {
            const bool audio_enabled = effective_audio_enabled(host);
            set_audio_device_enabled(
                av_player.get(), audio_runtime, audio_enabled, prev_pts, audio_runtime.volume_pct);
            presenter.reset();
        }
        const bool paused_now = host.paused.load(std::memory_order_acquire);
        const bool muted_now  = host.muted.load(std::memory_order_acquire);
        sync_audio_state(av_player.get(),
                         audio_runtime,
                         paused_now,
                         muted_now,
                         host.pause_fade_ms.load(std::memory_order_acquire),
                         host.mute_fade_ms.load(std::memory_order_acquire),
                         Clock::now());

        /* hwdec change requested — apply at this loop boundary by
         * tearing down + reopening the decoder. The reopen runs the
         * full hwaccel trial again with the new mode. */
        {
            std::string new_hwdec;
            bool        do_reopen = false;
            {
                std::lock_guard<std::mutex> lk(host.hwdec_mu);
                if (host.hwdec_pending) {
                    new_hwdec          = host.pending_hwdec;
                    host.hwdec_pending = false;
                    do_reopen          = true;
                }
            }
            if (do_reopen) {
                wavsen::video::HwAccel new_h = parse_hwdec(new_hwdec.c_str());
                if (new_h != hwaccel) {
                    rstd_info("waywallen-video-renderer: hwdec change {} → {}, reopening decoder",
                              hwdec_label(hwaccel),
                              hwdec_label(new_h));
                    decoder.reset();
                    wavsen::video::OpenOpts new_opts { new_h, opt.render_node };
                    auto                    re_res = wavsen::video::VideoDecoder::open_with_vk(
                        opt.video_path,
                        even_w,
                        even_h,
                        host.loop_value.load(std::memory_order_acquire),
                        *producer,
                        new_opts);
                    if (re_res.is_err()) {
                        rstd_error("waywallen-video-renderer: reopen failed: {}",
                                   std::move(re_res).unwrap_err().message);
                        signal_shutdown(host);
                        break;
                    }
                    decoder = std::move(re_res).unwrap();
                    hwaccel = new_h;
                    presenter.reset();
                    // Video reopened at PTS 0 — keep audio aligned.
                    if (av_player) av_player->seek_to_start();
                    prev_pts = -1.0;
                    rstd_info("waywallen-video-renderer: reopened, kind={}",
                              kind_label(decoder->kind()));
                }
            }
        }

        if (paused_now && submitted_since_negotiate) {
            std::unique_lock<std::mutex> lk(host.neg_mu);
            auto                         wake = [&] {
                return host.shutdown.load(std::memory_order_acquire) || host.neg_pending ||
                       ! host.paused.load(std::memory_order_acquire);
            };
            if (audio_runtime.pause_pending) {
                host.neg_cv.wait_until(lk, audio_runtime.pause_at, wake);
            } else {
                host.neg_cv.wait(lk, wake);
            }
            continue;
        }

        double                                                       frame_pts = -1.0;
        const auto                                                   fkind     = decoder->kind();
        rstd::Result<wavsen::video::NextFrame, wavsen::video::Error> fs_res =
            rstd::Ok(wavsen::video::NextFrame::Ok);
        switch (fkind) {
        case wavsen::video::FrameKind::VulkanShared: fs_res = decoder->next_vk_frame(vkv); break;
        case wavsen::video::FrameKind::VaapiDrm: fs_res = decoder->next_drm_frame(drmv); break;
        case wavsen::video::FrameKind::Sw: fs_res = decoder->next_frame(frame); break;
        }
        if (fs_res.is_err()) {
            rstd_error("waywallen-video-renderer: decode error (hwdec={}): {}",
                       hwdec_label(hwaccel),
                       std::move(fs_res).unwrap_err().message);
            signal_shutdown(host);
            break;
        }
        const auto fs = std::move(fs_res).unwrap();
        if (fs == wavsen::video::NextFrame::Eof) {
            rstd_info("waywallen-video-renderer: clean EOF (loop=off); idling until shutdown");
            std::unique_lock<std::mutex> lk(host.neg_mu);
            host.neg_cv.wait(lk, [&] {
                return host.shutdown.load(std::memory_order_acquire) || host.neg_pending ||
                       host.loop_pending.load(std::memory_order_acquire);
            });
            continue;
        }
        const bool decoder_looped = fs == wavsen::video::NextFrame::Looped;
        switch (fkind) {
        case wavsen::video::FrameKind::VulkanShared: frame_pts = vkv.pts_seconds; break;
        case wavsen::video::FrameKind::VaapiDrm: frame_pts = drmv.pts_seconds; break;
        case wavsen::video::FrameKind::Sw: frame_pts = frame.pts_seconds; break;
        }

        const bool pts_regressed =
            frame_pts >= 0.0 && prev_pts >= 0.0 && frame_pts + 0.5 < prev_pts;
        if (decoder_looped || pts_regressed) {
            if (av_player) av_player->seek_to_start();
            presenter.reset();
        }
        prev_pts = frame_pts;

        // PTS pacing: sleep until this frame is due. Drop if too late.
        if (! presenter.present_frame(frame_pts)) continue;

        ww_pool_slot_acquire_result_t acquired {};
        int acquire_rc = ww_bridge_pool_acquire_slot_for_render(host.pool, slot, 600, &acquired);
        if (acquire_rc != 0) {
            rstd_error("waywallen-video-renderer: acquire slot contract failed: {}", acquire_rc);
            signal_shutdown(host);
            break;
        }
        if (acquired.status == WW_POOL_SLOT_ACQUIRE_BUSY) {
            if ((stall_warn_counter++ % 30) == 0) {
                rstd_warn("waywallen-video-renderer: slot {} stalled, dropping frame", slot);
            }
            continue;
        }
        stall_warn_counter = 0;
        if (acquired.status == WW_POOL_SLOT_ACQUIRE_FORCED_RELEASE ||
            acquired.status == WW_POOL_SLOT_ACQUIRE_SESSION_LOST ||
            acquired.status == WW_POOL_SLOT_ACQUIRE_ERROR) {
            rstd_error("waywallen-video-renderer: slot {} cannot be reused (status={}, error={})",
                       slot,
                       static_cast<int>(acquired.status),
                       acquired.error_code);
            signal_shutdown(host);
            break;
        }
        const auto& s = acquired.slot;
        if (! s.vk_image) {
            rstd_error("waywallen-video-renderer: slot {} has no VkImage handle", slot);
            signal_shutdown(host);
            break;
        }

        int      sync_fd = -1;
        uint32_t cs_id = 0, cr_id = 0;
        switch (fkind) {
        case wavsen::video::FrameKind::VulkanShared:
            cs_id = vkv.colorspace;
            cr_id = vkv.color_range;
            break;
        case wavsen::video::FrameKind::VaapiDrm:
            cs_id = drmv.colorspace;
            cr_id = drmv.color_range;
            break;
        case wavsen::video::FrameKind::Sw:
            cs_id = frame.colorspace;
            cr_id = frame.color_range;
            break;
        }
        const auto color_matrix =
            wavsen::video::make_color_matrix(static_cast<wavsen::video::ColorSpace>(cs_id),
                                             static_cast<wavsen::video::ColorRange>(cr_id));
        rstd::Result<int, wavsen::video::Error> cv_res = rstd::Ok(-1);
        switch (fkind) {
        case wavsen::video::FrameKind::VulkanShared: {
            wavsen::video::YuvToRgba::VkFrameImports im {};
            im.y_image           = vkv.img[0];
            im.uv_image          = vkv.plane_count > 1 ? vkv.img[1] : VK_NULL_HANDLE;
            im.y_sem             = vkv.sem[0];
            im.uv_sem            = vkv.plane_count > 1 ? vkv.sem[1] : vkv.sem[0];
            im.y_sem_val_in_out  = &vkv.sem_value[0];
            im.uv_sem_val_in_out = vkv.plane_count > 1 ? &vkv.sem_value[1] : &vkv.sem_value[0];
            im.y_layout_in_out   = &vkv.layout[0];
            im.uv_layout_in_out  = vkv.plane_count > 1 ? &vkv.layout[1] : &vkv.layout[0];
            im.y_qf_in_out       = &vkv.queue_family[0];
            im.uv_qf_in_out = vkv.plane_count > 1 ? &vkv.queue_family[1] : &vkv.queue_family[0];
            im.src_w        = vkv.width;
            im.src_h        = vkv.height;
            im.bit_depth    = vkv.bit_depth;
            cv_res          = yuv->convert_av_vk_frame(
                im, reinterpret_cast<VkImage>(s.vk_image), s.width, s.height, color_matrix);
            break;
        }
        case wavsen::video::FrameKind::VaapiDrm:
            cv_res = yuv->convert_drm_prime(
                drmv, reinterpret_cast<VkImage>(s.vk_image), s.width, s.height, color_matrix);
            break;
        case wavsen::video::FrameKind::Sw:
            cv_res = yuv->convert_nv12(reinterpret_cast<VkImage>(s.vk_image),
                                       s.width,
                                       s.height,
                                       frame.data.data(),
                                       frame.data.size(),
                                       color_matrix);
            break;
        }
        if (cv_res.is_err()) {
            rstd_error("waywallen-video-renderer: yuv conversion failed: {}",
                       std::move(cv_res).unwrap_err().message);
            signal_shutdown(host);
            break;
        }
        sync_fd = std::move(cv_res).unwrap();
        std::lock_guard<std::mutex>  send_lk(host.send_mu);
        ww_pool_slot_submit_result_t submitted {};
        int                          submit_rc =
            ww_bridge_pool_submit_slot_for_render(host.pool, host.sock, slot, sync_fd, &submitted);
        if (submit_rc != 0 || submitted.status != WW_POOL_SLOT_SUBMIT_SUBMITTED) {
            rstd_error("waywallen-video-renderer: submit slot failed (rc={}, status={}, error={})",
                       submit_rc,
                       static_cast<int>(submitted.status),
                       submitted.error_code);
            signal_shutdown(host);
            break;
        }
        submitted_since_negotiate = true;

        slot = (slot + 1) % SLOT_COUNT;
    }

    if (reader.joinable()) {
        ::shutdown(host.sock, SHUT_RD);
        reader.join();
    }
    if (host.pool) ww_bridge_pool_destroy(host.pool);
    ww_bridge_close(host.sock);
    return 0;
}
