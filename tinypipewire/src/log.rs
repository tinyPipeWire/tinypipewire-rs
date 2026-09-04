//! The library's own diagnostics: where they go and how much of them.

use std::ffi::{c_char, c_int, c_void, CStr};
use std::sync::Mutex;

use tinypipewire_sys as sys;

use crate::util::guard;

/// Severity of one log message, most to least severe.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum LogLevel {
    /// Something failed.
    Error,
    /// The library's default threshold.
    Warning,
    /// Ordinary progress.
    Info,
    /// Detail useful while debugging.
    Debug,
    /// Per-cycle detail.
    Verbose,
}

impl LogLevel {
    fn to_raw(self) -> sys::tpw_log_level {
        match self {
            LogLevel::Error => sys::TPW_LOG_ERROR,
            LogLevel::Warning => sys::TPW_LOG_WARNING,
            LogLevel::Info => sys::TPW_LOG_INFO,
            LogLevel::Debug => sys::TPW_LOG_DEBUG,
            LogLevel::Verbose => sys::TPW_LOG_VERBOSE,
        }
    }

    fn from_raw(raw: sys::tpw_log_level) -> Self {
        match raw {
            sys::TPW_LOG_ERROR => LogLevel::Error,
            sys::TPW_LOG_WARNING => LogLevel::Warning,
            sys::TPW_LOG_INFO => LogLevel::Info,
            sys::TPW_LOG_DEBUG => LogLevel::Debug,
            _ => LogLevel::Verbose,
        }
    }
}

/// One already-formatted message from the library.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Record<'a> {
    /// How severe the message is.
    pub level: LogLevel,
    /// Basename of the C source file that logged it.
    pub file: &'a str,
    /// Line within that file.
    pub line: i32,
    /// The formatted message, with no trailing newline.
    pub message: &'a str,
}

type LogFn = Box<dyn FnMut(Record<'_>) + Send>;

// The C callback is process-wide and carries no per-registration state, so the
// closure lives here rather than in a handle.
static SINK: Mutex<Option<LogFn>> = Mutex::new(None);

/// Sets the minimum severity delivered; anything less severe is dropped
/// before it is formatted. The library's default is [`LogLevel::Warning`].
pub fn set_level(level: LogLevel) {
    unsafe { sys::tpw_log_set_level(level.to_raw()) };
}

/// Routes the library's diagnostics to `callback`, replacing any callback set
/// before. With none set, messages go to stderr.
///
/// This is process-wide, and the callback may run on PipeWire's loop thread.
pub fn set_callback<F>(callback: F)
where
    F: FnMut(Record<'_>) + Send + 'static,
{
    *SINK.lock().unwrap() = Some(Box::new(callback));
    unsafe { sys::tpw_log_set_callback(Some(on_log), std::ptr::null_mut()) };
}

/// Clears the callback, sending diagnostics back to stderr.
pub fn clear_callback() {
    unsafe { sys::tpw_log_set_callback(None, std::ptr::null_mut()) };
    *SINK.lock().unwrap() = None;
}

unsafe extern "C" fn on_log(
    level: sys::tpw_log_level,
    file: *const c_char,
    line: c_int,
    message: *const c_char,
    _user_data: *mut c_void,
) {
    guard(|| {
        // A message logged from inside the callback would deadlock on a
        // blocking lock, so such a message is dropped instead.
        let Ok(mut slot) = SINK.try_lock() else {
            return;
        };
        let Some(callback) = slot.as_mut() else {
            return;
        };
        callback(Record {
            level: LogLevel::from_raw(level),
            file: borrow(file),
            line,
            message: borrow(message),
        });
    });
}

unsafe fn borrow<'a>(s: *const c_char) -> &'a str {
    if s.is_null() {
        return "";
    }
    CStr::from_ptr(s).to_str().unwrap_or_default()
}
