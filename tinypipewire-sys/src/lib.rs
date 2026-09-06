//! Raw FFI bindings to [tinypipewire], a small C wrapper around PipeWire's
//! `pw_stream` for audio and video capture and audio playback.
//!
//! Everything here is generated from the C headers and is `unsafe` to call.
//! Use the `tinypipewire` crate for a safe interface.
//!
//! [tinypipewire]: https://github.com/tinyPipeWire/tinypipewire

#![allow(non_upper_case_globals, non_camel_case_types, non_snake_case)]
#![allow(clippy::all)]
#![no_std]

include!(concat!(env!("OUT_DIR"), "/bindings.rs"));
