//! Checks that need no PipeWire daemon: pure conversions and error mapping.

use tinypipewire::{
    AudioConfig, Error, EventKind, PixelFormat, PortDirection, PortMemory, SampleFormat,
    StreamType, VideoConfig, C_API_VERSION,
};

#[test]
fn c_errors_carry_their_code_and_a_message() {
    let mapped = [
        (Error::InvalidArgument, -1),
        (Error::ConnectFailed, -2),
        (Error::InvalidFormat, -3),
        (Error::NotConfigured, -4),
        (Error::SourceUnavailable, -5),
    ];
    for (error, code) in mapped {
        assert_eq!(error.code(), Some(code));
        assert!(!error.to_string().is_empty());
    }
    assert_eq!(Error::Unknown(-99).code(), Some(-99));
    assert_eq!(Error::CreateFailed.code(), None);
    assert_eq!(Error::InvalidString.code(), None);
}

#[test]
fn format_names_match_the_c_api() {
    assert_eq!(SampleFormat::default(), SampleFormat::S16);
    assert_eq!(SampleFormat::S24In32.as_str(), "S24_32");
    assert_eq!(SampleFormat::F32.as_str(), "F32");
    assert_eq!(PixelFormat::Yuyv.as_str(), "YUYV");
    assert_eq!(PixelFormat::H264.as_str(), "H264");
    assert!(PixelFormat::Mjpg.is_compressed());
    assert!(!PixelFormat::I420.is_compressed());
}

#[test]
fn config_builders_keep_what_they_are_given() {
    let audio = AudioConfig::new(44_100, 1).with_format(SampleFormat::F32);
    assert_eq!(audio.sample_rate, 44_100);
    assert_eq!(audio.channels, 1);
    assert_eq!(audio.format, SampleFormat::F32);
    assert_eq!(AudioConfig::new(48_000, 2).format, SampleFormat::S16);

    let video = VideoConfig::new(1280, 720, PixelFormat::Nv12).with_fps(30);
    assert_eq!((video.width, video.height), (1280, 720));
    assert_eq!(video.pixel_format, PixelFormat::Nv12);
    assert_eq!(video.fps, 30);
    assert_eq!(VideoConfig::new(640, 480, PixelFormat::Rgb).fps, 0);
}

#[test]
fn enum_variants_stay_distinct() {
    assert_ne!(StreamType::Audio, StreamType::Video);
    assert_ne!(StreamType::Signal, StreamType::Event);
    assert_ne!(PortDirection::Input, PortDirection::Output);
    assert_ne!(EventKind::Midi, EventKind::Property);
    assert_eq!(PortMemory::default(), PortMemory::Auto);
}

#[test]
fn the_bound_c_api_is_the_pinned_one() {
    assert_eq!(C_API_VERSION.0, 0);
    assert!(C_API_VERSION.1 >= 8);
}
