//! Captures from the default audio source and prints each buffer's size.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tinypipewire::{AudioConfig, Stream};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let lost = Arc::new(AtomicBool::new(false));

    let stream = Stream::audio_capture(|buf| {
        if let Some(data) = buf.data() {
            println!("{} bytes (pts={:?} ns)", data.len(), buf.pts());
        }
    })?;

    let flag = Arc::clone(&lost);
    stream.set_error_callback(move |err| {
        eprintln!("stream error: {err}");
        flag.store(true, Ordering::Relaxed);
    })?;

    stream.set_audio_config(&AudioConfig::new(48_000, 2))?;
    stream.start()?;

    while !lost.load(Ordering::Relaxed) {
        std::thread::sleep(Duration::from_millis(200));
    }

    stream.stop(false)?;
    Ok(())
}
