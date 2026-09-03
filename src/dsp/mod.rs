//! noob-wave DSP plus the parameter / stream layout shared by the standalone
//! binary and the plugin. Nothing here knows where parameter values or notes
//! come from: the plug-in reads nih-plug parameters and host MIDI, the
//! standalone reads noob-vst-webgui-framework's parameter atomics and browser events, and
//! both feed the same [`Synth`] and [`Telemetry`].
//!
//! # Layout
//!
//! * [`wavetable`] — mipmapped single-cycle tables built from harmonic
//!   spectra with an inverse FFT.
//! * [`synth`] — [`Synth`]: voices, unison, sub oscillator, glide, pitch
//!   bend, per-voice filter and envelopes, one global LFO; [`Settings`] is
//!   the plain-data snapshot of every parameter it needs.
//! * [`filter`] — the topology-preserving state-variable filter.
//! * [`env`](mod@env) — the ADSR envelope.
//! * [`lfo`] — the low-frequency oscillator.
//! * This module — the parameter ids ([`param_specs`], [`ParamIx`]), the
//!   stream ids ([`streams`], [`STREAM_IX`]), the bridge builder for the
//!   standalone ([`build_bridge`]), the audio-thread parameter reader
//!   ([`read_settings`]) and the telemetry publisher ([`Telemetry`]).
//!
//! # Parameter groups
//!
//! Ids are stable strings shared with the SPA; the plug-in's hand-written
//! `Params` implementation uses the same ids in the same order. Groups:
//! `osc` (12), `filter` (5), `amp` (4), `filt` (4), `lfo` (6), `global` (4),
//! 35 parameters in all. Percent-style parameters are exposed as 0–100 and
//! scaled to 0–1 in [`read_settings`]; the plug-in does the same in its
//! `settings()`.
//!
//! # Streams
//!
//! | id | kind | capacity | published |
//! |---|---|---|---|
//! | `scope` | waveform | 512 | every 2nd block: the most recent mono samples |
//! | `spectrum` | spectrum | 1025 | every 2nd block: dBFS magnitudes of a 2048-point Hann FFT |
//! | `meter_out` | meter | 4 | every block: `peak L, peak R, rms L, rms R` |
//! | `voices` | raw | 32 | every block: `level[16]` then `note[16]` (`-1` = idle) |
//! | `modulation` | raw | 2 | every block: live wavetable position and LFO value |
//! | `wavetable` | raw, sticky | 8192 | when the table changes: 32 frames × 256-sample preview |
//!
//! # Real-time rules
//!
//! Everything called from the audio thread ([`read_settings`],
//! [`Synth::render`], [`Telemetry::publish`]) is allocation-free and
//! lock-free: buffers are sized once in `new()`, parameters are atomics,
//! stream frames go into noob-vst-webgui-framework's triple buffers. Building a [`Synth`]
//! renders every factory wavetable and must happen off the audio thread.

pub mod env;
pub mod filter;
pub mod lfo;
pub mod synth;
pub mod wavetable;

pub use env::AdsrParams;
pub use filter::{FILTER_MODE_NAMES, FilterMode};
pub use lfo::LFO_SHAPES;
pub use synth::{MAX_UNISON, MAX_VOICES, Settings, Synth};
pub use wavetable::{FRAMES, PREVIEW_LEN, TABLE_NAMES, Wavetable};

use std::f32::consts::PI;
use std::sync::Arc;

use noob_vst_webgui_framework::{
    AudioHandle, NoobVstWebguiFramework, ParamSpec, StreamKind, StreamSpec,
};
use rustfft::num_complex::Complex;
use rustfft::{Fft, FftPlanner};
use serde_json::json;

/// Labels for the `sub_octave` choice parameter, in index order.
pub const SUB_OCTAVE_NAMES: [&str; 2] = ["-1 oct", "-2 oct"];
/// Samples in one `scope` frame (the most recent output, mono).
pub const SCOPE_LEN: usize = 512;
/// FFT length of the output analyzer; the `spectrum` stream carries
/// [`BINS`] magnitudes from DC to Nyquist.
pub const FFT_SIZE: usize = 2048;
/// Number of spectrum bins published per frame (`FFT_SIZE / 2 + 1`).
pub const BINS: usize = FFT_SIZE / 2 + 1;

// ---------------------------------------------------------------------------
// Parameters
// ---------------------------------------------------------------------------

/// Parameter indices resolved from ids, so the audio thread can read
/// values by index without string lookups. Built by [`param_index`] for
/// the standalone; the plug-in reads its own nih-plug parameters instead.
pub struct ParamIx {
    /// `wt_table`: factory wavetable, index into [`TABLE_NAMES`].
    pub wt_table: usize,
    /// `wt_position`: 0–1 morph position between the table's frames.
    pub wt_position: usize,
    /// `osc_octave`: -3..3 octaves.
    pub osc_octave: usize,
    /// `osc_semi`: -12..12 semitones.
    pub osc_semi: usize,
    /// `osc_fine`: -100..100 cents.
    pub osc_fine: usize,
    /// `unison_voices`: 1..[`MAX_UNISON`] oscillators per voice.
    pub unison_voices: usize,
    /// `unison_detune`: cents between the outermost unison oscillators.
    pub unison_detune: usize,
    /// `unison_width`: 0–100 % stereo spread of the unison oscillators.
    pub unison_width: usize,
    /// `osc_level`: 0–100 % oscillator level.
    pub osc_level: usize,
    /// `osc_phase_random`: toggle, random start phases per note.
    pub osc_phase_random: usize,
    /// `sub_level`: 0–100 % sine sub oscillator level.
    pub sub_level: usize,
    /// `sub_octave`: choice, one or two octaves below.
    pub sub_octave: usize,
    /// `filter_mode`: choice, index into [`FILTER_MODE_NAMES`].
    pub filter_mode: usize,
    /// `filter_cutoff`: 20 Hz – 20 kHz, log taper.
    pub filter_cutoff: usize,
    /// `filter_res`: 0–100 % resonance.
    pub filter_res: usize,
    /// `filter_env`: -100..100 % filter envelope amount (±6 octaves).
    pub filter_env: usize,
    /// `filter_key`: 0–100 % keyboard tracking.
    pub filter_key: usize,
    /// `amp_attack`, `amp_decay`, `amp_sustain`, `amp_release`.
    pub amp: [usize; 4],
    /// `filt_attack`, `filt_decay`, `filt_sustain`, `filt_release`.
    pub filt: [usize; 4],
    /// `lfo_rate`: 0.02–20 Hz, log taper.
    pub lfo_rate: usize,
    /// `lfo_shape`: choice, index into [`LFO_SHAPES`].
    pub lfo_shape: usize,
    /// `lfo_pos`: -100..100 % LFO to wavetable position.
    pub lfo_pos: usize,
    /// `lfo_cutoff`: -4..4 octaves LFO to cutoff.
    pub lfo_cutoff: usize,
    /// `lfo_pitch`: -12..12 semitones LFO to pitch.
    pub lfo_pitch: usize,
    /// `lfo_retrig`: toggle, restart the LFO on every note.
    pub lfo_retrig: usize,
    /// `vel_amp`: 0–100 % velocity to level.
    pub vel_amp: usize,
    /// `glide`: 0–2 s portamento.
    pub glide: usize,
    /// `master`: -24..12 dB output gain.
    pub master: usize,
    /// `poly`: 1..[`MAX_VOICES`] polyphony.
    pub poly: usize,
}

/// Stream indices, in the order [`streams`] declares them.
pub struct StreamIx {
    /// `scope`: recent output samples.
    pub scope: usize,
    /// `spectrum`: output magnitudes in dBFS.
    pub spectrum: usize,
    /// `meter_out`: per-block peak and RMS, both channels.
    pub meter_out: usize,
    /// `voices`: per-voice level and note.
    pub voices: usize,
    /// `modulation`: live wavetable position and LFO value.
    pub modulation: usize,
    /// `wavetable`: preview of the selected table (sticky).
    pub wavetable: usize,
}

/// The fixed stream indices; [`streams`] must declare them in this order.
pub const STREAM_IX: StreamIx = StreamIx {
    scope: 0,
    spectrum: 1,
    meter_out: 2,
    voices: 3,
    modulation: 4,
    wavetable: 5,
};

/// The telemetry streams, in [`STREAM_IX`] order. `sr` goes into the
/// metadata so the page can label the scope and spectrum axes; the
/// `wavetable` stream is sticky, so a page that connects later still gets
/// the current table without waiting for a change. See the module docs for
/// the layout and rate of each stream.
pub fn streams(sr: f32) -> Vec<StreamSpec> {
    vec![
        StreamSpec::new("scope", SCOPE_LEN)
            .name("Output")
            .kind(StreamKind::Waveform)
            .meta(json!({ "sample_rate": sr })),
        StreamSpec::new("spectrum", BINS)
            .name("Output Spectrum")
            .kind(StreamKind::Spectrum)
            .meta(json!({ "sample_rate": sr, "fft_size": FFT_SIZE, "db": true })),
        StreamSpec::new("meter_out", 4)
            .name("Output")
            .kind(StreamKind::Meter)
            .channels(2)
            .meta(json!({ "layout": "peak,peak,rms,rms" })),
        StreamSpec::new("voices", MAX_VOICES * 2)
            .name("Voices")
            .kind(StreamKind::Raw)
            .meta(json!({ "layout": "level[16],note[16]", "voices": MAX_VOICES })),
        StreamSpec::new("modulation", 2)
            .name("Modulation")
            .kind(StreamKind::Raw)
            .meta(json!({ "layout": "position,lfo" })),
        StreamSpec::new("wavetable", FRAMES * PREVIEW_LEN)
            .name("Wavetable")
            .kind(StreamKind::Raw)
            .sticky()
            .meta(json!({ "frames": FRAMES, "size": PREVIEW_LEN })),
    ]
}

/// The four specs of one ADSR: `<prefix>_attack`, `_decay`, `_sustain`,
/// `_release`. Times are 1 ms – 10 s on a log taper, sustain is 0–100 %.
fn adsr_specs(prefix: &str, name: &str, d: &AdsrParams, group: &str) -> [ParamSpec; 4] {
    [
        ParamSpec::new(format!("{prefix}_attack"), format!("{name} Attack"))
            .range(0.001, 10.0)
            .log()
            .default(d.attack_s)
            .unit("s")
            .group(group),
        ParamSpec::new(format!("{prefix}_decay"), format!("{name} Decay"))
            .range(0.001, 10.0)
            .log()
            .default(d.decay_s)
            .unit("s")
            .group(group),
        ParamSpec::new(format!("{prefix}_sustain"), format!("{name} Sustain"))
            .range(0.0, 100.0)
            .default(d.sustain * 100.0)
            .unit("%")
            .group(group),
        ParamSpec::new(format!("{prefix}_release"), format!("{name} Release"))
            .range(0.001, 10.0)
            .log()
            .default(d.release_s)
            .unit("s")
            .group(group),
    ]
}

/// Parameter specs for the standalone binary. Ids and order match the
/// plugin's `Params` implementation, and defaults come from
/// [`Settings::default`] so the two hosts start from the same sound.
/// Choice parameters carry their labels ([`TABLE_NAMES`],
/// [`SUB_OCTAVE_NAMES`], [`FILTER_MODE_NAMES`], [`LFO_SHAPES`]) and are
/// stored as the label index; see [`ParamIx`] for every id, range and unit.
pub fn param_specs() -> Vec<ParamSpec> {
    let d = Settings::default();
    let mut v = vec![
        ParamSpec::new("wt_table", "Wavetable")
            .labels(TABLE_NAMES)
            .default(d.table as f32)
            .group("osc"),
        ParamSpec::new("wt_position", "Position")
            .range(0.0, 1.0)
            .default(d.position)
            .group("osc"),
        ParamSpec::new("osc_octave", "Octave")
            .range(-3.0, 3.0)
            .steps(7)
            .default(0.0)
            .group("osc"),
        ParamSpec::new("osc_semi", "Semi")
            .range(-12.0, 12.0)
            .steps(25)
            .default(0.0)
            .group("osc"),
        ParamSpec::new("osc_fine", "Fine")
            .range(-100.0, 100.0)
            .default(0.0)
            .unit("ct")
            .group("osc"),
        ParamSpec::new("unison_voices", "Unison")
            .range(1.0, MAX_UNISON as f32)
            .steps(MAX_UNISON as u32)
            .default(1.0)
            .group("osc"),
        ParamSpec::new("unison_detune", "Detune")
            .range(0.0, 100.0)
            .default(d.detune)
            .unit("ct")
            .group("osc"),
        ParamSpec::new("unison_width", "Width")
            .range(0.0, 100.0)
            .default(d.width * 100.0)
            .unit("%")
            .group("osc"),
        ParamSpec::new("osc_level", "Osc Level")
            .range(0.0, 100.0)
            .default(d.osc_level * 100.0)
            .unit("%")
            .group("osc"),
        ParamSpec::new("osc_phase_random", "Random Phase")
            .toggle()
            .default(1.0)
            .group("osc"),
        ParamSpec::new("sub_level", "Sub Level")
            .range(0.0, 100.0)
            .default(0.0)
            .unit("%")
            .group("osc"),
        ParamSpec::new("sub_octave", "Sub Octave")
            .labels(SUB_OCTAVE_NAMES)
            .default(0.0)
            .group("osc"),
        ParamSpec::new("filter_mode", "Filter Type")
            .labels(FILTER_MODE_NAMES)
            .default(0.0)
            .group("filter"),
        ParamSpec::new("filter_cutoff", "Cutoff")
            .range(20.0, 20000.0)
            .log()
            .default(d.cutoff)
            .unit("Hz")
            .group("filter"),
        ParamSpec::new("filter_res", "Resonance")
            .range(0.0, 100.0)
            .default(d.resonance * 100.0)
            .unit("%")
            .group("filter"),
        ParamSpec::new("filter_env", "Env Amount")
            .range(-100.0, 100.0)
            .default(d.filter_env * 100.0)
            .unit("%")
            .group("filter"),
        ParamSpec::new("filter_key", "Key Track")
            .range(0.0, 100.0)
            .default(d.key_track * 100.0)
            .unit("%")
            .group("filter"),
    ];
    v.extend(adsr_specs("amp", "Amp", &d.amp, "amp"));
    v.extend(adsr_specs("filt", "Filter", &d.filt, "filt"));
    v.extend([
        ParamSpec::new("lfo_rate", "LFO Rate")
            .range(0.02, 20.0)
            .log()
            .default(d.lfo_rate)
            .unit("Hz")
            .group("lfo"),
        ParamSpec::new("lfo_shape", "LFO Shape")
            .labels(LFO_SHAPES)
            .default(0.0)
            .group("lfo"),
        ParamSpec::new("lfo_pos", "LFO → Position")
            .range(-100.0, 100.0)
            .default(0.0)
            .unit("%")
            .group("lfo"),
        ParamSpec::new("lfo_cutoff", "LFO → Cutoff")
            .range(-4.0, 4.0)
            .default(0.0)
            .unit("oct")
            .group("lfo"),
        ParamSpec::new("lfo_pitch", "LFO → Pitch")
            .range(-12.0, 12.0)
            .default(0.0)
            .unit("st")
            .group("lfo"),
        ParamSpec::new("lfo_retrig", "LFO Retrigger")
            .toggle()
            .group("lfo"),
        ParamSpec::new("vel_amp", "Velocity → Amp")
            .range(0.0, 100.0)
            .default(d.vel_amp * 100.0)
            .unit("%")
            .group("global"),
        ParamSpec::new("glide", "Glide")
            .range(0.0, 2.0)
            .skew(0.5)
            .default(0.0)
            .unit("s")
            .group("global"),
        ParamSpec::new("master", "Master")
            .range(-24.0, 12.0)
            .default(d.master_db)
            .unit("dB")
            .group("global"),
        ParamSpec::new("poly", "Voices")
            .range(1.0, MAX_VOICES as f32)
            .steps(MAX_VOICES as u32)
            .default(d.poly as f32)
            .group("global"),
    ]);
    v
}

/// Build the noob-vst-webgui-framework bridge for the standalone binary: metadata (vendor,
/// version, sample rate, voice count, frame count, `standalone: true`),
/// every parameter from [`param_specs`] and every stream from [`streams`].
/// Returns the bridge and the resolved [`ParamIx`].
pub fn build_bridge(name: &str, sr: f32) -> (NoobVstWebguiFramework, ParamIx) {
    let mut b = NoobVstWebguiFramework::builder(name)
        .meta(json!({
            "vendor": "Ely Erin Fox",
            "version": env!("CARGO_PKG_VERSION"),
            "sample_rate": sr,
            "voices": MAX_VOICES,
            "frames": FRAMES,
            "standalone": true,
        }))
        .params(param_specs());
    for s in streams(sr) {
        b = b.stream(s);
    }
    let s = b.build();
    let ix = param_index(&s);
    (s, ix)
}

/// Resolve the parameter indices by id (works for the plugin's mirror too,
/// since it uses the same ids). Panics on a missing id, which would be a
/// mismatch between [`param_specs`] and the plug-in's `param_map`.
pub fn param_index(s: &NoobVstWebguiFramework) -> ParamIx {
    let ix = |id: &str| s.index_of(id).expect(id);
    let adsr = |p: &str| {
        [
            ix(&format!("{p}_attack")),
            ix(&format!("{p}_decay")),
            ix(&format!("{p}_sustain")),
            ix(&format!("{p}_release")),
        ]
    };
    ParamIx {
        wt_table: ix("wt_table"),
        wt_position: ix("wt_position"),
        osc_octave: ix("osc_octave"),
        osc_semi: ix("osc_semi"),
        osc_fine: ix("osc_fine"),
        unison_voices: ix("unison_voices"),
        unison_detune: ix("unison_detune"),
        unison_width: ix("unison_width"),
        osc_level: ix("osc_level"),
        osc_phase_random: ix("osc_phase_random"),
        sub_level: ix("sub_level"),
        sub_octave: ix("sub_octave"),
        filter_mode: ix("filter_mode"),
        filter_cutoff: ix("filter_cutoff"),
        filter_res: ix("filter_res"),
        filter_env: ix("filter_env"),
        filter_key: ix("filter_key"),
        amp: adsr("amp"),
        filt: adsr("filt"),
        lfo_rate: ix("lfo_rate"),
        lfo_shape: ix("lfo_shape"),
        lfo_pos: ix("lfo_pos"),
        lfo_cutoff: ix("lfo_cutoff"),
        lfo_pitch: ix("lfo_pitch"),
        lfo_retrig: ix("lfo_retrig"),
        vel_amp: ix("vel_amp"),
        glide: ix("glide"),
        master: ix("master"),
        poly: ix("poly"),
    }
}

/// Read the settings from the noob-vst-webgui-framework store on the audio thread: one
/// relaxed atomic load per parameter, percent values scaled to 0–1,
/// choice values rounded to their index. Cheap enough to call every block;
/// [`Synth::configure`] then compares the result with the previous
/// snapshot and does nothing when it is unchanged.
#[inline]
pub fn read_settings(audio: &AudioHandle, ix: &ParamIx) -> Settings {
    let adsr = |a: &[usize; 4]| AdsrParams {
        attack_s: audio.param(a[0]),
        decay_s: audio.param(a[1]),
        sustain: audio.param(a[2]) / 100.0,
        release_s: audio.param(a[3]),
    };
    Settings {
        table: audio.param(ix.wt_table).round() as usize,
        position: audio.param(ix.wt_position),
        octave: audio.param(ix.osc_octave).round() as i32,
        semi: audio.param(ix.osc_semi).round() as i32,
        fine: audio.param(ix.osc_fine),
        unison: audio.param(ix.unison_voices).round() as usize,
        detune: audio.param(ix.unison_detune),
        width: audio.param(ix.unison_width) / 100.0,
        osc_level: audio.param(ix.osc_level) / 100.0,
        phase_random: audio.param(ix.osc_phase_random) >= 0.5,
        sub_level: audio.param(ix.sub_level) / 100.0,
        sub_octave: audio.param(ix.sub_octave).round() as u8 + 1,
        filter_mode: FilterMode::from_index(audio.param(ix.filter_mode).round() as usize),
        cutoff: audio.param(ix.filter_cutoff),
        resonance: audio.param(ix.filter_res) / 100.0,
        filter_env: audio.param(ix.filter_env) / 100.0,
        key_track: audio.param(ix.filter_key) / 100.0,
        amp: adsr(&ix.amp),
        filt: adsr(&ix.filt),
        lfo_rate: audio.param(ix.lfo_rate),
        lfo_shape: audio.param(ix.lfo_shape).round() as usize,
        lfo_pos: audio.param(ix.lfo_pos) / 100.0,
        lfo_cutoff: audio.param(ix.lfo_cutoff),
        lfo_pitch: audio.param(ix.lfo_pitch),
        lfo_retrig: audio.param(ix.lfo_retrig) >= 0.5,
        vel_amp: audio.param(ix.vel_amp) / 100.0,
        glide_s: audio.param(ix.glide),
        master_db: audio.param(ix.master),
        poly: audio.param(ix.poly).round() as usize,
    }
}

// ---------------------------------------------------------------------------
// Telemetry helpers
// ---------------------------------------------------------------------------

/// Output spectrum in dBFS (a full-scale sine reads 0 dB) and the scope's
/// sample history, both fed from one ring buffer.
///
/// The ring holds `2 × FFT_SIZE` mono samples. [`compute`](Self::compute)
/// windows the most recent `FFT_SIZE` of them with a Hann window and takes
/// a forward FFT; the `4 / N` gain compensates the window's coherent gain
/// (0.5) and the one-sided spectrum (×2) so a sine of amplitude 1 peaks at
/// 0 dBFS. [`recent`](Self::recent) copies the newest samples out for the
/// scope. Both are allocation-free after `new()`.
pub struct Analyzer {
    /// Planned forward FFT of `FFT_SIZE` points.
    fft: Arc<dyn Fft<f32>>,
    /// In-place FFT buffer.
    buf: Vec<Complex<f32>>,
    /// Scratch space the FFT needs.
    scratch: Vec<Complex<f32>>,
    /// Hann window, `FFT_SIZE` long.
    window: Vec<f32>,
    /// Sample history, `2 × FFT_SIZE` long.
    ring: Vec<f32>,
    /// Next write position in `ring`.
    pos: usize,
}

impl Analyzer {
    /// Plans the FFT and allocates every buffer; call off the audio thread.
    pub fn new() -> Self {
        let fft = FftPlanner::new().plan_fft_forward(FFT_SIZE);
        let scratch = vec![Complex::default(); fft.get_inplace_scratch_len()];
        Analyzer {
            fft,
            buf: vec![Complex::default(); FFT_SIZE],
            scratch,
            window: (0..FFT_SIZE)
                .map(|i| 0.5 - 0.5 * (2.0 * PI * i as f32 / FFT_SIZE as f32).cos())
                .collect(),
            ring: vec![0.0; FFT_SIZE * 2],
            pos: 0,
        }
    }
    /// Append one mono sample to the history.
    #[inline]
    pub fn push(&mut self, x: f32) {
        self.ring[self.pos] = x;
        self.pos = (self.pos + 1) % self.ring.len();
    }
    /// The most recent `n` samples (for the scope), oldest first.
    pub fn recent(&self, out: &mut [f32]) {
        let n = out.len().min(self.ring.len());
        let len = self.ring.len();
        let start = (self.pos + len - n) % len;
        for (i, o) in out.iter_mut().enumerate().take(n) {
            *o = self.ring[(start + i) % len];
        }
    }
    /// Write up to [`BINS`] magnitudes in dBFS into `out`, from the newest
    /// `FFT_SIZE` samples. Values are floored at -180 dB.
    pub fn compute(&mut self, out: &mut [f32]) {
        let n = self.ring.len();
        let start = (self.pos + n - FFT_SIZE) % n;
        for (k, c) in self.buf.iter_mut().enumerate() {
            *c = Complex::new(self.ring[(start + k) % n] * self.window[k], 0.0);
        }
        self.fft
            .process_with_scratch(&mut self.buf, &mut self.scratch);
        let gain = 4.0 / FFT_SIZE as f32;
        for (k, o) in out.iter_mut().enumerate().take(BINS) {
            *o = 20.0 * (self.buf[k].norm() * gain).max(1e-9).log10();
        }
    }
}

impl Default for Analyzer {
    fn default() -> Self {
        Analyzer::new()
    }
}

/// Per-block peak and RMS meter for a stereo pair. Accumulates with
/// [`feed`](Self::feed); [`take`](Self::take) returns the block's
/// `[peak L, peak R, rms L, rms R]` (linear, 1.0 = full scale) and resets.
#[derive(Default)]
pub struct Meter {
    /// Highest absolute sample seen since the last `take`, per channel.
    peak: [f32; 2],
    /// Sum of squares since the last `take`, per channel.
    sum_sq: [f32; 2],
    /// Samples fed since the last `take`.
    n: u32,
}

impl Meter {
    /// Accumulate one stereo sample.
    #[inline]
    pub fn feed(&mut self, l: f32, r: f32) {
        self.peak[0] = self.peak[0].max(l.abs());
        self.peak[1] = self.peak[1].max(r.abs());
        self.sum_sq[0] += l * l;
        self.sum_sq[1] += r * r;
        self.n += 1;
    }
    /// The block's `[peak L, peak R, rms L, rms R]`, then start a new block.
    pub fn take(&mut self) -> [f32; 4] {
        let n = self.n.max(1) as f32;
        let out = [
            self.peak[0],
            self.peak[1],
            (self.sum_sq[0] / n).sqrt(),
            (self.sum_sq[1] / n).sqrt(),
        ];
        *self = Meter::default();
        out
    }
}

/// Everything the audio thread publishes each block, shared by the
/// standalone and the plugin so the two never drift apart.
///
/// Call [`publish`](Self::publish) once per rendered block. Rates:
///
/// * every block: `meter_out`, `voices`, `modulation`;
/// * every second block: `spectrum` and `scope` (an FFT per block would be
///   wasted at small block sizes; the page interpolates anyway);
/// * on change only: `wavetable`, the preview of the selected table. The
///   stream is sticky, so late clients still receive it.
///
/// All buffers are allocated in [`new`](Self::new); publishing copies into
/// noob-vst-webgui-framework's wait-free triple buffers and never blocks.
pub struct Telemetry {
    /// Ring buffer, FFT and window for the spectrum and scope.
    pub analyzer: Analyzer,
    /// Peak / RMS accumulator for `meter_out`.
    pub meter: Meter,
    /// Scratch for one `spectrum` frame (`BINS` values).
    spectrum: Vec<f32>,
    /// Scratch for one `scope` frame (`SCOPE_LEN` values).
    scope: Vec<f32>,
    /// Scratch for one `voices` frame (`2 × MAX_VOICES` values).
    voices: Vec<f32>,
    /// Blocks published so far; drives the every-second-block cadence.
    blocks: u64,
    /// Table index whose preview was last published, to send it once.
    table_sent: Option<usize>,
}

impl Telemetry {
    /// Allocate every scratch buffer; call off the audio thread.
    pub fn new() -> Self {
        Telemetry {
            analyzer: Analyzer::new(),
            meter: Meter::default(),
            spectrum: vec![0.0; BINS],
            scope: vec![0.0; SCOPE_LEN],
            voices: vec![0.0; MAX_VOICES * 2],
            blocks: 0,
            table_sent: None,
        }
    }

    /// Call after rendering a block into `l` / `r`. Feeds the analyzer (mono
    /// mix) and the meter with the block, then publishes according to the
    /// cadence in the type docs.
    pub fn publish(&mut self, audio: &mut AudioHandle, synth: &Synth, l: &[f32], r: &[f32]) {
        for (a, b) in l.iter().zip(r) {
            self.analyzer.push(0.5 * (a + b));
            self.meter.feed(*a, *b);
        }
        self.blocks += 1;
        audio.publish_slice(STREAM_IX.meter_out, &self.meter.take());
        synth.voice_state(&mut self.voices);
        audio.publish_slice(STREAM_IX.voices, &self.voices);
        audio.publish_slice(
            STREAM_IX.modulation,
            &[synth.live_position(), synth.lfo_value()],
        );
        if self.blocks.is_multiple_of(2) {
            self.analyzer.compute(&mut self.spectrum);
            audio.publish_slice(STREAM_IX.spectrum, &self.spectrum);
            self.analyzer.recent(&mut self.scope);
            audio.publish_slice(STREAM_IX.scope, &self.scope);
        }
        let table = synth.settings().table;
        if self.table_sent != Some(table) {
            self.table_sent = Some(table);
            audio.publish_slice(STREAM_IX.wavetable, &synth.table().preview);
        }
    }
}

impl Default for Telemetry {
    fn default() -> Self {
        Telemetry::new()
    }
}
