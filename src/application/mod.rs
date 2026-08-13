use std::time::Duration;

use crate::catalog::entry::WallpaperEntry;
use crate::wallframe::scheduler::DisplayId;

mod catalog;
pub mod playback;
mod plugins;

pub use catalog::*;
pub use playback::*;
pub use plugins::*;

pub const APPLY_FIRST_FRAME_TIMEOUT: Duration = Duration::from_secs(15);
const PLUGIN_UPDATE_NOTIFICATION_ID: &str = "org.waywallen.waywallen.plugin-updates";

pub struct ApplyResult {
    pub renderer_id: String,
    pub entry: WallpaperEntry,
    pub activation: ApplyActivation,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApplyActivation {
    Active,
    Deferred,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RendererSharingPolicy {
    #[default]
    UseSettings,
    Shared,
}

#[derive(Clone, Debug, Default)]
pub struct ApplyRequest {
    pub display_ids: Option<Vec<DisplayId>>,
    pub renderer_name: Option<String>,
    pub first_frame_timeout: Option<Duration>,
    pub require_display: bool,
    pub sharing: RendererSharingPolicy,
}

fn should_duplicate_renderers(
    setting_enabled: bool,
    has_targets: bool,
    sharing: RendererSharingPolicy,
) -> bool {
    setting_enabled && has_targets && sharing == RendererSharingPolicy::UseSettings
}

#[cfg(test)]
mod tests;
