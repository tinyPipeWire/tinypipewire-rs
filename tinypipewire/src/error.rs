use std::ffi::c_int;
use std::fmt;

use tinypipewire_sys as sys;

/// The result of a fallible tinypipewire call.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Everything a tinypipewire call can go wrong with.
///
/// The first five variants are the C library's `tpw_stream_error` codes, which
/// every entry point in the C API shares. The rest are raised by this binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Error {
    /// A NULL or out-of-range argument, or a call that is invalid for the
    /// object's current state or routing mode.
    InvalidArgument,
    /// Connecting to PipeWire, or negotiating a link or format, failed or
    /// timed out.
    ConnectFailed,
    /// An unrecognized pixel or sample format, or an out-of-range dimension
    /// or rate.
    InvalidFormat,
    /// A required earlier step was skipped, such as starting before a format
    /// was set, or linking before starting.
    NotConfigured,
    /// The connected source disappeared, or could not provide the requested
    /// memory type.
    SourceUnavailable,
    /// The C library refused to create the object and gave no reason.
    CreateFailed,
    /// A string argument held an interior NUL byte, so it could not cross the
    /// FFI boundary.
    InvalidString,
    /// An error code this binding does not know, from a newer C library.
    Unknown(i32),
}

impl Error {
    pub(crate) fn from_code(code: c_int) -> Self {
        match code {
            sys::TPW_STREAM_ERR_INVALID_ARG => Error::InvalidArgument,
            sys::TPW_STREAM_ERR_CONNECT_FAILED => Error::ConnectFailed,
            sys::TPW_STREAM_ERR_INVALID_FORMAT => Error::InvalidFormat,
            sys::TPW_STREAM_ERR_NOT_CONFIGURED => Error::NotConfigured,
            sys::TPW_STREAM_ERR_SOURCE_UNAVAILABLE => Error::SourceUnavailable,
            other => Error::Unknown(other),
        }
    }

    /// The `tpw_stream_error` code behind this error, or `None` for the
    /// variants this binding raises on its own.
    pub fn code(self) -> Option<i32> {
        match self {
            Error::InvalidArgument => Some(sys::TPW_STREAM_ERR_INVALID_ARG),
            Error::ConnectFailed => Some(sys::TPW_STREAM_ERR_CONNECT_FAILED),
            Error::InvalidFormat => Some(sys::TPW_STREAM_ERR_INVALID_FORMAT),
            Error::NotConfigured => Some(sys::TPW_STREAM_ERR_NOT_CONFIGURED),
            Error::SourceUnavailable => Some(sys::TPW_STREAM_ERR_SOURCE_UNAVAILABLE),
            Error::Unknown(code) => Some(code),
            Error::CreateFailed | Error::InvalidString => None,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Error::InvalidArgument => "invalid argument, or a call invalid in this state",
            Error::ConnectFailed => "connecting to PipeWire or negotiating a format failed",
            Error::InvalidFormat => "unrecognized format, or an out-of-range dimension or rate",
            Error::NotConfigured => "a required earlier configuration step was skipped",
            Error::SourceUnavailable => "the source disappeared or cannot supply this memory type",
            Error::CreateFailed => "the C library could not create the object",
            Error::InvalidString => "a string argument held an interior NUL byte",
            Error::Unknown(code) => return write!(f, "unknown tinypipewire error {code}"),
        };
        f.write_str(text)
    }
}

impl std::error::Error for Error {}

/// Turns a C return code into a `Result`.
pub(crate) fn check(code: c_int) -> Result<()> {
    if code == sys::TPW_STREAM_OK {
        Ok(())
    } else {
        Err(Error::from_code(code))
    }
}
