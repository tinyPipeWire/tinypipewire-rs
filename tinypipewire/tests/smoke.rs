//! Checks that need a live PipeWire daemon with a null sink to talk to.
//!
//! They are ignored by default so `cargo test` stays runnable anywhere; CI
//! creates the sink and then runs them with `--ignored`. Set `TPW_SMOKE_SINK`
//! to point them at a different node.
//!
//! What these cover that the daemon-free tests cannot: the callback
//! trampolines actually firing, the target list coming back with real
//! entries, a handle being dropped while its loop thread still runs, and the
//! calls the C library refuses from inside a callback.
//!
//! Routing here goes through the session manager, the path an application
//! normally takes. Manual routing — autoconnect off, then `link()` — is
//! covered only by the C library's hardware suite, and a stream's own ports
//! never appear in a bare headless graph for it to link.

use std::sync::atomic::{AtomicI32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tinypipewire::{
    AudioConfig, DataType, Error, Filter, PortDirection, Routing, SampleFormat, Stream,
};

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

    let targets = stream.targets().expect("the graph is reachable");
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
    stream
        .set_routing(Routing::Autoconnect(Some(&sink())))
        .expect("target the sink");
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

/// Each mode has to undo the other's half. Setting a target and then asking
/// for another mode used to leave that target in place — silently when
/// autoconnect stayed on, and as an error when it was turned off.
#[test]
#[ignore = "needs a running PipeWire daemon with a null sink"]
fn routing_can_be_changed_until_the_format_connects() {
    let stream = Stream::playback(|_| {}).expect("a playback stream needs a daemon");

    stream
        .set_routing(Routing::Autoconnect(Some(&sink())))
        .expect("target the sink");
    stream
        .set_routing(Routing::Manual)
        .expect("manual routing has to clear the target it replaces");
    stream
        .set_routing(Routing::Autoconnect(Some(&sink())))
        .expect("a target has to be settable again");
    stream
        .set_routing(Routing::Autoconnect(None))
        .expect("dropping the target has to clear the one it replaces");

    // With the target really gone, the default sink takes the stream, and the
    // callback runs at all only because something wired it.
    stream
        .set_audio_config(&AudioConfig::new(48_000, 2).with_format(SampleFormat::F32))
        .expect("the sink is stereo f32");
    stream.start().expect("start");
    stream.stop(false).expect("stop");

    // The mode is fixed once the format has connected the stream.
    assert!(stream.set_routing(Routing::Manual).is_err());
}

#[test]
#[ignore = "needs a running PipeWire daemon with a null sink"]
fn a_running_stream_can_be_dropped_without_stopping() {
    let stream = Stream::audio_capture(|_| {}).expect("a stream needs a daemon");
    stream.set_routing(Routing::Manual).expect("manual routing");
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
    assert_eq!(input.kind(), Some(DataType::Audio));
    assert_eq!(output.kind(), Some(DataType::Audio));

    filter.start().expect("start");
    filter.stop(false).expect("stop");
}

#[test]
#[ignore = "needs a running PipeWire daemon with a null sink"]
fn a_role_is_accepted_under_either_routing() {
    let stream = Stream::playback(|_| {}).expect("a playback stream needs a daemon");

    // A role is only a hint, so unlike a target it is not tied to autoconnect.
    stream.set_routing(Routing::Manual).expect("manual routing");
    stream
        .set_role(Some("Music"))
        .expect("a role under manual routing");
    stream.set_role(None).expect("clearing the role");
    stream
        .set_routing(Routing::Autoconnect(Some(&sink())))
        .expect("target the sink");
    stream
        .set_role(Some("Notification"))
        .expect("a role under autoconnect");

    stream
        .set_audio_config(&AudioConfig::new(48_000, 2).with_format(SampleFormat::F32))
        .expect("the sink is stereo f32");
    stream.start().expect("start");
    stream.stop(false).expect("stop");
}

#[test]
#[ignore = "needs a running PipeWire daemon with a null sink"]
fn unlinking_with_nothing_linked_is_not_configured() {
    let stream = Stream::audio_capture(|_| {}).expect("a stream needs a daemon");
    assert_eq!(stream.unlink(), Err(Error::NotConfigured));
}

#[test]
#[ignore = "needs a running PipeWire daemon with a null sink"]
fn a_filter_port_linked_to_a_missing_node_is_not_found() {
    let filter = Filter::new("tpw-smoke-link", |_| {}).expect("a filter needs a daemon");
    let input = filter
        .add_audio_port(
            PortDirection::Input,
            &AudioConfig::new(48_000, 2).with_format(SampleFormat::F32),
        )
        .expect("input port");

    assert_eq!(input.link(&sink()), Err(Error::NotConfigured));
    filter.start().expect("start");
    assert_eq!(input.link("tpw-smoke-no-such-node"), Err(Error::NotFound));
    filter.stop(false).expect("stop");
}

#[test]
#[ignore = "needs a running PipeWire daemon with a null sink"]
fn a_call_from_the_buffer_callback_is_refused() {
    let shared: Arc<Mutex<Option<Stream>>> = Arc::new(Mutex::new(None));
    let refused = Arc::new(AtomicI32::new(0));

    let (slot, code) = (Arc::clone(&shared), Arc::clone(&refused));
    let stream = Stream::playback(move |_| {
        if let Ok(guard) = slot.try_lock() {
            if let Some(Err(error)) = guard.as_ref().map(|stream| stream.stop(false)) {
                code.store(error.code().unwrap_or(0), Ordering::Relaxed);
            }
        }
    })
    .expect("a playback stream needs a daemon");
    stream
        .set_routing(Routing::Autoconnect(Some(&sink())))
        .expect("target the sink");
    stream
        .set_audio_config(&AudioConfig::new(48_000, 2).with_format(SampleFormat::F32))
        .expect("the sink is stereo f32");
    stream.start().expect("start");
    *shared.lock().unwrap() = Some(stream);

    wait_for(|| refused.load(Ordering::Relaxed) != 0);
    let stream = shared
        .lock()
        .unwrap()
        .take()
        .expect("the stream is still shared");
    stream
        .stop(false)
        .expect("a stop outside the callback still works");
    assert_eq!(
        refused.load(Ordering::Relaxed),
        Error::InCallback.code().unwrap(),
        "a stop from the data thread has to be refused"
    );
}

#[test]
#[ignore = "needs a running PipeWire daemon with a null sink"]
fn a_stream_dropped_from_its_own_callback_keeps_what_it_runs() {
    let shared: Arc<Mutex<Option<Stream>>> = Arc::new(Mutex::new(None));
    let dropped = Arc::new(AtomicUsize::new(0));
    let calls_after = Arc::new(AtomicUsize::new(0));

    let (slot, done, after) = (
        Arc::clone(&shared),
        Arc::clone(&dropped),
        Arc::clone(&calls_after),
    );
    let stream = Stream::playback(move |_| {
        if done.load(Ordering::Relaxed) > 0 {
            after.fetch_add(1, Ordering::Relaxed);
        } else if let Ok(mut guard) = slot.try_lock() {
            if let Some(stream) = guard.take() {
                drop(stream);
                done.store(1, Ordering::Relaxed);
            }
        }
    })
    .expect("a playback stream needs a daemon");
    stream
        .set_routing(Routing::Autoconnect(Some(&sink())))
        .expect("target the sink");
    stream
        .set_audio_config(&AudioConfig::new(48_000, 2).with_format(SampleFormat::F32))
        .expect("the sink is stereo f32");
    stream.start().expect("start");
    *shared.lock().unwrap() = Some(stream);

    // The C library leaves the stream running, so the callback keeps being
    // called; a freed closure would crash here instead.
    wait_for(|| calls_after.load(Ordering::Relaxed) > 0);
    assert_eq!(
        dropped.load(Ordering::Relaxed),
        1,
        "the callback never dropped the stream"
    );
    assert!(
        calls_after.load(Ordering::Relaxed) > 0,
        "the leaked stream stopped running"
    );
}
