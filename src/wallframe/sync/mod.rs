pub mod drm_syncobj;
pub mod reaper;

pub use drm_syncobj::{merge_sync_files, BinarySyncobjState, DrmDevice, SyncobjHandle};
pub(crate) use reaper::{register_frame, spawn_reaper, FrameRecord};
pub use reaper::{
    DisplaySessionId, FrameConsumerArm, FrameConsumerIdentity, FrameConsumerMember,
    FrameConsumerSession, FrameIdentity, ReleaseEvent, ReleaseWaitState,
};

/// Daemon-global, lazily opened DRM render node for drm_syncobj.
/// Returns the same `&'static DrmDevice` on every call.
pub fn drm_device() -> std::io::Result<&'static DrmDevice> {
    use std::sync::OnceLock;
    static DEV: OnceLock<DrmDevice> = OnceLock::new();
    if let Some(d) = DEV.get() {
        return Ok(d);
    }
    let new = DrmDevice::open_first_render_node()?;
    let _ = DEV.set(new);
    Ok(DEV.get().expect("just set"))
}
