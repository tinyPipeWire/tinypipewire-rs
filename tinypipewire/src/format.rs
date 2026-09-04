//! Formats, configuration structs, and the descriptions a device reports.

use std::ffi::CStr;
use std::os::fd::RawFd;

use tinypipewire_sys as sys;

/// What kind of data a stream or filter port carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum StreamType {
    /// Raw audio samples.
    Audio,
    /// Raw video frames.
    Video,
    /// A filter-only port of application-defined samples.
    Signal,
    /// A filter-only port of timed events such as MIDI.
    Event,
}

impl StreamType {
    pub(crate) fn to_raw(self) -> sys::tpw_stream_type {
        match self {
            StreamType::Audio => sys::TPW_STREAM_TYPE_AUDIO,
            StreamType::Video => sys::TPW_STREAM_TYPE_VIDEO,
            StreamType::Signal => sys::TPW_STREAM_TYPE_SIGNAL,
            StreamType::Event => sys::TPW_STREAM_TYPE_EVENT,
        }
    }

    pub(crate) fn from_raw(raw: sys::tpw_stream_type) -> Option<Self> {
        match raw {
            sys::TPW_STREAM_TYPE_AUDIO => Some(StreamType::Audio),
            sys::TPW_STREAM_TYPE_VIDEO => Some(StreamType::Video),
            sys::TPW_STREAM_TYPE_SIGNAL => Some(StreamType::Signal),
            sys::TPW_STREAM_TYPE_EVENT => Some(StreamType::Event),
            _ => None,
        }
    }
}

/// A sample format the C library accepts for audio.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum SampleFormat {
    /// Unsigned 8-bit.
    U8,
    /// Signed 16-bit, the C library's default.
    #[default]
    S16,
    /// Signed 24-bit, packed into three bytes.
    S24,
    /// Signed 24-bit, padded into four bytes.
    S24In32,
    /// Signed 32-bit.
    S32,
    /// 32-bit float.
    F32,
}

impl SampleFormat {
    pub(crate) fn as_cstr(self) -> &'static CStr {
        match self {
            SampleFormat::U8 => c"U8",
            SampleFormat::S16 => c"S16",
            SampleFormat::S24 => c"S24",
            SampleFormat::S24In32 => c"S24_32",
            SampleFormat::S32 => c"S32",
            SampleFormat::F32 => c"F32",
        }
    }

    /// The name the C API uses for this format.
    pub fn as_str(self) -> &'static str {
        self.as_cstr().to_str().expect("format names are ASCII")
    }
}

/// A pixel format the C library accepts for video.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum PixelFormat {
    /// Packed 24-bit RGB.
    Rgb,
    /// Packed 4:2:2 YUV.
    Yuyv,
    /// Semi-planar 4:2:0 YUV, chroma as Cb/Cr.
    Nv12,
    /// Semi-planar 4:2:0 YUV, chroma as Cr/Cb.
    Nv21,
    /// Planar 4:2:0 YUV.
    I420,
    /// Motion JPEG; frame sizes vary and DMABUF delivery is unavailable.
    Mjpg,
    /// H.264; frame sizes vary and DMABUF delivery is unavailable.
    H264,
}

impl PixelFormat {
    pub(crate) fn as_cstr(self) -> &'static CStr {
        match self {
            PixelFormat::Rgb => c"RGB",
            PixelFormat::Yuyv => c"YUYV",
            PixelFormat::Nv12 => c"NV12",
            PixelFormat::Nv21 => c"NV21",
            PixelFormat::I420 => c"I420",
            PixelFormat::Mjpg => c"MJPG",
            PixelFormat::H264 => c"H264",
        }
    }

    pub(crate) fn from_cstr(name: &CStr) -> Option<Self> {
        Some(match name.to_bytes() {
            b"RGB" => PixelFormat::Rgb,
            b"YUYV" => PixelFormat::Yuyv,
            b"NV12" => PixelFormat::Nv12,
            b"NV21" => PixelFormat::Nv21,
            b"I420" => PixelFormat::I420,
            b"MJPG" => PixelFormat::Mjpg,
            b"H264" => PixelFormat::H264,
            _ => return None,
        })
    }

    /// The name the C API uses for this format.
    pub fn as_str(self) -> &'static str {
        self.as_cstr().to_str().expect("format names are ASCII")
    }

    /// True for formats whose frames are compressed, so each one has its own
    /// size and none can arrive as a DMABUF.
    pub fn is_compressed(self) -> bool {
        matches!(self, PixelFormat::Mjpg | PixelFormat::H264)
    }
}

/// The audio format to negotiate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AudioConfig {
    /// Sample rate in Hz, e.g. 48000.
    pub sample_rate: i32,
    /// Channel count, e.g. 2.
    pub channels: i32,
    /// Sample format.
    pub format: SampleFormat,
}

impl AudioConfig {
    /// A config for `sample_rate` and `channels` in the default `S16` format.
    pub fn new(sample_rate: i32, channels: i32) -> Self {
        AudioConfig {
            sample_rate,
            channels,
            format: SampleFormat::S16,
        }
    }

    /// Sets the sample format.
    pub fn with_format(mut self, format: SampleFormat) -> Self {
        self.format = format;
        self
    }

    pub(crate) fn to_raw(self) -> sys::tpw_audio_config {
        sys::tpw_audio_config {
            sample_rate: self.sample_rate,
            channels: self.channels,
            format: self.format.as_cstr().as_ptr(),
        }
    }
}

/// The video format to negotiate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct VideoConfig {
    /// Frame width in pixels.
    pub width: i32,
    /// Frame height in pixels.
    pub height: i32,
    /// Pixel format.
    pub pixel_format: PixelFormat,
    /// Frames per second; 0 negotiates automatically.
    pub fps: i32,
}

impl VideoConfig {
    /// A config for this size and pixel format, letting the frame rate be
    /// negotiated.
    pub fn new(width: i32, height: i32, pixel_format: PixelFormat) -> Self {
        VideoConfig {
            width,
            height,
            pixel_format,
            fps: 0,
        }
    }

    /// Asks for a specific frame rate instead of a negotiated one.
    pub fn with_fps(mut self, fps: i32) -> Self {
        self.fps = fps;
        self
    }

    pub(crate) fn to_raw(self) -> sys::tpw_video_config {
        sys::tpw_video_config {
            width: self.width,
            height: self.height,
            pixel_format: self.pixel_format.as_cstr().as_ptr(),
            fps: self.fps,
        }
    }
}

/// Which memory a video stream or port should negotiate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum PortMemory {
    /// Let the graph choose, which normally means CPU-mapped buffers.
    #[default]
    Auto,
    /// Negotiate DMABUF file descriptors.
    Dmabuf,
}

impl PortMemory {
    pub(crate) fn to_raw(self) -> sys::tpw_port_memory {
        match self {
            PortMemory::Auto => sys::TPW_PORT_MEMORY_AUTO,
            PortMemory::Dmabuf => sys::TPW_PORT_MEMORY_DMABUF,
        }
    }
}

/// One node a stream or port can be pointed at.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TargetInfo {
    /// Node name, usable directly as a target string.
    pub name: String,
    /// The node's `object.serial` as decimal digits, also usable as a target
    /// string, and unique where two nodes share a name.
    pub serial: String,
    /// Human-readable `node.description`, or empty if the node set none.
    pub description: String,
}

impl TargetInfo {
    pub(crate) fn from_raw(raw: &sys::tpw_target_info) -> Self {
        TargetInfo {
            name: fixed_str(&raw.name),
            serial: fixed_str(&raw.serial),
            description: fixed_str(&raw.description),
        }
    }
}

/// One video format a device reports, in a form that goes straight into a
/// [`VideoConfig`].
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct VideoFormatInfo {
    /// The pixel format, or `None` if the device named one this binding does
    /// not know.
    pub pixel_format: Option<PixelFormat>,
    /// Frame width, or the smallest one for a size range.
    pub width: i32,
    /// Frame height, or the smallest one for a size range.
    pub height: i32,
    /// Equal to `width` for a discrete size, the range's largest otherwise.
    pub width_max: i32,
    /// Equal to `height` for a discrete size, the range's largest otherwise.
    pub height_max: i32,
    /// Whole frames per second at this size, highest first.
    pub fps: Vec<i32>,
}

impl VideoFormatInfo {
    pub(crate) fn from_raw(raw: &sys::tpw_video_format_info) -> Self {
        let n_fps = raw.n_fps.min(raw.fps.len());
        VideoFormatInfo {
            pixel_format: CStr::from_bytes_until_nul(bytes_of(&raw.pixel_format))
                .ok()
                .and_then(PixelFormat::from_cstr),
            width: raw.width,
            height: raw.height,
            width_max: raw.width_max,
            height_max: raw.height_max,
            fps: raw.fps[..n_fps].to_vec(),
        }
    }

    /// True when the device reported a size range rather than one fixed size.
    pub fn is_size_range(&self) -> bool {
        self.width != self.width_max || self.height != self.height_max
    }

    /// A config for this entry's smallest size and fastest frame rate, or
    /// `None` if the pixel format was unrecognized.
    pub fn to_config(&self) -> Option<VideoConfig> {
        Some(VideoConfig {
            width: self.width,
            height: self.height,
            pixel_format: self.pixel_format?,
            fps: self.fps.first().copied().unwrap_or(0),
        })
    }
}

/// One plane of a DMABUF frame.
///
/// The file descriptor is borrowed from the buffer and is only valid while the
/// callback that produced this plane is running; do not close it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DmabufPlane {
    /// Borrowed DMABUF file descriptor.
    pub fd: RawFd,
    /// Byte offset to the plane within the dmabuf.
    pub offset: u32,
    /// Row stride in bytes.
    pub stride: u32,
    /// Valid bytes of this plane.
    pub size: u32,
}

impl DmabufPlane {
    pub(crate) fn from_raw(raw: &sys::tpw_dmabuf_plane) -> Self {
        DmabufPlane {
            fd: raw.fd,
            offset: raw.offset,
            stride: raw.stride,
            size: raw.size,
        }
    }
}

fn bytes_of(chars: &[std::ffi::c_char]) -> &[u8] {
    // c_char is i8 on every platform PipeWire runs on; the layouts match.
    unsafe { std::slice::from_raw_parts(chars.as_ptr().cast::<u8>(), chars.len()) }
}

/// Reads a NUL-terminated C string out of a fixed-size struct field.
fn fixed_str(chars: &[std::ffi::c_char]) -> String {
    CStr::from_bytes_until_nul(bytes_of(chars))
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}
