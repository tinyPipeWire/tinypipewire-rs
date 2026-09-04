//! Multi-port filters: several inputs and outputs processed together in one
//! graph cycle.

use std::cell::UnsafeCell;
use std::ffi::{c_int, c_void, CStr, CString};
use std::ptr::NonNull;
use std::slice;
use std::sync::Mutex;
use std::time::Duration;

use tinypipewire_sys as sys;

use crate::error::{check, Error, Result};
use crate::format::{
    AudioConfig, DmabufPlane, PortMemory, StreamType, VideoConfig, VideoFormatInfo,
};
use crate::util::{collect_list, guard, state, try_collect_list, with_cstr};

type ProcessFn = Box<dyn FnMut(&mut [PortBuffer]) + Send>;
type ErrorFn = Box<dyn FnMut(Option<Port>, Error) + Send>;

/// The closures the C library calls back into, kept alive for as long as the
/// filter handle is.
struct FilterState {
    // Only PipeWire's loop thread ever touches this, and only between
    // tpw_filter_start() and tpw_filter_destroy().
    process: UnsafeCell<ProcessFn>,
    error: Mutex<Option<ErrorFn>>,
}

/// Which way data flows through a port.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum PortDirection {
    /// Consumes data the graph delivers, or that was pushed into it.
    Input,
    /// Produces data for the graph, written from the processing callback.
    Output,
}

impl PortDirection {
    fn to_raw(self) -> sys::tpw_filter_port_direction {
        match self {
            PortDirection::Input => sys::TPW_FILTER_PORT_INPUT,
            PortDirection::Output => sys::TPW_FILTER_PORT_OUTPUT,
        }
    }
}

/// A filter with any number of input and output ports, processed together.
///
/// Ports are added before [`Filter::start`]. The processing callback is
/// registered at construction, so a port a callback needs to recognize has to
/// reach it through something shared — an `Arc`, a channel, or a `OnceLock`.
///
/// Dropping the filter stops it and releases every port behind it.
///
/// # Shared references
///
/// Every method takes `&self`, as on [`Stream`](crate::Stream) and for the
/// same reason: the C library locks PipeWire's thread loop inside its own
/// calls, so this is genuine interior mutability rather than a claim of
/// thread safety. `Filter` is `Send` but not `Sync`.
pub struct Filter {
    handle: sys::tpw_filter_h,
    // Boxed so the address handed to C stays put while the Filter moves.
    state: Box<FilterState>,
}

// As for Stream: the C library locks PipeWire's thread loop internally, so a
// handle may move between threads, but destruction is not serialized.
unsafe impl Send for Filter {}

impl Filter {
    /// Creates a filter named `name`.
    ///
    /// `callback` runs once per graph cycle on PipeWire's loop thread, with
    /// one entry per port.
    pub fn new<F>(name: &str, callback: F) -> Result<Self>
    where
        F: FnMut(&mut [PortBuffer]) + Send + 'static,
    {
        let state = Box::new(FilterState {
            process: UnsafeCell::new(Box::new(callback)),
            error: Mutex::new(None),
        });
        let user_data = &*state as *const FilterState as *mut c_void;
        let handle = with_cstr(name, |name| unsafe {
            sys::tpw_filter_create(name.as_ptr(), Some(on_process), user_data)
        })?;
        if handle.is_null() {
            return Err(Error::CreateFailed);
        }
        Ok(Filter { handle, state })
    }

    /// Registers the callback invoked when a port's peer is lost.
    ///
    /// Set it before [`Filter::start`]; replacing it on a running filter
    /// blocks until the loop thread is between error reports.
    pub fn set_error_callback<F>(&self, callback: F) -> Result<()>
    where
        F: FnMut(Option<Port>, Error) + Send + 'static,
    {
        *self.state.error.lock().unwrap() = Some(Box::new(callback));
        check(unsafe { sys::tpw_filter_set_error_cb(self.handle, Some(on_error)) })
    }

    /// Clears the error callback.
    pub fn clear_error_callback(&self) -> Result<()> {
        check(unsafe { sys::tpw_filter_set_error_cb(self.handle, None) })?;
        *self.state.error.lock().unwrap() = None;
        Ok(())
    }

    /// Adds one audio port. Must be called before [`Filter::start`].
    pub fn add_audio_port(&self, direction: PortDirection, config: &AudioConfig) -> Result<Port> {
        let raw = config.to_raw();
        Port::new(unsafe { sys::tpw_filter_add_audio_port(self.handle, direction.to_raw(), &raw) })
    }

    /// Adds one video port. Must be called before [`Filter::start`].
    pub fn add_video_port(&self, direction: PortDirection, config: &VideoConfig) -> Result<Port> {
        let raw = config.to_raw();
        Port::new(unsafe { sys::tpw_filter_add_video_port(self.handle, direction.to_raw(), &raw) })
    }

    /// Adds one video port that negotiates a particular buffer memory type.
    ///
    /// With [`PortMemory::Dmabuf`] the port's buffers carry file descriptors,
    /// which [`PortBuffer::dmabuf_planes`] reads.
    pub fn add_video_port_with(
        &self,
        direction: PortDirection,
        config: &VideoConfig,
        memory: PortMemory,
    ) -> Result<Port> {
        let raw = config.to_raw();
        let opts = sys::tpw_filter_port_opts {
            memory: memory.to_raw(),
            reserved: [0; 2],
        };
        Port::new(unsafe {
            sys::tpw_filter_add_video_port_ex(self.handle, direction.to_raw(), &raw, &opts)
        })
    }

    /// Adds one signal port, which carries application-defined samples with
    /// no format negotiation.
    pub fn add_signal_port(&self, direction: PortDirection) -> Result<Port> {
        Port::new(unsafe { sys::tpw_filter_add_signal_port(self.handle, direction.to_raw()) })
    }

    /// Adds one event port, which carries timed events rather than a
    /// continuous stream.
    pub fn add_event_port(&self, direction: PortDirection) -> Result<Port> {
        Port::new(unsafe { sys::tpw_filter_add_event_port(self.handle, direction.to_raw()) })
    }

    /// Lists the video formats `target` can deliver to a video port of this
    /// filter, so a port can be added with a format the device really has.
    pub fn target_video_formats(&self, target: &str) -> Result<Vec<VideoFormatInfo>> {
        with_cstr(target, |target| unsafe {
            try_collect_list(
                32,
                |out, len, found| {
                    sys::tpw_filter_get_target_video_formats(
                        self.handle,
                        target.as_ptr(),
                        out,
                        len,
                        found,
                    )
                },
                VideoFormatInfo::from_raw,
            )
        })?
    }

    /// Asks the graph to run this filter at least once every `max_period`,
    /// for filters whose slowest input would otherwise set the pace.
    pub fn set_period_hint(&self, max_period: Duration) -> Result<()> {
        let nanos = u32::try_from(max_period.as_nanos()).unwrap_or(u32::MAX);
        check(unsafe { sys::tpw_filter_set_period_hint(self.handle, nanos) })
    }

    /// Stages `data` for `port` to receive on the next cycle, with no
    /// PipeWire-level connection involved.
    ///
    /// Only the most recently pushed buffer per port is kept. `pts` is carried
    /// through to that cycle's [`PortBuffer::pts`]. Event ports take
    /// [`Port::push_event`] instead.
    pub fn push_port_data(&self, port: Port, data: &[u8], pts: Option<i64>) -> Result<()> {
        check(unsafe {
            sys::tpw_filter_push_port_data(
                self.handle,
                port.as_raw(),
                data.as_ptr().cast::<c_void>(),
                data.len(),
                pts.unwrap_or(-1),
            )
        })
    }

    /// Starts the filter, after which the processing callback runs each cycle.
    pub fn start(&self) -> Result<()> {
        check(unsafe { sys::tpw_filter_start(self.handle) })
    }

    /// Stops the filter. With `drain` set, output ports first publish what
    /// they have already produced.
    pub fn stop(&self, drain: bool) -> Result<()> {
        check(unsafe { sys::tpw_filter_stop(self.handle, drain) })
    }

    /// The raw handle, for calls this binding does not cover.
    ///
    /// The handle stays owned by this `Filter` and must not be destroyed.
    pub fn as_raw(&self) -> sys::tpw_filter_h {
        self.handle
    }
}

impl Drop for Filter {
    fn drop(&mut self) {
        // Destroying joins the loop thread, so no callback can be running by
        // the time the boxed state goes with it.
        unsafe { sys::tpw_filter_destroy(self.handle) };
    }
}

impl std::fmt::Debug for Filter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Filter")
            .field("handle", &self.handle)
            .finish()
    }
}

/// One port of a filter.
///
/// A port is an identifier, not an owner: it stays valid as long as the
/// [`Filter`] that produced it, and is released with that filter. Because it
/// is `Copy` and `Send` it can be shared into the processing callback, which
/// is registered before any port exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Port(NonNull<sys::tpw_filter_port>);

// A Port is a bare identifier; every call it forwards to takes PipeWire's
// thread-loop lock inside the C library.
unsafe impl Send for Port {}
unsafe impl Sync for Port {}

impl Port {
    fn new(raw: sys::tpw_filter_port_h) -> Result<Self> {
        NonNull::new(raw).map(Port).ok_or(Error::CreateFailed)
    }

    /// What kind of data this port carries.
    pub fn kind(self) -> Option<StreamType> {
        StreamType::from_raw(unsafe { sys::tpw_filter_port_get_type(self.as_raw()) })
    }

    /// Keeps re-presenting the last buffer on cycles where no new one
    /// arrived, so a slow input does not read as empty next to a fast one.
    ///
    /// [`PortBuffer::is_fresh`] tells the two cases apart.
    pub fn set_hold(self, enable: bool) -> Result<()> {
        check(unsafe { sys::tpw_filter_port_set_hold(self.as_raw(), enable) })
    }

    /// Links this port to a node, by name or `object.serial`, with no session
    /// manager involved.
    pub fn link(self, target: &str) -> Result<()> {
        with_cstr(target, |target| unsafe {
            sys::tpw_filter_port_link(self.as_raw(), target.as_ptr())
        })
        .and_then(check)
    }

    /// Drops the links [`Port::link`] made.
    pub fn unlink(self) -> Result<()> {
        check(unsafe { sys::tpw_filter_port_unlink(self.as_raw()) })
    }

    /// Enqueues one event, whose data the library copies.
    ///
    /// On an output port this appends to the current cycle's outgoing events
    /// and is only valid from within the processing callback. On an input port
    /// it stages the event for the next cycle and may be called at any time.
    pub fn push_event(self, event: &Event<'_>) -> Result<()> {
        let key = match event.key {
            Some(key) => Some(CString::new(key).map_err(|_| Error::InvalidString)?),
            None => None,
        };
        let raw = sys::tpw_event {
            offset: event.offset,
            kind: event.kind.to_raw(),
            key: key.as_ref().map_or(std::ptr::null(), |k| k.as_ptr()),
            data: event.data.as_ptr().cast::<c_void>(),
            size: event.data.len(),
        };
        check(unsafe { sys::tpw_filter_port_push_event(self.as_raw(), &raw) })
    }

    /// The raw handle, for calls this binding does not cover.
    pub fn as_raw(self) -> sys::tpw_filter_port_h {
        self.0.as_ptr()
    }
}

/// One port's slot in a processing cycle.
///
/// The processing callback receives one of these per port, in the order the
/// ports were added.
#[repr(transparent)]
pub struct PortBuffer(sys::tpw_filter_port_buffer);

impl PortBuffer {
    /// Which port this entry describes.
    pub fn port(&self) -> Port {
        Port(NonNull::new(self.0.port).expect("the C library never reports a NULL port"))
    }

    /// The bytes an input port has to read this cycle, or `None` when no
    /// buffer was available or the port negotiated DMABUF.
    ///
    /// An output port's slot reads as an empty slice until the callback has
    /// filled it; use [`PortBuffer::output`] there.
    pub fn input(&self) -> Option<&[u8]> {
        if self.0.data.is_null() {
            None
        } else {
            Some(unsafe { slice::from_raw_parts(self.0.data.cast::<u8>(), self.0.size) })
        }
    }

    /// The region an output port may fill this cycle, or `None` when no
    /// buffer was available.
    ///
    /// The C library sets a capacity only on output ports, so an input port's
    /// slot always reads as `None` here rather than as an empty region.
    ///
    /// Writing here does not by itself publish anything; follow it with
    /// [`PortBuffer::set_filled`].
    pub fn output(&mut self) -> Option<&mut [u8]> {
        if self.0.data.is_null() || self.0.capacity == 0 {
            None
        } else {
            Some(unsafe { slice::from_raw_parts_mut(self.0.data.cast::<u8>(), self.0.capacity) })
        }
    }

    /// How many bytes [`PortBuffer::output`] can hold.
    pub fn capacity(&self) -> usize {
        self.0.capacity
    }

    /// Declares how many bytes an output port produced this cycle, clamped to
    /// the capacity. Zero publishes nothing.
    pub fn set_filled(&mut self, bytes: usize) {
        self.0.size = bytes.min(self.0.capacity);
    }

    /// Copies `src` into an output port's region and marks that many bytes
    /// produced, returning how many fit.
    pub fn write(&mut self, src: &[u8]) -> usize {
        let Some(out) = self.output() else { return 0 };
        let n = src.len().min(out.len());
        out[..n].copy_from_slice(&src[..n]);
        self.set_filled(n);
        n
    }

    /// An input port's timestamp in nanoseconds, or `None` if the source gave
    /// none. Always `None` on output and event ports.
    pub fn pts(&self) -> Option<i64> {
        (self.0.pts >= 0).then_some(self.0.pts)
    }

    /// True only when this buffer is new this cycle, so a held
    /// re-presentation reads as false.
    pub fn is_fresh(&self) -> bool {
        self.0.fresh
    }

    /// A per-port counter that advances only when new data arrives, so its
    /// deltas count genuinely new buffers.
    pub fn seq(&self) -> u64 {
        self.0.seq
    }

    /// The DMABUF planes behind this buffer, empty unless the port negotiated
    /// DMABUF. The file descriptors are borrowed for this cycle only.
    pub fn dmabuf_planes(&self) -> Vec<DmabufPlane> {
        unsafe {
            collect_list(
                4,
                |out, len| sys::tpw_filter_port_get_dmabuf_planes(&self.0, out, len),
                DmabufPlane::from_raw,
            )
        }
    }

    /// How many events an input event port received this cycle.
    pub fn event_count(&self) -> usize {
        unsafe { sys::tpw_filter_port_get_event_count(self.0.port) }
    }

    /// Reads this cycle's event at `index`, in delivery order.
    pub fn event(&self, index: usize) -> Result<Event<'_>> {
        let mut raw: sys::tpw_event = unsafe { std::mem::zeroed() };
        check(unsafe { sys::tpw_filter_port_get_event(self.0.port, index, &mut raw) })?;
        Ok(unsafe { Event::from_raw(&raw) })
    }

    /// Every event this input event port received this cycle.
    pub fn events(&self) -> impl Iterator<Item = Event<'_>> + '_ {
        (0..self.event_count()).filter_map(|i| self.event(i).ok())
    }
}

impl std::fmt::Debug for PortBuffer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PortBuffer")
            .field("size", &self.0.size)
            .field("capacity", &self.0.capacity)
            .field("pts", &self.pts())
            .field("fresh", &self.0.fresh)
            .field("seq", &self.0.seq)
            .finish()
    }
}

/// What an event on an event port carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum EventKind {
    /// Real MIDI wire bytes.
    Midi,
    /// Real OSC wire bytes.
    Osc,
    /// A general-purpose named value, selected by the event's key.
    Property,
    /// Read-only: an undecoded control item from another client.
    Unknown,
}

impl EventKind {
    fn to_raw(self) -> sys::tpw_event_kind {
        match self {
            EventKind::Midi => sys::TPW_EVENT_MIDI,
            EventKind::Osc => sys::TPW_EVENT_OSC,
            EventKind::Property => sys::TPW_EVENT_PROPERTY,
            EventKind::Unknown => sys::TPW_EVENT_UNKNOWN,
        }
    }

    fn from_raw(raw: sys::tpw_event_kind) -> Self {
        match raw {
            sys::TPW_EVENT_MIDI => EventKind::Midi,
            sys::TPW_EVENT_OSC => EventKind::Osc,
            sys::TPW_EVENT_PROPERTY => EventKind::Property,
            _ => EventKind::Unknown,
        }
    }
}

/// One event on an event port.
///
/// A read event borrows the cycle's buffer, so it must not outlive the
/// processing callback it came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Event<'a> {
    /// This event's position within the current cycle, in frames.
    pub offset: u32,
    /// What the event carries.
    pub kind: EventKind,
    /// The property name, set only for [`EventKind::Property`].
    pub key: Option<&'a str>,
    /// The event's bytes: MIDI or OSC wire format, or a property value.
    pub data: &'a [u8],
}

impl<'a> Event<'a> {
    /// A MIDI event of raw wire bytes.
    pub fn midi(offset: u32, data: &'a [u8]) -> Self {
        Event {
            offset,
            kind: EventKind::Midi,
            key: None,
            data,
        }
    }

    /// An OSC event of raw wire bytes.
    pub fn osc(offset: u32, data: &'a [u8]) -> Self {
        Event {
            offset,
            kind: EventKind::Osc,
            key: None,
            data,
        }
    }

    /// A named property value.
    pub fn property(offset: u32, key: &'a str, data: &'a [u8]) -> Self {
        Event {
            offset,
            kind: EventKind::Property,
            key: Some(key),
            data,
        }
    }

    /// # Safety
    /// `raw`'s pointers must be valid, which they are for the length of the
    /// processing callback that filled it.
    unsafe fn from_raw(raw: &sys::tpw_event) -> Self {
        Event {
            offset: raw.offset,
            kind: EventKind::from_raw(raw.kind),
            key: raw
                .key
                .as_ref()
                .and_then(|key| CStr::from_ptr(key).to_str().ok()),
            data: if raw.data.is_null() {
                &[]
            } else {
                slice::from_raw_parts(raw.data.cast::<u8>(), raw.size)
            },
        }
    }
}

unsafe extern "C" fn on_process(
    _filter: sys::tpw_filter_h,
    buffers: *mut sys::tpw_filter_port_buffer,
    n_buffers: usize,
    user_data: *mut c_void,
) {
    guard(|| {
        let Some(state) = state::<FilterState>(user_data) else {
            return;
        };
        // PortBuffer is repr(transparent) over the C struct, so the array the
        // library passes is already the slice the callback wants.
        let ports = if buffers.is_null() {
            &mut [][..]
        } else {
            slice::from_raw_parts_mut(buffers.cast::<PortBuffer>(), n_buffers)
        };
        (*state.process.get())(ports);
    });
}

unsafe extern "C" fn on_error(
    _filter: sys::tpw_filter_h,
    port: sys::tpw_filter_port_h,
    error_code: c_int,
    user_data: *mut c_void,
) {
    guard(|| {
        let Some(state) = state::<FilterState>(user_data) else {
            return;
        };
        if let Ok(mut slot) = state.error.lock() {
            if let Some(callback) = slot.as_mut() {
                callback(NonNull::new(port).map(Port), Error::from_code(error_code));
            }
        }
    });
}
