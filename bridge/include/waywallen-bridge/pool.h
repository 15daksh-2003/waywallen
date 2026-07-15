// waywallen-bridge — buffer pool + path-explicit modifier negotiation.

#ifndef WAYWALLEN_BRIDGE_POOL_H
#define WAYWALLEN_BRIDGE_POOL_H

#include <waywallen-bridge/ipc_v1.h>
#include <waywallen-bridge/protocol_bits.h>

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* -----------------------------------------------------------------------
 * Path / memory classification — wire-mirrored to ipc-v3
 * `ww_evt_in_negotiate_buffers_t.{path, mem_source}` and
 * `negotiate.rs::PathCategory` / `MemSource`.
 * ----------------------------------------------------------------------- */

typedef enum ww_path_category
{
    /* Both peers on the same physical GPU (UUIDs match, or DRM render
     * node major:minor matches when UUIDs are unknown). Use the
     * tile / vendor modifier the daemon picked. Modifier in the
     * directive is authoritative — bridge MUST pass it through to the
     * allocator. */
    WW_PATH_OPTIMIZED_SAME_DEVICE = 0,

    /* Same vendor, different device (e.g. dual-AMD). Iter 1 falls
     * through to COMPAT_LINEAR; reserved for the Iter-2 vendor-tier
     * table. */
    WW_PATH_OPTIMIZED_SAME_VENDOR = 1,

    /* Cross-vendor, or any case where same-device tiling cannot be
     * proven. Bridge forces LINEAR modifier and selects the source
     * named by `mem_source`. No driver-specific tiling is used. */
    WW_PATH_COMPAT_LINEAR = 2,

    /* Reserved for Iter 3+: render to GPU memory, copy back to system
     * RAM, ship pixels through a separate IPC channel. The daemon
     * never emits this category in Iter 1; bridge MUST treat it as
     * "fail negotiation" until the channel is wired. */
    WW_PATH_COMPAT_CPU_READBACK = 3,
} ww_path_category_t;

typedef enum ww_mem_source
{
    /* GBM_BO_USE_RENDERING / Vulkan DEVICE_LOCAL exportable. The
     * default for OPTIMIZED paths. */
    WW_MEM_SRC_GPU_NATIVE = 0,

    /* GBM_BO_USE_LINEAR / Vulkan LINEAR-tiled exportable.
     *
     * EGL/GBM: GBM_BO_USE_LINEAR lands in GTT/sysmem on every Mesa
     * driver — PRIME-importable across GPUs by construction.
     *
     * Vulkan: the bridge MUST pick a HOST_VISIBLE memory type for this
     * source (see `pool_vulkan.c::pick_memory_type`). On the NVIDIA
     * proprietary driver, the LINEAR-tiling image's default memtype
     * is DEVICE_LOCAL VRAM, which a different-vendor GPU cannot
     * reference in a CS submit ("Not enough memory for command
     * submission" on amdgpu, then device-lost). See cross_gpu.md. */
    WW_MEM_SRC_GPU_LINEAR = 1,

    /* /dev/dma_heap/system. Reserved for the case where neither GBM
     * nor Vulkan can produce a cross-GPU-importable LINEAR buffer.
     * Iter 1: not implemented; daemon will not select. */
    WW_MEM_SRC_DMABUF_HEAP = 2,
} ww_mem_source_t;

/* Daemon → bridge: full description of the slot pool to allocate. */
typedef struct ww_pool_directive {
    uint32_t category;   /* ww_path_category_t */
    uint32_t mem_source; /* ww_mem_source_t */
    uint32_t fourcc;
    uint64_t modifier;
    uint32_t plane_count;
    uint32_t sync_mode; /* exactly one WW_SYNC_* bit */
    uint32_t color;     /* WW_COLOR_* packed */
    uint32_t mem_hint;  /* unused by bridge in Iter 1; advisory */
    uint32_t width;
    uint32_t height;
    uint32_t count; /* pool size; >=1 */
} ww_pool_directive_t;

/* -----------------------------------------------------------------------
 * Backend selection
 * ----------------------------------------------------------------------- */

typedef enum ww_pool_backend
{
    WW_POOL_BACKEND_EGL_GBM = 0, /* mpv plugin */
    WW_POOL_BACKEND_VULKAN  = 1, /* image plugin */
} ww_pool_backend_t;

/* EGL/GBM backend init descriptor. Plugin owns EGLDisplay + EGL
 * context — bridge only borrows them via this struct and the
 * eglGetProcAddress callback. The DRM render-node fd is *moved*
 * into the bridge: bridge wraps it in a gbm_device and closes both
 * on pool destroy.
 *
 * `egl_display` is `EGLDisplay` (an opaque handle on every Mesa
 * platform); cast in/out at the call site to keep this header
 * EGL-include-free. */
typedef struct ww_pool_egl_gbm_init {
    void* egl_display;                           /* EGLDisplay */
    int   drm_render_fd;                         /* moved */
    void* (*get_proc_address)(const char* name); /* eglGetProcAddress */
    /* DRM render major/minor as reported by EGL_DRM_RENDER_NODE_FILE_EXT
     * → stat(); used for the `ready` event sent by the bridge. Pass
     * (0,0) if unknown. */
    uint32_t drm_render_major;
    uint32_t drm_render_minor;
} ww_pool_egl_gbm_init_t;

/* Vulkan backend init descriptor. Plugin owns the VkInstance, the
 * VkPhysicalDevice it picked, and the VkDevice + queue. Bridge
 * borrows them and creates VkImage / VkDeviceMemory / VkSemaphore
 * objects on top.
 *
 * `instance` / `physical_device` / `device` are `VkInstance` /
 * `VkPhysicalDevice` / `VkDevice` (opaque handles); cast at the
 * call site to keep this header Vulkan-include-free.
 *
 * `device_uuid` and `driver_uuid` are 16-byte buffers from
 * `VkPhysicalDeviceIDProperties`; bridge ships them in `format_caps`.
 * Pass NULL on either to send 16 zero bytes. */
typedef struct ww_pool_vulkan_init {
    void*    instance;
    void*    physical_device;
    void*    device;
    uint32_t queue_family_index;
    void*    queue; /* VkQueue used for transfers */
    void* (*get_instance_proc_addr)(void* instance, const char* name);
    const uint8_t* device_uuid; /* NULL or 16 bytes */
    const uint8_t* driver_uuid; /* NULL or 16 bytes */
    /* Advisory fallback for the DRM render-node identity advertised
     * to the daemon. Bridge always tries VK_EXT_physical_device_drm
     * on the supplied physical_device first; these fields are used
     * only when the extension isn't available (older drivers).
     * Pass 0/0 to skip the fallback — empty values cause the daemon
     * to treat topology as unknown for this producer. */
    uint32_t drm_render_major;
    uint32_t drm_render_minor;
    /* Optional render-node fd for drm_syncobj timeline export. If -1,
     * bridge opens `/dev/dri/renderD<minor>` for the queried minor
     * (or falls back to first-openable when the minor isn't queryable).
     * Pass an existing fd to make bridge dup it — useful when the
     * producer already keeps the render node open and wants to share
     * its kernel object table with bridge. */
    int drm_render_fd;
    /* VkImageUsageFlags for VkImageCreateInfo.usage at slot allocation
     * time. */
    uint32_t image_usage_flags;
    /* VkFormatFeatureFlags the negotiated modifier's
     * drmFormatModifierTilingFeatures must cover.
     * Pass 0 for the TRANSFER_DST_BIT default. */
    uint32_t format_feature_flags;
} ww_pool_vulkan_init_t;

/* -----------------------------------------------------------------------
 * Opaque pool object
 * ----------------------------------------------------------------------- */

typedef struct ww_pool ww_pool_t;

/* Create a pool with the chosen backend. `init_data` points at an
 * `ww_pool_egl_gbm_init_t` or `ww_pool_vulkan_init_t` matching the
 * backend selector. The pool DOES NOT yet have any slots — caller
 * must follow with `advertise_caps` then react to `negotiate_buffers`
 * (which reaches `apply_directive`).
 *
 * On success: `*out_pool` non-NULL, returns 0. On failure returns a
 * negative errno; `*out_pool` is left NULL.
 *
 * Bridge takes ownership of:
 *   - `init_data->drm_render_fd` (will be close()'d in destroy).
 *   - All allocations the bridge makes inside the pool.
 *
 * Bridge does NOT take ownership of:
 *   - `egl_display`, `instance`, `physical_device`, `device`, `queue`
 *     (caller's lifetime). */
int ww_bridge_pool_create(ww_pool_backend_t backend, const void* init_data, ww_pool_t** out_pool);

/* Destroy a pool. Tears down all slot resources, closes the drm_fd,
 * destroys the GBM device or Vulkan-side bridge objects, and frees
 * the pool. Safe on NULL. */
void ww_bridge_pool_destroy(ww_pool_t* pool);

/* Probe the producer's per-fourcc modifier capabilities, encode them
 * as `format_caps`, and send on `sock`. Sends `ready` first if not
 * already sent and `release_syncobj` (timeline) right after. After
 * this call the producer is fully advertised and the daemon may
 * dispatch `negotiate_buffers` at any time.
 *
 * `width` / `height` are the pixel extent the renderer wants to
 * allocate at; bridge probes each candidate modifier with this size.
 *
 * `mem_hints` is the producer's advertised mem-hint set. Bridge ORs
 * `WW_MEM_HINT_LINEAR_ONLY` into it automatically when the modifier
 * probe finds zero modifier-aware (fourcc, modifier) combinations on
 * the device — that signals the daemon to pick `COMPAT_LINEAR`
 * unconditionally without trying any tile modifiers first. */
int ww_bridge_pool_advertise_caps(ww_pool_t* pool, int sock, uint32_t width, uint32_t height,
                                  uint32_t mem_hints);

/* Apply a directive received via `WW_EVT_IN_NEGOTIATE_BUFFERS`. Internal
 * sequence:
 *
 *   1) Validate directive (path/mem_source range, modifier in
 *      advertised set when path is OPTIMIZED_*).
 *   2) Tear down existing slots if any.
 *   3) Allocate the first slot as a dry run.
 *   4) On dry-run failure: emit `bind_failed{reason=import_failed}`
 *      on `sock` and return a negative errno. Caller should NOT
 *      shutdown — the daemon will re-pick.
 *   5) Allocate remaining `directive->count - 1` slots.
 *   6) Emit `bind_buffers` carrying the dmabuf fds.
 *
 * Returns 0 on success, negative on dry-run failure (bind_failed
 * already sent), positive on protocol/system error (caller should
 * shut down). */
int ww_bridge_pool_apply_directive(ww_pool_t* pool, int sock, const ww_pool_directive_t* directive);

/* Per-slot resource view returned from `acquire_slot`. Plugin renders
 * into the backend-specific handle:
 *   - EGL/GBM: bind `gl_export_fbo`, draw, glFlush, hand back.
 *   - Vulkan:  record commands targeting `vk_image`, submit, hand back.
 *
 * The slot remains owned by the bridge throughout — the plugin only
 * writes into the exposed handle. Slot index roundtrips back through
 * `submit_slot`. */
typedef struct ww_pool_slot {
    uint32_t index;
    /* EGL/GBM backend: bind these to draw. Both 0 on the Vulkan
     * backend. */
    uint32_t gl_export_fbo;
    uint32_t gl_export_texture;
    /* Vulkan backend: render into this image. Both NULL on the
     * EGL/GBM backend. */
    void* vk_image;
    void* vk_memory;
    /* Layout (informational; same across slots within one directive).
     * Plugin doesn't usually need these — bridge already filled
     * `bind_buffers` — but they're handy when the upload path needs
     * the stride (image plugin's vkCmdCopyBufferToImage). */
    uint32_t width;
    uint32_t height;
    uint32_t stride;
    uint32_t plane_offset;
    uint32_t size;
} ww_pool_slot_t;

typedef enum ww_pool_slot_acquire_status
{
    /* The slot has never been submitted in this pool generation. */
    WW_POOL_SLOT_ACQUIRE_READY_UNUSED = 0,
    /* The daemon release timeline proves the previous use completed. */
    WW_POOL_SLOT_ACQUIRE_READY_RELEASED = 1,
    /* The previous use did not complete before the requested deadline. */
    WW_POOL_SLOT_ACQUIRE_BUSY = 2,
    /* Pool state, wait, or backend snapshot failed. See `error_code`. */
    WW_POOL_SLOT_ACQUIRE_ERROR = 3,
    /* Daemon advanced the point without proof from every consumer. */
    WW_POOL_SLOT_ACQUIRE_FORCED_RELEASE = 4,
    /* The renderer transport failed; no further pool work is valid. */
    WW_POOL_SLOT_ACQUIRE_SESSION_LOST = 5,
} ww_pool_slot_acquire_status_t;

typedef struct ww_pool_slot_acquire_result {
    ww_pool_slot_acquire_status_t status;
    /* Negative errno-style value for ERROR, otherwise zero. */
    int32_t        error_code;
    uint64_t       bind_generation;
    uint64_t       previous_release_point;
    ww_pool_slot_t slot;
} ww_pool_slot_acquire_result_t;

/* Acquire any free slot. Iter 1: no internal queueing — caller picks
 * a slot index it knows is free (e.g. round-robin via libmpv's
 * `mpv_render_context_update` low bits). The returned struct is a
 * snapshot; bridge does not hold internal state that cares about it.
 *
 * Returns 0 on success. Returns -EINVAL when the pool has no
 * directive applied yet, or when `slot_index` is out of range. */
int ww_bridge_pool_acquire_slot(ww_pool_t* pool, uint32_t slot_index, ww_pool_slot_t* out_slot);

/* Wait for the selected slot's previous consumer use and acquire its
 * backend snapshot as one owner operation. READY_* results contain a
 * writable `slot`. BUSY and ERROR never expose a backend handle.
 *
 * `bind_generation + slot.index + previous_release_point` identifies
 * the exact reuse proof consumed by this acquisition. The producer
 * should add its own monotonically increasing acquire serial when it
 * permits only one outstanding acquisition.
 *
 * Returns 0 when `out_result` contains a typed outcome. Returns
 * -EINVAL only when `pool` or `out_result` itself is null. */
int ww_bridge_pool_acquire_slot_for_render(ww_pool_t* pool, uint32_t slot_index,
                                           uint32_t                       timeout_ms,
                                           ww_pool_slot_acquire_result_t* out_result);

typedef enum ww_pool_release_outcome
{
    WW_POOL_RELEASE_CONSUMER_RELEASED = 0,
    WW_POOL_RELEASE_NO_CONSUMERS      = 1,
    WW_POOL_RELEASE_FORCED            = 2,
} ww_pool_release_outcome_t;

/* Record the daemon's sideband result for one release point. This is
 * thread-safe and may be called by the control-reader thread while the
 * render thread waits in `acquire_slot_for_render`.
 *
 * Exact duplicate notifications are accepted. Stale generation/point
 * notifications return -ESTALE; conflicting duplicates return
 * -EPROTO. The pool never upgrades FORCED to a writable reuse proof. */
int ww_bridge_pool_report_release(ww_pool_t* pool, uint64_t bind_generation, uint32_t slot_index,
                                  uint64_t release_point, ww_pool_release_outcome_t outcome);

typedef enum ww_pool_slot_submit_status
{
    WW_POOL_SLOT_SUBMIT_SUBMITTED    = 0,
    WW_POOL_SLOT_SUBMIT_SESSION_LOST = 1,
    WW_POOL_SLOT_SUBMIT_ERROR        = 2,
} ww_pool_slot_submit_status_t;

typedef struct ww_pool_slot_submit_result {
    ww_pool_slot_submit_status_t status;
    int32_t                      error_code;
    uint64_t                     bind_generation;
    uint32_t                     slot_index;
    uint64_t                     release_point;
} ww_pool_slot_submit_result_t;

/* Publish one rendered slot transactionally. The release point remains
 * private while `frame_ready` is sent and is committed to the slot only
 * after send success. A transport failure marks the entire pool session
 * lost; subsequent acquire/submit calls return SESSION_LOST. */
int ww_bridge_pool_submit_slot_for_render(ww_pool_t* pool, int sock, uint32_t slot_index,
                                          int                           acquire_sync_fd,
                                          ww_pool_slot_submit_result_t* out_result);

/* Submit a rendered slot. Bridge:
 *   - Takes ownership of `acquire_sync_fd` (closes after sendmsg).
 *   - Bumps the producer's release timeline by 1.
 *   - Records the new release_point on the slot.
 *   - Emits `frame_ready{image_index=slot_index, release_point=…}`
 *     on `sock`.
 *
 * `acquire_sync_fd` is the dma_fence sync_file the plugin obtained
 * from its rendering API:
 *   - EGL/GBM: `eglDupNativeFenceFDANDROID` after `glFlush`.
 *   - Vulkan:  `vkGetSemaphoreFdKHR(SYNC_FD)` after queue submit.
 *             Use a binary semaphore created with
 *             `VkExportSemaphoreCreateInfo.handleTypes = SYNC_FD`;
 *             OPAQUE_FD (timeline) is NOT cross-vendor portable.
 *
 * The fd is REQUIRED on the COMPAT_LINEAR / GPU_LINEAR path: cross-
 * vendor importing GPUs (notably amdgpu) refuse to schedule a foreign
 * dma-buf without an explicit dma_fence dependency and report
 * "Not enough memory for command submission" before losing the device.
 * On OPTIMIZED same-GPU paths the fd is optional but recommended.
 *
 * The bridge will close the fd; plugin MUST NOT close it after this
 * call. Pass -1 only on shutdown (consumers will see no acquire
 * fence and the daemon may stall briefly).
 *
 * Returns 0 on success. This is the legacy integer wrapper around
 * `ww_bridge_pool_submit_slot_for_render`. */
int ww_bridge_pool_submit_slot(ww_pool_t* pool, int sock, uint32_t slot_index, int acquire_sync_fd);

/* Block until the slot's last release_point has been signaled by the
 * daemon's reaper, with a wall-clock timeout. Plugin SHOULD call
 * this before re-rendering into the same `slot_index`. Returns 0 on
 * signal, negative on timeout/error. This is the legacy split API;
 * new producers should use `ww_bridge_pool_acquire_slot_for_render`
 * so a busy slot is never exposed as writable by accident. */
int ww_bridge_pool_wait_slot_release(ww_pool_t* pool, uint32_t slot_index, uint32_t timeout_ms);

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* WAYWALLEN_BRIDGE_POOL_H */
