# tinypipewire

Safe Rust bindings to [tinypipewire], a small C library that wraps PipeWire's
`pw_stream` for audio and video capture, audio playback, and multi-port
filters. It hides the thread loop, SPA POD format negotiation, and buffer
dequeue/queue plumbing behind owned handles that return `Result`.

PipeWire is Linux-only, so this crate builds and runs there.

```toml
[dependencies]
tinypipewire = "0.1"
```

Capture from the default microphone for five seconds:

```rust
use tinypipewire::{AudioConfig, Stream, StreamType};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let stream = Stream::new_capture(StreamType::Audio, |buf| {
        if let Some(data) = buf.data() {
            println!("{} bytes (pts={:?} ns)", data.len(), buf.pts());
        }
    })?;

    stream.set_audio_config(&AudioConfig::new(48_000, 2))?;
    stream.start()?;
    std::thread::sleep(std::time::Duration::from_secs(5));
    stream.stop(false)?;
    Ok(())
}
```

Every callback runs on the PipeWire thread loop the handle owns, so each
closure is `Send + 'static`, and the buffers it receives borrow the graph's
memory for one call only. Dropping a `Stream` or `Filter` stops it and joins
that thread.

## Building

The C library comes from one of two places. By default `tinypipewire-sys`
probes pkg-config for an installed `tinypipewire` >= 0.9.0; the `vendored`
feature builds the C sources the `-sys` crate ships, which needs Meson, Ninja
and `libpipewire-0.3` >= 0.3.50 development files.

```sh
sudo apt-get install meson ninja-build pkg-config libpipewire-0.3-dev libspa-0.2-dev
cargo build --features vendored
```

## License

MIT, matching the C library.

[tinypipewire]: https://github.com/tinyPipeWire/tinypipewire
