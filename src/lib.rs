//! noob-wave: a free wavetable synth by Noob Audio Engineering, built on
//! noob-vst-webgui-framework. It shows the *instrument* side of the framework: notes arriving from a host as
//! sample-accurate MIDI, notes arriving from the browser as binary event
//! frames, and a page that renders scope, spectrum, voice and modulation
//! telemetry published straight from the audio thread.
//!
//! # What lives where
//!
//! | Layer | Path | Role |
//! |---|---|---|
//! | DSP | [`dsp`] | Mipmapped wavetables, voices with unison and a sub oscillator, a TPT state-variable filter, two ADSRs, one LFO, plus the parameter / stream layout shared by both hosts. Knows nothing about MIDI, nih-plug or the network. |
//! | Plug-in | `plugin` (feature `plugin`) | The nih-plug VST3 / CLAP instrument. Owns the nih-plug parameters, splits host events sample-accurately, forwards on-screen-keyboard events, and mounts the noob-vst-webgui-framework editor (the OS web view showing the SPA embedded from `web/dist`). |
//! | Standalone | `src/bin/standalone.rs` | The same engine without a DAW: real audio output through cpal, the noob-vst-webgui-framework server on port 4243 (or the next free one), and the SPA served from `web/dist`. |
//! | UI | `web/` | The Vue 3 + Tailwind single-page app (its own README lives there). |
//!
//! # Where the framework ends and the synth begins
//!
//! Everything generic is in `noob-vst-webgui-framework` (bridge, wire format, server,
//! UI store, discovery), `noob-vst-webgui-framework-nih` (the nih-plug editor adapter) and
//! `@noob-audio-engineering/noob-vst-webgui-framework` (browser client and generic components such as the
//! keyboard, scope and wavetable views). This crate only contributes what is
//! specific to a wavetable synth: the sound engine, the 35 parameters, the
//! six telemetry streams and the page that draws them. If something here
//! looks reusable, it probably belongs in the framework instead.
//!
//! # Threads
//!
//! * **Audio thread** (host `process()` or the cpal callback): reads the
//!   parameter atomics, drains note events, renders, publishes telemetry.
//!   Never allocates or blocks after construction.
//! * **noob-vst-webgui-framework pump / network threads**: fan the published frames out to
//!   the connected pages and queue their edits and events. Owned by
//!   `noob-vst-webgui-framework`.
//! * **GUI thread** (plug-in only): forwards page edits to the host as
//!   parameter gestures. Owned by `noob-vst-webgui-framework-nih`.
//!
//! The crate exports the plug-in entry points with nih-plug's macros when
//! the `plugin` feature is on; the default features build only the DSP and
//! the standalone binary, so `cargo test` needs no host SDK.

// DSP loops index several buffers by the same sample or bin index; iterator
// chains would hide the arithmetic the comments describe.
#![allow(clippy::needless_range_loop)]

pub mod dsp;

#[cfg(feature = "plugin")]
pub mod plugin;

#[cfg(feature = "plugin")]
nih_plug::nih_export_vst3!(plugin::NoobWave);
#[cfg(feature = "plugin")]
nih_plug::nih_export_clap!(plugin::NoobWave);
