//! Audio and video capture, and audio playback, over one PipeWire node.

use std::cell::UnsafeCell;
use std::ffi::{c_int, c_void};
use std::slice;
use std::sync::Mutex;

use tinypipewire_sys as sys;

use crate::error::{check, Error, Result};
use crate::format::{
    AudioConfig, DmabufPlane, PortMemory, Routing, StreamType, TargetInfo, VideoConfig,
    VideoFormatInfo,
};
use crate::util::{collect_list, guard, state, try_collect_list, with_cstr};

type CaptureFn = Box<dyn FnMut(CaptureBuffer<'_>) + Send>;
type PlaybackFn = Box<dyn FnMut(&mut PlaybackBuffer<'_>) + Send>;
type ErrorFn = Box<dyn FnMut(Error) + Send>;

enum Delivery {
    Capture(CaptureFn),
    Playback(PlaybackFn),
}

/// The closures the C library calls back into, kept alive for as long as the
/// stream handle is.
struct StreamState {
    // Only PipeWire's loop thread ever touches this, and only between
    // tpw_stream_start() and tpw_stream_destroy().
    delivery: UnsafeCell<Delivery>,
    error: Mutex<Option<ErrorFn>>,
}

/// A capture or playback stream.
///
/// Created with [`Stream::audio_capture`], [`Stream::video_capture`] or
/// [`Stream::playback`], then
/// configured with a format — and optionally a target — before
/// [`Stream::start`]. The buffer callback runs on PipeWire's own thread, so
/// the closure it was built with must be `Send`.
///
/// Dropping the stream stops it and releases every resource behind it.
///
/// # Shared references
///
/// Every method takes `&self`, including the ones that change the stream.
/// That is not a claim of thread safety: the C library takes PipeWire's
/// thread-loop lock inside each of its own calls, which makes this genuine
/// interior mutability, and `Stream` is deliberately not `Sync`, so the
/// compiler still refuses to share one across threads. It is `Send`, so a
/// stream can be moved to another thread and driven from there.
pub struct Stream {
    handle: sys::tpw_stream_h,
    // Boxed so the address handed to C stays put while the Stream moves.
    state: Box<StreamState>,
}

// The C library takes PipeWire's thread-loop lock inside its own calls, so a
// handle may be moved between threads. It is not Sync: destruction is not
// serialized against the other entry points.
unsafe impl Send for Stream {}

impl Stream {
    /// Creates a stream that captures audio.
    ///
    /// `callback` is invoked with every delivered buffer once the stream has
    /// started, on PipeWire's loop thread.
    pub fn audio_capture<F>(callback: F) -> Result<Self>
    where
        F: FnMut(CaptureBuffer<'_>) + Send + 'static,
    {
        Self::capture(StreamType::Audio, callback)
    }

    /// Creates a stream that captures video.
    ///
    /// `callback` is invoked with every delivered frame once the stream has
    /// started, on PipeWire's loop thread.
    pub fn video_capture<F>(callback: F) -> Result<Self>
    where
        F: FnMut(CaptureBuffer<'_>) + Send + 'static,
    {
        Self::capture(StreamType::Video, callback)
    }

    /// Creates an audio playback stream, emitting to an output device.
    ///
    /// `callback` is invoked once per cycle to fill the next block of audio.
    /// Video playback has no C API: a node that emits video is a filter output
    /// port instead.
    pub fn playback<F>(callback: F) -> Result<Self>
    where
        F: FnMut(&mut PlaybackBuffer<'_>) + Send + 'static,
    {
        Self::create(Delivery::Playback(Box::new(callback)), |state| unsafe {
            sys::tpw_stream_create_playback(Some(on_playback), state)
        })
    }

    /// The two capture constructors differ only in the media type they ask
    /// for; the C API rejects the signal and event types here.
    fn capture<F>(kind: StreamType, callback: F) -> Result<Self>
    where
        F: FnMut(CaptureBuffer<'_>) + Send + 'static,
    {
        Self::create(Delivery::Capture(Box::new(callback)), |state| unsafe {
            sys::tpw_stream_create(kind.to_raw(), Some(on_capture), state)
        })
    }

    fn create(
        delivery: Delivery,
        create: impl FnOnce(*mut c_void) -> sys::tpw_stream_h,
    ) -> Result<Self> {
        let state = Box::new(StreamState {
            delivery: UnsafeCell::new(delivery),
            error: Mutex::new(None),
        });
        let handle = create(&*state as *const StreamState as *mut c_void);
        if handle.is_null() {
            return Err(Error::CreateFailed);
        }
        Ok(Stream { handle, state })
    }

    /// Registers the callback invoked when the stream's source is lost.
    ///
    /// Set it before [`Stream::start`]; replacing it on a running stream
    /// blocks until the loop thread is between error reports.
    pub fn set_error_callback<F>(&self, callback: F) -> Result<()>
    where
        F: FnMut(Error) + Send + 'static,
    {
        *self.state.error.lock().unwrap() = Some(Box::new(callback));
        check(unsafe { sys::tpw_stream_set_error_cb(self.handle, Some(on_error)) })
    }

    /// Clears the error callback.
    pub fn clear_error_callback(&self) -> Result<()> {
        check(unsafe { sys::tpw_stream_set_error_cb(self.handle, None) })?;
        *self.state.error.lock().unwrap() = None;
        Ok(())
    }

    /// Chooses how the stream reaches the graph.
    ///
    /// Must be called before the format, which is what actually connects the
    /// stream and fixes the routing mode; afterwards it is refused. Until
    /// then the choice can be changed freely, in either direction.
    pub fn set_routing(&self, routing: Routing<'_>) -> Result<()> {
        let (autoconnect, target) = match routing {
            Routing::Autoconnect(target) => (true, target),
            Routing::Manual => (false, None),
        };

        // Each variant sets the whole mode, not just the half its matching C
        // setter covers: a target left behind would still route the stream,
        // and the C library refuses to turn autoconnect off while one is set.
        check(unsafe { sys::tpw_stream_set_target(self.handle, std::ptr::null()) })?;
        check(unsafe { sys::tpw_stream_set_autoconnect(self.handle, autoconnect) })?;

        match target {
            Some(target) => with_cstr(target, |target| unsafe {
                sys::tpw_stream_set_target(self.handle, target.as_ptr())
            })
            .and_then(check),
            None => Ok(()),
        }
    }

    /// Lists every node [`Routing::Autoconnect`] would accept for this
    /// stream's media type.
    ///
    /// An empty list means the graph holds no such node; a graph that could
    /// not be reached is an error instead.
    pub fn targets(&self) -> Result<Vec<TargetInfo>> {
        unsafe {
            try_collect_list(
                16,
                |out, len, found| sys::tpw_stream_get_target_list(self.handle, out, len, found),
                TargetInfo::from_raw,
            )
        }
    }

    /// Lists the video formats `target` can deliver to this stream, or those
    /// of the target already set when `target` is `None`.
    ///
    /// Every entry is one [`Stream::set_video_config`] accepts for that
    /// target. Reading them opens the device briefly, unlike the free lookup
    /// [`Stream::targets`] does.
    pub fn target_video_formats(&self, target: Option<&str>) -> Result<Vec<VideoFormatInfo>> {
        let query = |name: *const std::ffi::c_char| unsafe {
            try_collect_list(
                32,
                |out, len, found| {
                    sys::tpw_stream_get_target_video_formats(self.handle, name, out, len, found)
                },
                VideoFormatInfo::from_raw,
            )
        };
        match target {
            Some(target) => with_cstr(target, |target| query(target.as_ptr()))?,
            None => query(std::ptr::null()),
        }
    }

    /// Links this stream's port to `target` by hand, with no session manager
    /// involved. Requires [`Routing::Manual`] and the stream started.
    pub fn link(&self, target: &str) -> Result<()> {
        with_cstr(target, |target| unsafe {
            sys::tpw_stream_link(self.handle, target.as_ptr())
        })
        .and_then(check)
    }

    /// Drops the links [`Stream::link`] made.
    pub fn unlink(&self) -> Result<()> {
        check(unsafe { sys::tpw_stream_unlink(self.handle) })
    }

    /// Sets the audio format and connects the stream.
    pub fn set_audio_config(&self, config: &AudioConfig) -> Result<()> {
        let raw = config.to_raw();
        check(unsafe { sys::tpw_stream_set_audio_config(self.handle, &raw) })
    }

    /// Sets the video format and connects the stream.
    pub fn set_video_config(&self, config: &VideoConfig) -> Result<()> {
        let raw = config.to_raw();
        check(unsafe { sys::tpw_stream_set_video_config(self.handle, &raw) })
    }

    /// Sets the video format and asks for a particular buffer memory type.
    ///
    /// With [`PortMemory::Dmabuf`] the delivered buffers carry file
    /// descriptors instead of mapped bytes, which
    /// [`CaptureBuffer::dmabuf_planes`] reads.
    pub fn set_video_config_with(&self, config: &VideoConfig, memory: PortMemory) -> Result<()> {
        let raw = config.to_raw();
        let opts = sys::tpw_stream_dmabuf_opts {
            memory: memory.to_raw(),
            reserved: [0; 2],
        };
        check(unsafe { sys::tpw_stream_set_video_config_ex(self.handle, &raw, &opts) })
    }

    /// Starts the stream, after which the buffer callback runs each cycle.
    pub fn start(&self) -> Result<()> {
        check(unsafe { sys::tpw_stream_start(self.handle) })
    }

    /// Stops the stream. With `drain` set, a playback stream first plays out
    /// what it has already queued.
    pub fn stop(&self, drain: bool) -> Result<()> {
        check(unsafe { sys::tpw_stream_stop(self.handle, drain) })
    }

    /// The raw handle, for calls this binding does not cover.
    ///
    /// The handle stays owned by this `Stream` and must not be destroyed.
    pub fn as_raw(&self) -> sys::tpw_stream_h {
        self.handle
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        // Destroying joins the loop thread, so no callback can be running by
        // the time the boxed state goes with it.
        unsafe { sys::tpw_stream_destroy(self.handle) };
    }
}

impl std::fmt::Debug for Stream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Stream")
            .field("handle", &self.handle)
            .finish()
    }
}

/// One captured buffer, borrowed for the length of the callback.
pub struct CaptureBuffer<'a> {
    stream: sys::tpw_stream_h,
    raw: &'a sys::tpw_stream_buffer,
}

impl<'a> CaptureBuffer<'a> {
    /// The captured bytes, or `None` when the stream negotiated DMABUF and
    /// [`CaptureBuffer::dmabuf_planes`] holds the frame instead.
    pub fn data(&self) -> Option<&'a [u8]> {
        if self.raw.data.is_null() {
            None
        } else {
            Some(unsafe { slice::from_raw_parts(self.raw.data.cast::<u8>(), self.raw.size) })
        }
    }

    /// Capture timestamp in nanoseconds on the driver clock, or `None` if the
    /// buffer carried no timestamp.
    pub fn pts(&self) -> Option<i64> {
        (self.raw.pts >= 0).then_some(self.raw.pts)
    }

    /// The DMABUF planes of this frame, empty unless the stream negotiated
    /// DMABUF. The file descriptors are borrowed for this callback only.
    pub fn dmabuf_planes(&self) -> Vec<DmabufPlane> {
        unsafe {
            collect_list(
                4,
                |out, len| sys::tpw_stream_get_dmabuf_planes(self.stream, out, len),
                DmabufPlane::from_raw,
            )
        }
    }
}

impl std::fmt::Debug for CaptureBuffer<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CaptureBuffer")
            .field("size", &self.raw.size)
            .field("pts", &self.pts())
            .finish()
    }
}

/// The region a playback callback fills, borrowed for the length of the call.
pub struct PlaybackBuffer<'a> {
    raw: &'a mut sys::tpw_stream_playback_buffer,
}

impl PlaybackBuffer<'_> {
    /// Bytes the callback may write this cycle. This is what the device asked
    /// for, which is usually less than the region's capacity.
    pub fn available(&self) -> usize {
        self.raw.available
    }

    /// When this cycle's first sample is expected to be heard, in monotonic
    /// nanoseconds, or `None` if the graph cannot say.
    pub fn pts(&self) -> Option<i64> {
        (self.raw.pts >= 0).then_some(self.raw.pts)
    }

    /// The writable region, exactly [`PlaybackBuffer::available`] bytes long.
    ///
    /// Writing here does not by itself publish anything; follow it with
    /// [`PlaybackBuffer::set_filled`].
    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        unsafe { slice::from_raw_parts_mut(self.raw.data.cast::<u8>(), self.raw.available) }
    }

    /// Declares how many bytes were written. Clamped to
    /// [`PlaybackBuffer::available`] and floored to whole frames; the
    /// remainder is emitted as silence, so 0 plays a silent cycle rather than
    /// stopping the stream.
    pub fn set_filled(&mut self, bytes: usize) {
        self.raw.size = bytes.min(self.raw.available);
    }

    /// Copies `src` into the region and marks that many bytes filled,
    /// returning how many fit.
    pub fn write(&mut self, src: &[u8]) -> usize {
        let n = src.len().min(self.available());
        self.as_mut_slice()[..n].copy_from_slice(&src[..n]);
        self.set_filled(n);
        n
    }
}

impl std::fmt::Debug for PlaybackBuffer<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PlaybackBuffer")
            .field("available", &self.raw.available)
            .field("pts", &self.pts())
            .finish()
    }
}

unsafe extern "C" fn on_capture(
    stream: sys::tpw_stream_h,
    buf: *const sys::tpw_stream_buffer,
    user_data: *mut c_void,
) {
    guard(|| {
        let (Some(state), Some(raw)) = (state::<StreamState>(user_data), buf.as_ref()) else {
            return;
        };
        if let Delivery::Capture(callback) = &mut *state.delivery.get() {
            callback(CaptureBuffer { stream, raw });
        }
    });
}

unsafe extern "C" fn on_playback(
    _stream: sys::tpw_stream_h,
    buf: *mut sys::tpw_stream_playback_buffer,
    user_data: *mut c_void,
) {
    guard(|| {
        let (Some(state), Some(raw)) = (state::<StreamState>(user_data), buf.as_mut()) else {
            return;
        };
        if let Delivery::Playback(callback) = &mut *state.delivery.get() {
            callback(&mut PlaybackBuffer { raw });
        }
    });
}

unsafe extern "C" fn on_error(
    _stream: sys::tpw_stream_h,
    error_code: c_int,
    user_data: *mut c_void,
) {
    guard(|| {
        let Some(state) = state::<StreamState>(user_data) else {
            return;
        };
        if let Ok(mut slot) = state.error.lock() {
            if let Some(callback) = slot.as_mut() {
                callback(Error::from_code(error_code));
            }
        }
    });
}
