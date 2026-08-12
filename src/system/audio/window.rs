use std::sync::Arc;

pub const AUDIO_SAMPLE_RATE_HZ: u32 = 48_000;
pub const AUDIO_CHANNELS: u32 = 2;
pub const AUDIO_WINDOW_FRAMES: usize = 4096;
pub const AUDIO_SAMPLE_COUNT: usize = AUDIO_WINDOW_FRAMES * AUDIO_CHANNELS as usize;
pub const AUDIO_WINDOW_END_OF_STREAM: u32 = 1;

#[derive(Debug, Clone, PartialEq)]
pub struct AudioPcmWindow {
    pub generation: u64,
    pub sequence: u64,
    pub captured_at_ns: u64,
    pub end_sample_frame: u64,
    pub sample_rate_hz: u32,
    pub channels: u32,
    pub frames: u32,
    pub samples: Arc<[f32; AUDIO_SAMPLE_COUNT]>,
}

pub struct PcmWindowAssembler {
    ring: [f32; AUDIO_SAMPLE_COUNT],
    head: usize,
    filled: usize,
    generation: Option<u64>,
    sequence: u64,
    end_sample_frame: u64,
    last_snapshot_end: u64,
}

impl Default for PcmWindowAssembler {
    fn default() -> Self {
        Self {
            ring: [0.0; AUDIO_SAMPLE_COUNT],
            head: 0,
            filled: 0,
            generation: None,
            sequence: 0,
            end_sample_frame: 0,
            last_snapshot_end: 0,
        }
    }
}

impl PcmWindowAssembler {
    pub fn ingest_interleaved(&mut self, generation: u64, samples: &[f32]) {
        if self.generation != Some(generation) {
            self.reset(generation);
        }
        for frame in samples.chunks_exact(AUDIO_CHANNELS as usize) {
            let offset = self.head * AUDIO_CHANNELS as usize;
            self.ring[offset] = finite(frame[0]);
            self.ring[offset + 1] = finite(frame[1]);
            self.head = (self.head + 1) % AUDIO_WINDOW_FRAMES;
            self.filled = (self.filled + 1).min(AUDIO_WINDOW_FRAMES);
            self.end_sample_frame = self.end_sample_frame.saturating_add(1);
        }
    }

    pub fn snapshot(&mut self, captured_at_ns: u64) -> Option<AudioPcmWindow> {
        if self.filled < AUDIO_WINDOW_FRAMES || self.end_sample_frame == self.last_snapshot_end {
            return None;
        }
        let mut samples = [0.0; AUDIO_SAMPLE_COUNT];
        for index in 0..AUDIO_WINDOW_FRAMES {
            let source = ((self.head + index) % AUDIO_WINDOW_FRAMES) * AUDIO_CHANNELS as usize;
            let destination = index * AUDIO_CHANNELS as usize;
            samples[destination] = self.ring[source];
            samples[destination + 1] = self.ring[source + 1];
        }
        self.sequence = self.sequence.saturating_add(1);
        self.last_snapshot_end = self.end_sample_frame;
        Some(AudioPcmWindow {
            generation: self.generation.unwrap_or_default(),
            sequence: self.sequence,
            captured_at_ns,
            end_sample_frame: self.end_sample_frame,
            sample_rate_hz: AUDIO_SAMPLE_RATE_HZ,
            channels: AUDIO_CHANNELS,
            frames: AUDIO_WINDOW_FRAMES as u32,
            samples: Arc::new(samples),
        })
    }

    pub fn discard(&mut self) {
        self.ring.fill(0.0);
        self.head = 0;
        self.filled = 0;
        self.last_snapshot_end = self.end_sample_frame;
    }

    fn reset(&mut self, generation: u64) {
        self.ring.fill(0.0);
        self.head = 0;
        self.filled = 0;
        self.generation = Some(generation);
        self.sequence = 0;
        self.end_sample_frame = 0;
        self.last_snapshot_end = 0;
    }
}

fn finite(sample: f32) -> f32 {
    if sample.is_finite() {
        sample
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(frames: usize) -> Vec<f32> {
        (0..frames)
            .flat_map(|frame| [frame as f32, -(frame as f32)])
            .collect()
    }

    #[test]
    fn publishes_only_complete_fresh_windows() {
        let mut assembler = PcmWindowAssembler::default();
        assembler.ingest_interleaved(3, &input(AUDIO_WINDOW_FRAMES - 1));
        assert!(assembler.snapshot(10).is_none());
        assembler.ingest_interleaved(3, &[f32::NAN, f32::INFINITY]);
        let window = assembler.snapshot(11).unwrap();
        assert_eq!(window.sequence, 1);
        assert_eq!(window.end_sample_frame, AUDIO_WINDOW_FRAMES as u64);
        assert_eq!(window.samples[AUDIO_SAMPLE_COUNT - 2], 0.0);
        assert_eq!(window.samples[AUDIO_SAMPLE_COUNT - 1], 0.0);
        assert!(assembler.snapshot(12).is_none());
    }

    #[test]
    fn chunking_does_not_change_the_window() {
        let samples = input(AUDIO_WINDOW_FRAMES + 17);
        let mut whole = PcmWindowAssembler::default();
        whole.ingest_interleaved(1, &samples);
        let whole = whole.snapshot(1).unwrap();

        let mut chunked = PcmWindowAssembler::default();
        for chunk in samples.chunks(74) {
            chunked.ingest_interleaved(1, chunk);
        }
        let chunked = chunked.snapshot(1).unwrap();
        assert_eq!(whole.samples, chunked.samples);
        assert_eq!(whole.end_sample_frame, chunked.end_sample_frame);
    }

    #[test]
    fn generation_change_cannot_mix_samples() {
        let mut assembler = PcmWindowAssembler::default();
        assembler.ingest_interleaved(1, &input(AUDIO_WINDOW_FRAMES - 1));
        assembler.ingest_interleaved(2, &vec![1.0; AUDIO_SAMPLE_COUNT]);
        let window = assembler.snapshot(1).unwrap();
        assert_eq!(window.generation, 2);
        assert_eq!(window.sequence, 1);
        assert!(window.samples.iter().all(|sample| *sample == 1.0));
    }
}
