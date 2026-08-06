use super::pulse::{CaptureError, PulsePlaybackObserver, PulsePlaybackStream};
use crate::renderer_manager::RendererProcessOwnershipSnapshot;

pub trait PlaybackObserverBackend: Send {
    fn snapshot(&mut self) -> Result<Vec<PulsePlaybackStream>, CaptureError>;
}

impl PlaybackObserverBackend for PulsePlaybackObserver {
    fn snapshot(&mut self) -> Result<Vec<PulsePlaybackStream>, CaptureError> {
        PulsePlaybackObserver::snapshot(self)
    }
}

pub fn has_external_playback(
    streams: &[PulsePlaybackStream],
    ownership: &RendererProcessOwnershipSnapshot,
) -> bool {
    has_external_playback_with(streams, ownership, |pid| {
        let process_group = unsafe { libc::getpgid(pid) };
        (process_group > 0).then_some(process_group)
    })
}

fn has_external_playback_with(
    streams: &[PulsePlaybackStream],
    ownership: &RendererProcessOwnershipSnapshot,
    mut process_group_for: impl FnMut(i32) -> Option<i32>,
) -> bool {
    streams.iter().any(|stream| {
        if stream.corked != 0 || stream.muted != 0 || stream.has_nonzero_volume == 0 {
            return false;
        }
        let Some(process_group) = (stream.process_id > 0)
            .then(|| process_group_for(stream.process_id))
            .flatten()
        else {
            return true;
        };
        !ownership.owns_process_group(process_group)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stream(process_id: i32) -> PulsePlaybackStream {
        PulsePlaybackStream {
            index: process_id as u32,
            process_id,
            corked: 0,
            muted: 0,
            has_nonzero_volume: 1,
        }
    }

    #[test]
    fn active_owned_process_group_is_ignored() {
        let ownership = RendererProcessOwnershipSnapshot::from_process_groups([42]);
        assert!(!has_external_playback_with(
            &[stream(7)],
            &ownership,
            |_| Some(42)
        ));
    }

    #[test]
    fn active_process_from_different_group_is_external() {
        let ownership = RendererProcessOwnershipSnapshot::from_process_groups([42]);
        assert!(has_external_playback_with(&[stream(7)], &ownership, |_| {
            Some(77)
        }));
    }

    #[test]
    fn missing_process_identity_is_external() {
        let ownership = RendererProcessOwnershipSnapshot::default();
        assert!(has_external_playback_with(&[stream(0)], &ownership, |_| {
            None
        }));
    }

    #[test]
    fn corked_muted_and_zero_volume_streams_are_inactive() {
        let ownership = RendererProcessOwnershipSnapshot::default();
        let mut corked = stream(1);
        corked.corked = 1;
        let mut muted = stream(2);
        muted.muted = 1;
        let mut zero = stream(3);
        zero.has_nonzero_volume = 0;
        assert!(!has_external_playback_with(
            &[corked, muted, zero],
            &ownership,
            |_| Some(9),
        ));
    }
}
