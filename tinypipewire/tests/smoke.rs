//! Checks that need a live PipeWire daemon with a null sink to talk to.
//!
//! They are ignored by default so `cargo test` stays runnable anywhere; CI
//! creates the sink and then runs them with `--ignored`. Set `TPW_SMOKE_SINK`
//! to point them at a different node.
//!
//! What these cover that the daemon-free tests cannot: the callback
//! trampolines actually firing, the target list coming back with real
//! entries, and a handle being dropped while its loop thread still runs.
//!
//! Routing here goes through the session manager, the path an application
//! normally takes. Manual routing — autoconnect off, then `link()` — is
//! covered only by the C library's hardware suite, and a stream's own ports
//! never appear in a bare headless graph for it to link.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tinypipewire::{AudioConfig, Filter, PortDirection, SampleFormat, Stream, StreamType};

/// How long to wait for the graph to start handing out cycles.
const DEADLINE: Duration = Duration::from_secs(5);

fn sink() -> String {
    std::env::var("TPW_SMOKE_SINK").unwrap_or_else(|_| "tpw-smoke-sink".to_string())
}

/// Waits for `ready` to hold, or gives up after [`DEADLINE`].
fn wait_for(ready: impl Fn() -> bool) {
    let deadline = Instant::now() + DEADLINE;
    while !ready() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(25));
    }
}

#[test]
#[ignore = "needs a running PipeWire daemon with a null sink"]
fn the_target_list_carries_real_entries() {
    let sink = sink();
    let stream = Stream::playback(|_| {}).expect("a playback stream needs a daemon");

    let targets = stream.targets();
    let found = targets
        .iter()
        .find(|target| target.name == sink)
        .unwrap_or_else(|| panic!("{sink} is missing from {targets:?}"));

    // An empty list would pass a mere "did not crash" check, so assert the
    // fields the C struct actually filled in.
    assert!(
        found.serial.parse::<u64>().is_ok(),
        "serial {:?} is not the decimal object.serial the C API promises",
        found.serial
    );
}

#[test]
#[ignore = "needs a running PipeWire daemon with a null sink"]
fn the_playback_callback_runs_and_its_writes_are_taken() {
    let calls = Arc::new(AtomicUsize::new(0));
    let written = Arc::new(AtomicUsize::new(0));

    let (n, bytes) = (Arc::clone(&calls), Arc::clone(&written));
    let stream = Stream::playback(move |buf| {
        // No allocation here: the C API states this runs on the real-time
        // thread and must not allocate or block.
        let available = buf.available();
        buf.as_mut_slice().fill(0);
        buf.set_filled(available);
        n.fetch_add(1, Ordering::Relaxed);
        bytes.fetch_add(available, Ordering::Relaxed);
    })
    .expect("a playback stream needs a daemon");

    // The session manager does the wiring, so the target is a hint set
    // before the format, which is what connects the stream.
    stream.set_target(&sink()).expect("target the sink");
    stream
        .set_audio_config(&AudioConfig::new(48_000, 2).with_format(SampleFormat::F32))
        .expect("the sink is stereo f32");
    stream.start().expect("start");

    wait_for(|| calls.load(Ordering::Relaxed) > 0);
    stream.stop(true).expect("stop");

    let calls = calls.load(Ordering::Relaxed);
    assert!(calls > 0, "the playback callback never ran");
    assert!(
        written.load(Ordering::Relaxed) > 0,
        "the callback ran {calls} times but was never given a byte to write"
    );
}

#[test]
#[ignore = "needs a running PipeWire daemon with a null sink"]
fn a_running_stream_can_be_dropped_without_stopping() {
    let stream = Stream::audio_capture(|_| {}).expect("a stream needs a daemon");
    stream.set_autoconnect(false).expect("autoconnect off");
    stream
        .set_audio_config(&AudioConfig::new(48_000, 2))
        .expect("audio config");
    stream.start().expect("start");

    // Drop has to stop the stream and join the loop thread before the boxed
    // callback goes with it. A hang or a crash here is the whole point.
    drop(stream);
}

#[test]
#[ignore = "needs a running PipeWire daemon with a null sink"]
fn a_filter_takes_ports_and_starts() {
    let filter = Filter::new("tpw-smoke-filter", |_| {}).expect("a filter needs a daemon");
    let config = AudioConfig::new(48_000, 2).with_format(SampleFormat::F32);

    let input = filter
        .add_audio_port(PortDirection::Input, &config)
        .expect("input port");
    let output = filter
        .add_audio_port(PortDirection::Output, &config)
        .expect("output port");

    assert_ne!(input, output, "two ports must not share an identity");
    assert_eq!(input.kind(), Some(StreamType::Audio));
    assert_eq!(output.kind(), Some(StreamType::Audio));

    filter.start().expect("start");
    filter.stop(false).expect("stop");
}
