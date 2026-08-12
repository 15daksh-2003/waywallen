use crate::playback::Mode;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Playlist {
    pub id: i64,
    pub name: String,
    pub mode: Mode,
    pub interval_secs: u32,
}
