pub(crate) mod api;
pub(crate) mod application;
pub mod catalog;
pub mod control_proto;
pub mod daemon;
pub mod error;
mod event_process;
pub mod events;
pub mod model;
pub mod playback;
pub mod plugin;
pub mod probe;
pub mod settings;
pub mod system;
pub mod tasks;
pub mod wallframe;

pub(crate) use daemon::{spawn_ui, DaemonContext};
