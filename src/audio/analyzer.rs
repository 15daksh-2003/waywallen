use std::f32::consts::PI;

pub const AUDIO_SPECTRUM_BINS: usize = 64;
const FFT_SIZE: usize = 4096;
const HALF_FFT: usize = FFT_SIZE / 2;
const HOP_SIZE: usize = 1024;
const SAMPLE_RATE: f32 = 48_000.0;
const MIN_FREQUENCY_HZ: f32 = 10.0;
const MAX_FREQUENCY_HZ: f32 = 16_000.0;
const TILT_PIVOT_HZ: f32 = 1000.0;
const TILT_EXP: f32 = 1.15;
const DB_FLOOR: f32 = -100.0;
const DB_CEIL: f32 = -8.0;
const RESPONSE_CONTRAST: f32 = 1.6;
const ATTACK_TIME_SECONDS: f32 = 0.030;
const RELEASE_TIME_SECONDS: f32 = 0.140;

#[derive(Debug, Clone, PartialEq)]
pub struct AudioSpectrumFrame {
    pub generation: u64,
    pub sequence: u64,
    pub captured_at_ns: u64,
    pub left: [f32; AUDIO_SPECTRUM_BINS],
    pub right: [f32; AUDIO_SPECTRUM_BINS],
}

#[derive(Clone, Copy, Default)]
struct Complex {
    re: f32,
    im: f32,
}

impl Complex {
    fn magnitude(self) -> f32 {
        self.re.hypot(self.im)
    }

    fn mul(self, other: Self) -> Self {
        Self {
            re: self.re * other.re - self.im * other.im,
            im: self.re * other.im + self.im * other.re,
        }
    }
}

#[derive(Clone)]
struct BandLayout {
    edges: [usize; AUDIO_SPECTRUM_BINS + 1],
    gain: [f32; AUDIO_SPECTRUM_BINS],
}

pub struct SpectrumAnalyzer {
    ring_left: [f32; FFT_SIZE],
    ring_right: [f32; FFT_SIZE],
    ring_head: usize,
    samples_filled: usize,
    samples_since_fft: usize,
    smoothed_left: [f32; AUDIO_SPECTRUM_BINS],
    smoothed_right: [f32; AUDIO_SPECTRUM_BINS],
    layout: BandLayout,
    generation: u64,
    sequence: u64,
}

impl Default for SpectrumAnalyzer {
    fn default() -> Self {
        Self {
            ring_left: [0.0; FFT_SIZE],
            ring_right: [0.0; FFT_SIZE],
            ring_head: 0,
            samples_filled: 0,
            samples_since_fft: 0,
            smoothed_left: [0.0; AUDIO_SPECTRUM_BINS],
            smoothed_right: [0.0; AUDIO_SPECTRUM_BINS],
            layout: make_band_layout(),
            generation: 0,
            sequence: 0,
        }
    }
}

impl SpectrumAnalyzer {
    pub fn reset(&mut self, generation: u64) {
        self.clear();
        if generation != self.generation {
            self.sequence = 0;
        }
        self.generation = generation;
    }

    pub fn clear(&mut self) {
        self.ring_left.fill(0.0);
        self.ring_right.fill(0.0);
        self.ring_head = 0;
        self.samples_filled = 0;
        self.samples_since_fft = 0;
        self.smoothed_left.fill(0.0);
        self.smoothed_right.fill(0.0);
    }

    pub fn ingest_interleaved(
        &mut self,
        generation: u64,
        captured_at_ns: u64,
        samples: &[f32],
    ) -> Option<AudioSpectrumFrame> {
        if generation != self.generation {
            self.reset(generation);
        }
        for frame in samples.chunks_exact(2) {
            self.ring_left[self.ring_head] = finite_sample(frame[0]);
            self.ring_right[self.ring_head] = finite_sample(frame[1]);
            self.ring_head = (self.ring_head + 1) % FFT_SIZE;
            self.samples_filled = (self.samples_filled + 1).min(FFT_SIZE);
            self.samples_since_fft += 1;
        }
        if self.samples_filled < FFT_SIZE || self.samples_since_fft < HOP_SIZE {
            return None;
        }
        self.samples_since_fft = 0;

        let mut left = [Complex::default(); FFT_SIZE];
        let mut right = [Complex::default(); FFT_SIZE];
        for index in 0..FFT_SIZE {
            let ring_index = (self.ring_head + index) % FFT_SIZE;
            let window = hann_window(index);
            left[index].re = self.ring_left[ring_index] * window;
            right[index].re = self.ring_right[ring_index] * window;
        }
        fft_in_place(&mut left);
        fft_in_place(&mut right);

        let norm = 2.0 / FFT_SIZE as f32;
        let dt = HOP_SIZE as f32 / SAMPLE_RATE;
        let mut output_left = [0.0; AUDIO_SPECTRUM_BINS];
        let mut output_right = [0.0; AUDIO_SPECTRUM_BINS];
        for band in 0..AUDIO_SPECTRUM_BINS {
            let raw_left = visual_response(
                band_magnitude(&left, &self.layout, band, norm),
                &self.layout,
                band,
            );
            let raw_right = visual_response(
                band_magnitude(&right, &self.layout, band, norm),
                &self.layout,
                band,
            );
            self.smoothed_left[band] = smooth(self.smoothed_left[band], raw_left, dt);
            self.smoothed_right[band] = smooth(self.smoothed_right[band], raw_right, dt);
            output_left[band] = normalized(self.smoothed_left[band]);
            output_right[band] = normalized(self.smoothed_right[band]);
        }
        self.sequence += 1;
        Some(AudioSpectrumFrame {
            generation,
            sequence: self.sequence,
            captured_at_ns,
            left: output_left,
            right: output_right,
        })
    }
}

fn finite_sample(value: f32) -> f32 {
    if value.is_finite() {
        value
    } else {
        0.0
    }
}

fn normalized(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

const BAND_ANCHORS: [(f32, f32); 11] = [
    (MIN_FREQUENCY_HZ, 0.0),
    (60.0, 2.0),
    (125.0, 5.0),
    (250.0, 10.0),
    (500.0, 21.0),
    (1000.0, 32.0),
    (2000.0, 38.0),
    (3000.0, 46.0),
    (8000.0, 54.0),
    (12000.0, 60.0),
    (MAX_FREQUENCY_HZ, 64.0),
];

fn anchor_frequency(band: f32) -> f32 {
    if band <= BAND_ANCHORS[0].1 {
        return BAND_ANCHORS[0].0;
    }
    for pair in BAND_ANCHORS.windows(2) {
        let (lo_hz, lo_band) = pair[0];
        let (hi_hz, hi_band) = pair[1];
        if band <= hi_band {
            let t = (band - lo_band) / (hi_band - lo_band);
            return (lo_hz.ln() + (hi_hz.ln() - lo_hz.ln()) * t).exp();
        }
    }
    MAX_FREQUENCY_HZ
}

fn upper_bin(hz: f32) -> usize {
    ((hz * FFT_SIZE as f32 / SAMPLE_RATE).ceil() as usize).clamp(1, HALF_FFT)
}

fn make_band_layout() -> BandLayout {
    let mut layout = BandLayout {
        edges: [0; AUDIO_SPECTRUM_BINS + 1],
        gain: [0.0; AUDIO_SPECTRUM_BINS],
    };
    let max_bin = upper_bin(MAX_FREQUENCY_HZ.min(SAMPLE_RATE * 0.5));
    for band in 0..AUDIO_SPECTRUM_BINS {
        let mut next = upper_bin(anchor_frequency(band as f32 - 0.5).min(MAX_FREQUENCY_HZ));
        if band > 0 && next <= layout.edges[band - 1] {
            next = layout.edges[band - 1] + 1;
        }
        let remaining = AUDIO_SPECTRUM_BINS - band;
        if next + remaining > max_bin {
            next = max_bin - remaining;
        }
        layout.edges[band] = next;
    }
    layout.edges[AUDIO_SPECTRUM_BINS] = max_bin;
    for band in 0..AUDIO_SPECTRUM_BINS {
        let upper_hz = layout.edges[band + 1] as f32 * SAMPLE_RATE / FFT_SIZE as f32;
        layout.gain[band] = (upper_hz / TILT_PIVOT_HZ).powf(TILT_EXP);
    }
    layout
}

fn band_magnitude(
    spectrum: &[Complex; FFT_SIZE],
    layout: &BandLayout,
    band: usize,
    norm: f32,
) -> f32 {
    let lo = layout.edges[band];
    let hi = layout.edges[band + 1];
    let sum: f32 = spectrum[lo..hi].iter().map(|value| value.magnitude()).sum();
    sum / (hi - lo).max(1) as f32 * norm
}

fn shape_response(unit: f32) -> f32 {
    let value = unit.clamp(0.0, 1.0);
    if value <= 0.5 {
        0.5 * (value * 2.0).powf(RESPONSE_CONTRAST)
    } else {
        1.0 - 0.5 * ((1.0 - value) * 2.0).powf(RESPONSE_CONTRAST)
    }
}

fn visual_response(magnitude: f32, layout: &BandLayout, band: usize) -> f32 {
    let compensated = (magnitude * layout.gain[band]).max(1.0e-12);
    let db = 20.0 * compensated.log10();
    shape_response(((db - DB_FLOOR) / (DB_CEIL - DB_FLOOR)).clamp(0.0, 1.0)).min(1.0)
}

fn smooth(previous: f32, current: f32, dt: f32) -> f32 {
    let tau = if current > previous {
        ATTACK_TIME_SECONDS
    } else {
        RELEASE_TIME_SECONDS
    };
    previous + (1.0 - (-dt / tau).exp()) * (current - previous)
}

fn hann_window(index: usize) -> f32 {
    0.5 * (1.0 - (2.0 * PI * index as f32 / (FFT_SIZE - 1) as f32).cos())
}

fn fft_in_place(data: &mut [Complex; FFT_SIZE]) {
    let mut swap_index = 0;
    for index in 1..FFT_SIZE {
        let mut bit = FFT_SIZE >> 1;
        while swap_index & bit != 0 {
            swap_index ^= bit;
            bit >>= 1;
        }
        swap_index ^= bit;
        if index < swap_index {
            data.swap(index, swap_index);
        }
    }

    let mut len = 2;
    while len <= FFT_SIZE {
        let angle = -2.0 * PI / len as f32;
        let root = Complex {
            re: angle.cos(),
            im: angle.sin(),
        };
        for base in (0..FFT_SIZE).step_by(len) {
            let mut weight = Complex { re: 1.0, im: 0.0 };
            for offset in 0..len / 2 {
                let even = data[base + offset];
                let odd = data[base + offset + len / 2].mul(weight);
                data[base + offset] = Complex {
                    re: even.re + odd.re,
                    im: even.im + odd.im,
                };
                data[base + offset + len / 2] = Complex {
                    re: even.re - odd.re,
                    im: even.im - odd.im,
                };
                weight = weight.mul(root);
            }
        }
        len <<= 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(hz: f32, left_amplitude: f32, right_amplitude: f32) -> Vec<f32> {
        (0..FFT_SIZE)
            .flat_map(|index| {
                let sample = (2.0 * PI * hz * index as f32 / SAMPLE_RATE).sin();
                [sample * left_amplitude, sample * right_amplitude]
            })
            .collect()
    }

    fn peak(values: &[f32; AUDIO_SPECTRUM_BINS]) -> usize {
        values
            .iter()
            .enumerate()
            .max_by(|left, right| left.1.total_cmp(right.1))
            .map(|(index, _)| index)
            .unwrap()
    }

    #[test]
    fn tone_mapping_matches_existing_we_layout() {
        let cases = [
            (60.0, 0.04),
            (125.0, 0.08),
            (250.0, 0.16),
            (500.0, 0.33),
            (1000.0, 0.51),
            (2000.0, 0.60),
            (3000.0, 0.72),
            (8000.0, 0.85),
            (12000.0, 0.94),
        ];
        let mut previous = None;
        for (hz, expected) in cases {
            let mut analyzer = SpectrumAnalyzer::default();
            let frame = analyzer
                .ingest_interleaved(1, 1, &tone(hz, 0.25, 0.25))
                .unwrap();
            let band = peak(&frame.left);
            let center = (band as f32 + 0.5) / AUDIO_SPECTRUM_BINS as f32;
            assert!(
                (center - expected).abs() <= 0.03,
                "{hz} Hz peaked at {center}"
            );
            if let Some(previous) = previous {
                assert!(band > previous);
            }
            previous = Some(band);
        }
    }

    #[test]
    fn channel_split_non_finite_and_generation_reset() {
        let mut analyzer = SpectrumAnalyzer::default();
        let mut samples = tone(1000.0, 1.0, 0.0);
        samples[0] = f32::NAN;
        let first = analyzer.ingest_interleaved(2, 10, &samples).unwrap();
        let band = peak(&first.left);
        assert!(first.left[band] > 0.0 && first.left[band] <= 1.0);
        assert_eq!(first.right[band], 0.0);
        assert_eq!(first.sequence, 1);

        analyzer.clear();
        let resumed = analyzer.ingest_interleaved(2, 15, &samples).unwrap();
        assert_eq!(resumed.generation, 2);
        assert_eq!(resumed.sequence, 2);

        let second = analyzer.ingest_interleaved(3, 20, &samples).unwrap();
        assert_eq!(second.generation, 3);
        assert_eq!(second.sequence, 1);
        assert!(second
            .left
            .iter()
            .chain(second.right.iter())
            .all(|value| value.is_finite() && (0.0..=1.0).contains(value)));
    }
}
