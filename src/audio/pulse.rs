use std::ffi::{c_char, c_float, c_int, c_void, CStr};
use std::ptr::NonNull;

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
    ) -> usize;
    fn ww_pulse_capture_generation(capture: *mut c_void) -> u64;
    fn ww_pulse_capture_failed(
        capture: *mut c_void,
        error: *mut c_char,
        error_capacity: usize,
    ) -> c_int;
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
    fn generation(&self) -> u64;
    fn read(&mut self, samples: &mut [f32]) -> Result<usize, CaptureError>;
    fn discard(&mut self);
}

pub struct PulseCapture {
    handle: NonNull<c_void>,
}

// The opaque handle is created, polled, and destroyed by one audio worker.
unsafe impl Send for PulseCapture {}

impl PulseCapture {
    pub fn open() -> Result<Self, CaptureError> {
        let mut code = 0;
        let mut error = [0i8; ERROR_CAPACITY];
        let handle = unsafe { ww_pulse_capture_open(&mut code, error.as_mut_ptr(), error.len()) };
        let Some(handle) = NonNull::new(handle) else {
            return Err(CaptureError {
                kind: error_kind(code),
                message: c_message(&error),
            });
        };
        Ok(Self { handle })
    }

    fn status(&self) -> Result<(), CaptureError> {
        let mut error = [0i8; ERROR_CAPACITY];
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
    fn generation(&self) -> u64 {
        unsafe { ww_pulse_capture_generation(self.handle.as_ptr()) }
    }

    fn read(&mut self, samples: &mut [f32]) -> Result<usize, CaptureError> {
        self.status()?;
        let frames = unsafe {
            ww_pulse_capture_read(
                self.handle.as_ptr(),
                samples.as_mut_ptr(),
                samples.len() / 2,
            )
        };
        Ok(frames)
    }

    fn discard(&mut self) {
        let mut scratch = [0.0f32; 2048];
        while unsafe {
            ww_pulse_capture_read(
                self.handle.as_ptr(),
                scratch.as_mut_ptr(),
                scratch.len() / 2,
            )
        } > 0
        {}
    }
}

impl Drop for PulseCapture {
    fn drop(&mut self) {
        unsafe { ww_pulse_capture_close(self.handle.as_ptr()) };
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
