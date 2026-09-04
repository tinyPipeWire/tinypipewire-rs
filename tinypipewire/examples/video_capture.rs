//! Captures from the default camera and prints each frame's size.

use std::time::Duration;

use tinypipewire::{PixelFormat, Stream, VideoConfig};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let stream = Stream::video_capture(|buf| {
        println!(
            "frame: {} bytes (pts={:?} ns)",
            buf.data().map_or(0, <[u8]>::len),
            buf.pts()
        );
    })?;

    stream.set_video_config(&VideoConfig::new(640, 480, PixelFormat::Yuyv).with_fps(30))?;
    stream.start()?;
    std::thread::sleep(Duration::from_secs(5));
    stream.stop(false)?;
    Ok(())
}
