//! Safe Rust bindings to [tinypipewire], a small C library that wraps
//! PipeWire's `pw_stream` for audio and video capture, audio playback, and
//! multi-port filters.
//!
//! There are two things to build with: a [`Stream`], which is one PipeWire
//! node carrying one media type, and a [`Filter`], which is one node with any
//! number of input and output ports processed together each cycle. Both hand
//! their buffers to a closure that runs on PipeWire's own thread.
//!
//! # Capturing audio
//!
//! ```no_run
//! use tinypipewire::{AudioConfig, Stream};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let stream = Stream::audio_capture(|buf| {
//!     if let Some(data) = buf.data() {
//!         println!("{} bytes (pts={:?})", data.len(), buf.pts());
//!     }
//! })?;
//!
//! stream.set_audio_config(&AudioConfig::new(48_000, 2))?;
//! stream.start()?;
//! std::thread::sleep(std::time::Duration::from_secs(5));
//! stream.stop(false)?;
//! # Ok(())
//! # }
//! ```
//!
//! Use [`Stream::video_capture`] with [`Stream::set_video_config`] to capture
//! from a camera instead; everything else is the same.
//!
//! # Playing audio
//!
//! A playback stream is filled rather than drained: its callback writes the
//! next block of samples each cycle.
//!
//! ```no_run
//! use tinypipewire::{AudioConfig, Stream};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let stream = Stream::playback(|buf| {
//!     let silence = vec![0u8; buf.available()];
//!     buf.write(&silence);
//! })?;
//!
//! stream.set_audio_config(&AudioConfig::new(48_000, 2))?;
//! stream.start()?;
//! # Ok(())
//! # }
//! ```
//!
//! # Threads and lifetimes
//!
//! Every callback runs on the PipeWire thread loop the handle owns, so each
//! closure is `Send + 'static`. The buffers passed in borrow the graph's
//! memory for the length of one call: copy out anything that has to outlive
//! it. Dropping a [`Stream`] or [`Filter`] stops it and joins that thread, so
//! no callback can still be running afterwards.
//!
//! Handle methods take `&self` even when they change the handle, because the
//! C library locks PipeWire's thread loop inside each call. That is interior
//! mutability, not a promise of thread safety: both handles are `Send` and
//! neither is `Sync`, so sharing one between threads does not compile.
//!
//! # Linking
//!
//! The `tinypipewire-sys` crate finds the C library through `pkg-config`, and
//! builds the copy tinypipewire-sys ships when that fails or when the
//! `vendored` feature is on.
//!
//! [tinypipewire]: https://github.com/tinyPipeWire/tinypipewire

#![warn(missing_docs)]

// Every `unsafe` block below is one call into the C library with a handle this
// crate created and still owns, so the invariant is stated here once.

mod error;
mod filter;
mod format;
mod stream;
mod util;

pub mod log;

pub use error::{Error, Result};
pub use filter::{Event, EventKind, Filter, Port, PortBuffer, PortDirection};
pub use format::{
    AudioConfig, DmabufPlane, PixelFormat, PortMemory, Routing, SampleFormat, StreamType,
    TargetInfo, VideoConfig, VideoFormatInfo,
};
pub use stream::{CaptureBuffer, PlaybackBuffer, Stream};

/// The raw FFI bindings this crate is built on.
pub use tinypipewire_sys as sys;

/// The version of the C API these bindings were generated against.
pub const C_API_VERSION: (u32, u32, u32) = (
    sys::TPW_VERSION_MAJOR,
    sys::TPW_VERSION_MINOR,
    sys::TPW_VERSION_PATCH,
);
