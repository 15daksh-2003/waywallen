/* waywallen-bridge — protocol bit constants shared by every renderer
 * subprocess that participates in buffer negotiation.
 *
 * These values mirror waywallen/src/negotiate.rs (and the daemon's
 * `ConfigureBuffers` flag set). Producers OR them into the
 * `mem_hints` / `sync_caps` / `color_caps` fields of
 * `waywallen_producer_capabilities_t` and check them on incoming
 * `waywallen_buffer_directive_t`. The daemon takes the intersection with
 * each consumer's caps to pick a scheme.
 *
 * Keep in sync with negotiate.rs — all bits are part of the wire
 * contract.
 */
#ifndef WAYWALLEN_BRIDGE_PROTOCOL_BITS_H
#define WAYWALLEN_BRIDGE_PROTOCOL_BITS_H

#ifdef __cplusplus
extern "C" {
#endif

/* `mem_hints`: where the dmabuf is backed. Topology-first picker:
 * cross-device emits 0 (bridge picks any dma-buf-exportable type);
 * same-device prefers DEVICE_LOCAL when both peers have it. */
#define WW_MEM_HINT_DEVICE_LOCAL (1u << 0)
#define WW_MEM_HINT_HOST_VISIBLE (1u << 1)
#define WW_MEM_HINT_SCANOUT      (1u << 2)
#define WW_MEM_HINT_PROTECTED    (1u << 3)

/* `sync_caps`: which fence flavours the peer supports. The picker
 * lands on the highest tier both sides advertise. */
#define WW_SYNC_DMABUF_IMPLICIT  (1u << 0)
#define WW_SYNC_SYNCOBJ_BINARY   (1u << 1)
#define WW_SYNC_SYNCOBJ_TIMELINE (1u << 2)

/* `color_caps`: per-axis intersection (encoding | range | alpha). */
#define WW_COLOR_ENC_SRGB       (1u << 0)
#define WW_COLOR_ENC_BT709      (1u << 1)
#define WW_COLOR_ENC_BT2020     (1u << 2)
#define WW_COLOR_RANGE_FULL     (1u << 5)
#define WW_COLOR_RANGE_LIMITED  (1u << 6)
#define WW_COLOR_ALPHA_PREMUL   (1u << 7)
#define WW_COLOR_ALPHA_STRAIGHT (1u << 8)

/* `flags` on `waywallen_buffer_pool_t`. Bit 0
 * tells the consumer the dmabuf is backed by HOST_VISIBLE memory
 * (GTT) so a foreign GPU can PRIME-import it. */
#define WW_BUF_HOST_VISIBLE (1u << 0)

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* WAYWALLEN_BRIDGE_PROTOCOL_BITS_H */
