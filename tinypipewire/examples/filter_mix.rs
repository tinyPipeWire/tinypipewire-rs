//! Mixes two audio input ports into one audio output port.

use std::sync::mpsc;
use std::time::Duration;

use tinypipewire::{AudioConfig, Filter, PortDirection, SampleFormat};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // The processing callback is registered before any port exists, so the
    // ports reach it through a channel read on its first cycle.
    let (tx, rx) = mpsc::channel();
    let mut ports = None;

    let filter = Filter::new("rust-mixer", move |buffers| {
        // Nothing blocks the graph here: the ports are sent before start(), so
        // the first cycle already finds them waiting.
        if ports.is_none() {
            ports = rx.try_recv().ok();
        }
        let Some((a, b, out)) = &ports else { return };

        let mixed: Vec<f32> = mix(buffers, *a, *b);
        for buf in buffers.iter_mut() {
            if buf.port() == *out {
                let bytes: Vec<u8> = mixed.iter().flat_map(|s| s.to_le_bytes()).collect();
                buf.write(&bytes);
            }
        }
    })?;

    let config = AudioConfig::new(48_000, 2).with_format(SampleFormat::F32);
    let a = filter.add_audio_port(PortDirection::Input, &config)?;
    let b = filter.add_audio_port(PortDirection::Input, &config)?;
    let out = filter.add_audio_port(PortDirection::Output, &config)?;
    tx.send((a, b, out))?;

    filter.start()?;
    println!("mixing; connect two sources to rust-mixer with pw-link");
    std::thread::sleep(Duration::from_secs(30));
    filter.stop(false)?;
    Ok(())
}

/// Sums the samples of the two named input ports.
fn mix(
    buffers: &[tinypipewire::PortBuffer],
    a: tinypipewire::Port,
    b: tinypipewire::Port,
) -> Vec<f32> {
    let read = |port| {
        buffers
            .iter()
            .find(|buf| buf.port() == port)
            .and_then(|buf| buf.input())
            .map(samples)
            .unwrap_or_default()
    };

    let (left, right) = (read(a), read(b));
    let len = left.len().max(right.len());
    (0..len)
        .map(|i| left.get(i).copied().unwrap_or(0.0) + right.get(i).copied().unwrap_or(0.0))
        .collect()
}

fn samples(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes(c.try_into().expect("chunks_exact yields four bytes")))
        .collect()
}
