//! Plays a 440 Hz tone to the default output device, or to a sink named on
//! the command line.

use std::f32::consts::TAU;
use std::time::Duration;

use tinypipewire::{AudioConfig, Routing, SampleFormat, Stream};

const RATE: u32 = 48_000;
const CHANNELS: u32 = 2;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut phase = 0.0f32;
    let step = TAU * 440.0 / RATE as f32;

    let stream = Stream::playback(move |buf| {
        let frames = buf.available() / (CHANNELS as usize * 4);
        let mut samples = Vec::with_capacity(frames * CHANNELS as usize);
        for _ in 0..frames {
            let value = (phase.sin() * 0.2).to_le_bytes();
            phase = (phase + step) % TAU;
            for _ in 0..CHANNELS {
                samples.extend_from_slice(&value);
            }
        }
        buf.write(&samples);
    })?;

    if let Some(sink) = std::env::args().nth(1) {
        stream.set_routing(Routing::Autoconnect(Some(&sink)))?;
    }
    stream.set_audio_config(&AudioConfig::new(RATE, CHANNELS).with_format(SampleFormat::F32))?;
    stream.start()?;
    std::thread::sleep(Duration::from_secs(5));
    stream.stop(true)?;
    Ok(())
}
