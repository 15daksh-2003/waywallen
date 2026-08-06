use std::ffi::{c_char, c_float, c_int, c_void, CStr};
use std::ptr::NonNull;

use super::window::{AudioPcmWindow, PcmWindowAssembler};

const ERROR_CAPACITY: usize = 512;

#[link(name = "waywallen_pulse_adapter", kind = "static")]
unsafe extern "C" {
    fn ww_pulse_capture_open(
        error_code: *mut c_int,
        error: *mut c_char,
        error_capacity: usize,
    ) -> *mut c_void;
    fn ww_pulse_capture_close(capture: *mut c_void);
    fn ww_pulse_capture_read(
        capture: *mut c_void,
        samples: *mut c_float,
        frame_capacity: usize,
        generation: *mut u64,
    ) -> usize;
    fn ww_pulse_capture_failed(
        capture: *mut c_void,
        error: *mut c_char,
        error_capacity: usize,
    ) -> c_int;
    fn ww_pulse_playback_observer_open(
        error_code: *mut c_int,
        error: *mut c_char,
        error_capacity: usize,
    ) -> *mut c_void;
    fn ww_pulse_playback_observer_close(observer: *mut c_void);
    fn ww_pulse_playback_observer_snapshot(
        observer: *mut c_void,
        streams: *mut PulsePlaybackStream,
        capacity: usize,
    ) -> usize;
    fn ww_pulse_playback_observer_failed(
        observer: *mut c_void,
        error: *mut c_char,
        error_capacity: usize,
    ) -> c_int;
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PulsePlaybackStream {
    pub index: u32,
    pub process_id: i32,
    pub corked: c_int,
    pub muted: c_int,
    pub has_nonzero_volume: c_int,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureErrorKind {
    LibraryUnavailable,
    MissingSymbol,
    ServerUnavailable,
    MonitorUnavailable,
    StreamFailed,
    OutOfMemory,
    Unknown,
}

#[derive(Debug)]
pub struct CaptureError {
    pub kind: CaptureErrorKind,
    pub message: String,
}

impl std::fmt::Display for CaptureError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}", self.message)
    }
}

impl std::error::Error for CaptureError {}

pub trait AudioCaptureBackend: Send {
    fn snapshot(&mut self, captured_at_ns: u64) -> Result<Option<AudioPcmWindow>, CaptureError>;
    fn discard(&mut self);
}

pub struct PulseCapture {
    handle: NonNull<c_void>,
    assembler: PcmWindowAssembler,
    scratch: [f32; 2048],
}

// The opaque handle is created, polled, and destroyed by one audio worker.
unsafe impl Send for PulseCapture {}

impl PulseCapture {
    pub fn open() -> Result<Self, CaptureError> {
        let mut code = 0;
        let mut error = [0 as c_char; ERROR_CAPACITY];
        let handle = unsafe { ww_pulse_capture_open(&mut code, error.as_mut_ptr(), error.len()) };
        let Some(handle) = NonNull::new(handle) else {
            return Err(CaptureError {
                kind: error_kind(code),
                message: c_message(&error),
            });
        };
        Ok(Self {
            handle,
            assembler: PcmWindowAssembler::default(),
            scratch: [0.0; 2048],
        })
    }

    fn status(&self) -> Result<(), CaptureError> {
        let mut error = [0 as c_char; ERROR_CAPACITY];
        let code = unsafe {
            ww_pulse_capture_failed(self.handle.as_ptr(), error.as_mut_ptr(), error.len())
        };
        if code == 0 {
            Ok(())
        } else {
            Err(CaptureError {
                kind: error_kind(code),
                message: c_message(&error),
            })
        }
    }
}

impl AudioCaptureBackend for PulseCapture {
    fn snapshot(&mut self, captured_at_ns: u64) -> Result<Option<AudioPcmWindow>, CaptureError> {
        self.status()?;
        let mut generation = 0;
        let frames = unsafe {
            ww_pulse_capture_read(
                self.handle.as_ptr(),
                self.scratch.as_mut_ptr(),
                self.scratch.len() / 2,
                &mut generation,
            )
        };
        self.assembler
            .ingest_interleaved(generation, &self.scratch[..frames * 2]);
        Ok(self.assembler.snapshot(captured_at_ns))
    }

    fn discard(&mut self) {
        loop {
            let mut generation = 0;
            let frames = unsafe {
                ww_pulse_capture_read(
                    self.handle.as_ptr(),
                    self.scratch.as_mut_ptr(),
                    self.scratch.len() / 2,
                    &mut generation,
                )
            };
            self.assembler.ingest_interleaved(generation, &[]);
            if frames == 0 {
                break;
            }
        }
        self.assembler.discard();
    }
}

impl Drop for PulseCapture {
    fn drop(&mut self) {
        unsafe { ww_pulse_capture_close(self.handle.as_ptr()) };
    }
}

pub struct PulsePlaybackObserver {
    handle: NonNull<c_void>,
}

// The opaque handle is polled and destroyed by one audio worker. PulseAudio
// callbacks synchronize snapshots inside the adapter.
unsafe impl Send for PulsePlaybackObserver {}

impl PulsePlaybackObserver {
    pub fn open() -> Result<Self, CaptureError> {
        let mut code = 0;
        let mut error = [0 as c_char; ERROR_CAPACITY];
        let handle =
            unsafe { ww_pulse_playback_observer_open(&mut code, error.as_mut_ptr(), error.len()) };
        let Some(handle) = NonNull::new(handle) else {
            return Err(CaptureError {
                kind: error_kind(code),
                message: c_message(&error),
            });
        };
        Ok(Self { handle })
    }

    pub fn snapshot(&self) -> Result<Vec<PulsePlaybackStream>, CaptureError> {
        self.status()?;
        let mut streams = Vec::new();
        loop {
            let count = unsafe {
                ww_pulse_playback_observer_snapshot(
                    self.handle.as_ptr(),
                    streams.as_mut_ptr(),
                    streams.capacity(),
                )
            };
            if count <= streams.capacity() {
                unsafe { streams.set_len(count) };
                return Ok(streams);
            }
            streams.reserve(count - streams.capacity());
        }
    }

    fn status(&self) -> Result<(), CaptureError> {
        let mut error = [0 as c_char; ERROR_CAPACITY];
        let code = unsafe {
            ww_pulse_playback_observer_failed(self.handle.as_ptr(), error.as_mut_ptr(), error.len())
        };
        if code == 0 {
            Ok(())
        } else {
            Err(CaptureError {
                kind: error_kind(code),
                message: c_message(&error),
            })
        }
    }
}

impl Drop for PulsePlaybackObserver {
    fn drop(&mut self) {
        unsafe { ww_pulse_playback_observer_close(self.handle.as_ptr()) };
    }
}

fn error_kind(code: i32) -> CaptureErrorKind {
    match code {
        1 => CaptureErrorKind::LibraryUnavailable,
        2 => CaptureErrorKind::MissingSymbol,
        3 => CaptureErrorKind::ServerUnavailable,
        4 => CaptureErrorKind::MonitorUnavailable,
        5 => CaptureErrorKind::StreamFailed,
        6 => CaptureErrorKind::OutOfMemory,
        _ => CaptureErrorKind::Unknown,
    }
}

fn c_message(buffer: &[c_char]) -> String {
    unsafe { CStr::from_ptr(buffer.as_ptr()) }
        .to_string_lossy()
        .into_owned()
}
