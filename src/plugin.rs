//! The nih-plug instrument: VST3 + CLAP, MIDI in, stereo out. Host notes and
//! on-screen-keyboard notes both drive the synth; host notes are echoed to
//! the UI so its keyboard lights up.
//!
//! # Pieces
//!
//! * [`NoobWaveParams`] — the nih-plug parameters with a hand-written
//!   [`Params`] implementation, so the ids are the same strings the
//!   standalone and the SPA use (`wt_table`, `filter_cutoff`, …) rather
//!   than the derive macro's field-name scheme. It also carries a
//!   [`StoreSlot`], which persists the page's UI store (user presets) inside
//!   the host's plug-in state.
//! * [`NoobWave`] — the plug-in. `Default` builds the parameters, the
//!   noob-vst-webgui-framework editor (the OS web view showing the SPA embedded from
//!   `web/dist`) and the synth; `process` is the audio thread.
//! * The [`Table`], [`SubOctave`], [`FilterType`] and [`LfoShape`] enums
//!   mirror the DSP's label arrays as nih-plug enum parameters.
//!
//! # Events on the audio thread
//!
//! 1. Browser events (notes from the on-screen keyboard, pitch bend, CC
//!    120 / 123) are drained first and applied at the start of the block —
//!    they carry no timing, and the network already delayed them by more
//!    than a block.
//! 2. Host events are applied sample-accurately: the block is rendered up
//!    to each event's offset, the event is applied, and rendering resumes.
//!    Note on / off are also echoed to the page as noob-vst-webgui-framework events so the
//!    keyboard highlights what the host plays.
//!
//! # Threads and state
//!
//! * Parameter changes reach the audio thread through nih-plug's own
//!   atomics; every block builds a [`Settings`] snapshot and hands it to
//!   [`Synth::configure`], which ignores unchanged snapshots.
//! * Page edits travel page → noob-vst-webgui-framework → GUI-thread timer → host as
//!   parameter gestures (see `noob-vst-webgui-framework-nih`); the host then calls back into
//!   the editor with the new value, so the page shows what the host holds.
//! * The UI store is saved by `Params::serialize_fields` and restored by
//!   `Params::deserialize_fields`; the slot buffers state restored before
//!   the bridge exists.
//! * Telemetry is published every block from `process` through
//!   [`Telemetry`].
//!
//! The editor opens at 1080 × 640; the page may ask for other sizes with a
//! `resize` message. `initialize` tells the page the host's sample rate.

use std::collections::BTreeMap;
use std::num::NonZeroU32;
use std::sync::Arc;

use include_dir::{Dir, include_dir};
use nih_plug::prelude::*;
use noob_vst_webgui_framework::{Assets, AudioHandle, NoobVstWebguiFramework, UiEvent, event_kind};
use noob_vst_webgui_framework_nih::{EditorConfig, NoobVstWebguiFrameworkEditor, StoreSlot};

use crate::dsp::{
    self, AdsrParams, FilterMode, MAX_UNISON, MAX_VOICES, Settings, Synth, Telemetry,
};

/// The built SPA (`npm run build` in `web/`), embedded at compile time.
static UI: Dir = include_dir!("$CARGO_MANIFEST_DIR/web/dist");

/// Asset lookup for the noob-vst-webgui-framework server: a request path to the embedded
/// file's bytes, or `None` when the path does not exist.
fn ui_lookup(path: &str) -> Option<&'static [u8]> {
    UI.get_file(path).map(|f| f.contents())
}

/// `wt_table`: the factory wavetable, in [`TABLE_NAMES`](dsp::TABLE_NAMES)
/// order.
#[derive(Enum, Clone, Copy, PartialEq, Eq, Debug)]
pub enum Table {
    /// Sine → triangle → saw → square → pulse morph.
    #[name = "Basic Shapes"]
    Basic,
    /// Partials added two at a time, sine towards saw.
    Harmonics,
    /// Pulse width 50 % → 3 %.
    #[name = "PWM"]
    Pwm,
    /// Two moving resonant peaks, vowel-like.
    Formant,
    /// Hard-synced saw, ratio 1 → 4.
    Digital,
    /// Eight bell-like partials, darker to brighter.
    Bells,
}

/// `sub_octave`: how far below the note the sine sub oscillator sits.
#[derive(Enum, Clone, Copy, PartialEq, Eq, Debug)]
pub enum SubOctave {
    /// One octave down.
    #[name = "-1 oct"]
    One,
    /// Two octaves down.
    #[name = "-2 oct"]
    Two,
}

/// `filter_mode`: the filter response, in [`FilterMode`] order.
#[derive(Enum, Clone, Copy, PartialEq, Eq, Debug)]
pub enum FilterType {
    /// 12 dB/oct low-pass.
    #[name = "LP 12"]
    Lp12,
    /// 24 dB/oct low-pass.
    #[name = "LP 24"]
    Lp24,
    /// Band-pass.
    #[name = "BP"]
    Bp,
    /// High-pass.
    #[name = "HP"]
    Hp,
}

/// `lfo_shape`: the LFO waveform, in [`LFO_SHAPES`](dsp::LFO_SHAPES) order.
#[derive(Enum, Clone, Copy, PartialEq, Eq, Debug)]
pub enum LfoShape {
    /// Sine.
    Sine,
    /// Triangle.
    Triangle,
    /// Rising saw.
    Saw,
    /// Square, 50 % duty.
    Square,
    /// Sample-and-hold: a new random level per cycle.
    #[name = "S&H"]
    SampleHold,
}

/// The four nih-plug parameters of one envelope. Times are 1 ms – 10 s on
/// a skewed range (more resolution at the short end), sustain is 0–100 %.
pub struct AdsrParamSet {
    /// `<prefix>_attack`, seconds.
    pub attack: FloatParam,
    /// `<prefix>_decay`, seconds.
    pub decay: FloatParam,
    /// `<prefix>_sustain`, percent.
    pub sustain: FloatParam,
    /// `<prefix>_release`, seconds.
    pub release: FloatParam,
}

impl AdsrParamSet {
    /// Parameters named `<name> Attack` … `<name> Release` with defaults
    /// from `d`.
    fn new(name: &str, d: &AdsrParams) -> Self {
        let time = |label: String, dflt: f32| {
            FloatParam::new(
                label,
                dflt,
                FloatRange::Skewed {
                    min: 0.001,
                    max: 10.0,
                    factor: FloatRange::skew_factor(-2.0),
                },
            )
            .with_unit(" s")
            .with_value_to_string(formatters::v2s_f32_rounded(3))
        };
        AdsrParamSet {
            attack: time(format!("{name} Attack"), d.attack_s),
            decay: time(format!("{name} Decay"), d.decay_s),
            sustain: FloatParam::new(
                format!("{name} Sustain"),
                d.sustain * 100.0,
                FloatRange::Linear {
                    min: 0.0,
                    max: 100.0,
                },
            )
            .with_unit(" %"),
            release: time(format!("{name} Release"), d.release_s),
        }
    }
    /// The DSP-side values (sustain scaled to 0..1).
    fn value(&self) -> AdsrParams {
        AdsrParams {
            attack_s: self.attack.value(),
            decay_s: self.decay.value(),
            sustain: self.sustain.value() / 100.0,
            release_s: self.release.value(),
        }
    }
}

/// Every parameter of the instrument. Field order is the order of the
/// parameter map; ids, groups and ranges are:
///
/// | id | group | range | default |
/// |---|---|---|---|
/// | `wt_table` | osc | [`Table`] | Basic Shapes |
/// | `wt_position` | osc | 0 – 1 | 0 |
/// | `osc_octave` | osc | -3 – 3 | 0 |
/// | `osc_semi` | osc | -12 – 12 | 0 |
/// | `osc_fine` | osc | -100 – 100 ct | 0 |
/// | `unison_voices` | osc | 1 – 7 | 1 |
/// | `unison_detune` | osc | 0 – 100 ct | 15 |
/// | `unison_width` | osc | 0 – 100 % | 50 |
/// | `osc_level` | osc | 0 – 100 % | 80 |
/// | `osc_phase_random` | osc | on / off | on |
/// | `sub_level` | osc | 0 – 100 % | 0 |
/// | `sub_octave` | osc | [`SubOctave`] | -1 oct |
/// | `filter_mode` | filter | [`FilterType`] | LP 12 |
/// | `filter_cutoff` | filter | 20 Hz – 20 kHz (skewed) | 8 kHz |
/// | `filter_res` | filter | 0 – 100 % | 15 |
/// | `filter_env` | filter | -100 – 100 % | 40 |
/// | `filter_key` | filter | 0 – 100 % | 50 |
/// | `amp_attack` … `amp_release` | amp | see [`AdsrParamSet`] | 5 ms, 200 ms, 80 %, 300 ms |
/// | `filt_attack` … `filt_release` | filt | see [`AdsrParamSet`] | 5 ms, 400 ms, 30 %, 400 ms |
/// | `lfo_rate` | lfo | 0.02 – 20 Hz (skewed) | 2 Hz |
/// | `lfo_shape` | lfo | [`LfoShape`] | Sine |
/// | `lfo_pos` | lfo | -100 – 100 % | 0 |
/// | `lfo_cutoff` | lfo | -4 – 4 oct | 0 |
/// | `lfo_pitch` | lfo | -12 – 12 st | 0 |
/// | `lfo_retrig` | lfo | on / off | off |
/// | `vel_amp` | global | 0 – 100 % | 70 |
/// | `glide` | global | 0 – 2 s (skewed) | 0 |
/// | `master` | global | -24 – 12 dB | -6 |
/// | `poly` | global | 1 – 16 | 8 |
///
/// Defaults come from [`Settings::default`], so the plug-in and the
/// standalone start from the same sound. The standalone's
/// [`param_specs`](dsp::param_specs) declares the same list.
pub struct NoobWaveParams {
    /// `wt_table`: factory wavetable.
    pub wt_table: EnumParam<Table>,
    /// `wt_position`: morph position between the table's frames.
    pub wt_position: FloatParam,
    /// `osc_octave`: coarse tuning in octaves.
    pub osc_octave: IntParam,
    /// `osc_semi`: coarse tuning in semitones.
    pub osc_semi: IntParam,
    /// `osc_fine`: fine tuning in cents.
    pub osc_fine: FloatParam,
    /// `unison_voices`: oscillators per voice.
    pub unison_voices: IntParam,
    /// `unison_detune`: cents between the outermost unison oscillators.
    pub unison_detune: FloatParam,
    /// `unison_width`: stereo spread of the unison oscillators.
    pub unison_width: FloatParam,
    /// `osc_level`: oscillator level before the filter.
    pub osc_level: FloatParam,
    /// `osc_phase_random`: random start phases per note.
    pub osc_phase_random: BoolParam,
    /// `sub_level`: sine sub oscillator level.
    pub sub_level: FloatParam,
    /// `sub_octave`: sub oscillator one or two octaves down.
    pub sub_octave: EnumParam<SubOctave>,
    /// `filter_mode`: filter response.
    pub filter_mode: EnumParam<FilterType>,
    /// `filter_cutoff`: cutoff before modulation.
    pub filter_cutoff: FloatParam,
    /// `filter_res`: resonance.
    pub filter_res: FloatParam,
    /// `filter_env`: filter envelope amount (±6 octaves at ±100 %).
    pub filter_env: FloatParam,
    /// `filter_key`: keyboard tracking of the cutoff.
    pub filter_key: FloatParam,
    /// `amp_*`: amplitude envelope.
    pub amp: AdsrParamSet,
    /// `filt_*`: filter envelope.
    pub filt: AdsrParamSet,
    /// `lfo_rate`: LFO rate.
    pub lfo_rate: FloatParam,
    /// `lfo_shape`: LFO waveform.
    pub lfo_shape: EnumParam<LfoShape>,
    /// `lfo_pos`: LFO to wavetable position.
    pub lfo_pos: FloatParam,
    /// `lfo_cutoff`: LFO to cutoff, octaves.
    pub lfo_cutoff: FloatParam,
    /// `lfo_pitch`: LFO to pitch, semitones.
    pub lfo_pitch: FloatParam,
    /// `lfo_retrig`: restart the LFO on every note.
    pub lfo_retrig: BoolParam,
    /// `vel_amp`: how much velocity drives level.
    pub vel_amp: FloatParam,
    /// `glide`: portamento time.
    pub glide: FloatParam,
    /// `master`: output gain.
    pub master: FloatParam,
    /// `poly`: polyphony limit.
    pub poly: IntParam,
    /// The page's user presets; not a parameter, but saved with the state.
    pub ui_store: StoreSlot,
}

/// Builds every parameter with the ranges listed on [`NoobWaveParams`].
impl Default for NoobWaveParams {
    fn default() -> Self {
        let d = Settings::default();
        let pct = |name: &str, dflt: f32| {
            FloatParam::new(
                name,
                dflt,
                FloatRange::Linear {
                    min: 0.0,
                    max: 100.0,
                },
            )
            .with_unit(" %")
            .with_step_size(0.1)
        };
        NoobWaveParams {
            wt_table: EnumParam::new("Wavetable", Table::Basic),
            wt_position: FloatParam::new(
                "Position",
                d.position,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_value_to_string(formatters::v2s_f32_rounded(3)),
            osc_octave: IntParam::new("Octave", 0, IntRange::Linear { min: -3, max: 3 }),
            osc_semi: IntParam::new("Semi", 0, IntRange::Linear { min: -12, max: 12 }),
            osc_fine: FloatParam::new(
                "Fine",
                0.0,
                FloatRange::Linear {
                    min: -100.0,
                    max: 100.0,
                },
            )
            .with_unit(" ct")
            .with_step_size(1.0),
            unison_voices: IntParam::new(
                "Unison",
                1,
                IntRange::Linear {
                    min: 1,
                    max: MAX_UNISON as i32,
                },
            ),
            unison_detune: FloatParam::new(
                "Detune",
                d.detune,
                FloatRange::Linear {
                    min: 0.0,
                    max: 100.0,
                },
            )
            .with_unit(" ct")
            .with_step_size(0.5),
            unison_width: pct("Width", d.width * 100.0),
            osc_level: pct("Osc Level", d.osc_level * 100.0),
            osc_phase_random: BoolParam::new("Random Phase", true),
            sub_level: pct("Sub Level", 0.0),
            sub_octave: EnumParam::new("Sub Octave", SubOctave::One),
            filter_mode: EnumParam::new("Filter Type", FilterType::Lp12),
            filter_cutoff: FloatParam::new(
                "Cutoff",
                d.cutoff,
                FloatRange::Skewed {
                    min: 20.0,
                    max: 20_000.0,
                    factor: FloatRange::skew_factor(-2.0),
                },
            )
            .with_unit(" Hz")
            .with_value_to_string(formatters::v2s_f32_hz_then_khz(1))
            .with_string_to_value(formatters::s2v_f32_hz_then_khz()),
            filter_res: pct("Resonance", d.resonance * 100.0),
            filter_env: FloatParam::new(
                "Env Amount",
                d.filter_env * 100.0,
                FloatRange::Linear {
                    min: -100.0,
                    max: 100.0,
                },
            )
            .with_unit(" %")
            .with_step_size(0.5),
            filter_key: pct("Key Track", d.key_track * 100.0),
            amp: AdsrParamSet::new("Amp", &d.amp),
            filt: AdsrParamSet::new("Filter", &d.filt),
            lfo_rate: FloatParam::new(
                "LFO Rate",
                d.lfo_rate,
                FloatRange::Skewed {
                    min: 0.02,
                    max: 20.0,
                    factor: FloatRange::skew_factor(-1.5),
                },
            )
            .with_unit(" Hz")
            .with_value_to_string(formatters::v2s_f32_rounded(2)),
            lfo_shape: EnumParam::new("LFO Shape", LfoShape::Sine),
            lfo_pos: FloatParam::new(
                "LFO → Position",
                0.0,
                FloatRange::Linear {
                    min: -100.0,
                    max: 100.0,
                },
            )
            .with_unit(" %")
            .with_step_size(0.5),
            lfo_cutoff: FloatParam::new(
                "LFO → Cutoff",
                0.0,
                FloatRange::Linear {
                    min: -4.0,
                    max: 4.0,
                },
            )
            .with_unit(" oct")
            .with_step_size(0.01),
            lfo_pitch: FloatParam::new(
                "LFO → Pitch",
                0.0,
                FloatRange::Linear {
                    min: -12.0,
                    max: 12.0,
                },
            )
            .with_unit(" st")
            .with_step_size(0.01),
            lfo_retrig: BoolParam::new("LFO Retrigger", false),
            vel_amp: pct("Velocity → Amp", d.vel_amp * 100.0),
            glide: FloatParam::new(
                "Glide",
                0.0,
                FloatRange::Skewed {
                    min: 0.0,
                    max: 2.0,
                    factor: 0.5,
                },
            )
            .with_unit(" s")
            .with_value_to_string(formatters::v2s_f32_rounded(3)),
            master: FloatParam::new(
                "Master",
                d.master_db,
                FloatRange::Linear {
                    min: -24.0,
                    max: 12.0,
                },
            )
            .with_unit(" dB")
            .with_step_size(0.1),
            poly: IntParam::new(
                "Voices",
                d.poly as i32,
                IntRange::Linear {
                    min: 1,
                    max: MAX_VOICES as i32,
                },
            ),
            ui_store: StoreSlot::new(),
        }
    }
}

// Implemented by hand so the ids match the standalone binary and the SPA.
// SAFETY (nih-plug's contract for `unsafe impl Params`): every pointer in
// `param_map` points into `self`, which lives as long as the `Arc` the
// plug-in hands to the host, and the map is the same for the object's
// whole life.
unsafe impl Params for NoobWaveParams {
    /// `(id, pointer, group)` for every parameter, in [`NoobWaveParams`]
    /// field order.
    fn param_map(&self) -> Vec<(String, ParamPtr, String)> {
        let g = |s: &str| s.to_string();
        let mut v = vec![
            (g("wt_table"), self.wt_table.as_ptr(), g("osc")),
            (g("wt_position"), self.wt_position.as_ptr(), g("osc")),
            (g("osc_octave"), self.osc_octave.as_ptr(), g("osc")),
            (g("osc_semi"), self.osc_semi.as_ptr(), g("osc")),
            (g("osc_fine"), self.osc_fine.as_ptr(), g("osc")),
            (g("unison_voices"), self.unison_voices.as_ptr(), g("osc")),
            (g("unison_detune"), self.unison_detune.as_ptr(), g("osc")),
            (g("unison_width"), self.unison_width.as_ptr(), g("osc")),
            (g("osc_level"), self.osc_level.as_ptr(), g("osc")),
            (
                g("osc_phase_random"),
                self.osc_phase_random.as_ptr(),
                g("osc"),
            ),
            (g("sub_level"), self.sub_level.as_ptr(), g("osc")),
            (g("sub_octave"), self.sub_octave.as_ptr(), g("osc")),
            (g("filter_mode"), self.filter_mode.as_ptr(), g("filter")),
            (g("filter_cutoff"), self.filter_cutoff.as_ptr(), g("filter")),
            (g("filter_res"), self.filter_res.as_ptr(), g("filter")),
            (g("filter_env"), self.filter_env.as_ptr(), g("filter")),
            (g("filter_key"), self.filter_key.as_ptr(), g("filter")),
        ];
        for (prefix, set, grp) in [("amp", &self.amp, "amp"), ("filt", &self.filt, "filt")] {
            v.push((format!("{prefix}_attack"), set.attack.as_ptr(), g(grp)));
            v.push((format!("{prefix}_decay"), set.decay.as_ptr(), g(grp)));
            v.push((format!("{prefix}_sustain"), set.sustain.as_ptr(), g(grp)));
            v.push((format!("{prefix}_release"), set.release.as_ptr(), g(grp)));
        }
        v.extend([
            (g("lfo_rate"), self.lfo_rate.as_ptr(), g("lfo")),
            (g("lfo_shape"), self.lfo_shape.as_ptr(), g("lfo")),
            (g("lfo_pos"), self.lfo_pos.as_ptr(), g("lfo")),
            (g("lfo_cutoff"), self.lfo_cutoff.as_ptr(), g("lfo")),
            (g("lfo_pitch"), self.lfo_pitch.as_ptr(), g("lfo")),
            (g("lfo_retrig"), self.lfo_retrig.as_ptr(), g("lfo")),
            (g("vel_amp"), self.vel_amp.as_ptr(), g("global")),
            (g("glide"), self.glide.as_ptr(), g("global")),
            (g("master"), self.master.as_ptr(), g("global")),
            (g("poly"), self.poly.as_ptr(), g("global")),
        ]);
        v
    }

    /// Persist the page's UI store (user presets) alongside the parameters.
    fn serialize_fields(&self) -> BTreeMap<String, String> {
        let mut m = BTreeMap::new();
        self.ui_store.serialize_into(&mut m);
        m
    }

    /// Restore the UI store: applied at once if the bridge exists, kept
    /// until [`StoreSlot::attach`] otherwise.
    fn deserialize_fields(&self, serialized: &BTreeMap<String, String>) {
        self.ui_store.deserialize_from(serialized);
    }
}

impl NoobWaveParams {
    /// Snapshot the current values as DSP [`Settings`] (percent → 0..1,
    /// enums → indices). Called every block; cheap.
    fn settings(&self) -> Settings {
        Settings {
            table: self.wt_table.value() as usize,
            position: self.wt_position.value(),
            octave: self.osc_octave.value(),
            semi: self.osc_semi.value(),
            fine: self.osc_fine.value(),
            unison: self.unison_voices.value() as usize,
            detune: self.unison_detune.value(),
            width: self.unison_width.value() / 100.0,
            osc_level: self.osc_level.value() / 100.0,
            phase_random: self.osc_phase_random.value(),
            sub_level: self.sub_level.value() / 100.0,
            sub_octave: self.sub_octave.value() as u8 + 1,
            filter_mode: FilterMode::from_index(self.filter_mode.value() as usize),
            cutoff: self.filter_cutoff.value(),
            resonance: self.filter_res.value() / 100.0,
            filter_env: self.filter_env.value() / 100.0,
            key_track: self.filter_key.value() / 100.0,
            amp: self.amp.value(),
            filt: self.filt.value(),
            lfo_rate: self.lfo_rate.value(),
            lfo_shape: self.lfo_shape.value() as usize,
            lfo_pos: self.lfo_pos.value() / 100.0,
            lfo_cutoff: self.lfo_cutoff.value(),
            lfo_pitch: self.lfo_pitch.value(),
            lfo_retrig: self.lfo_retrig.value(),
            vel_amp: self.vel_amp.value() / 100.0,
            glide_s: self.glide.value(),
            master_db: self.master.value(),
            poly: self.poly.value() as usize,
        }
    }
}

/// The plug-in instance.
pub struct NoobWave {
    /// The parameters, shared with the host and the editor.
    params: Arc<NoobWaveParams>,
    /// The noob-vst-webgui-framework editor (web view + server); handed to the host on demand.
    editor: Arc<NoobVstWebguiFrameworkEditor>,
    /// The bridge, for messages such as `sample_rate`.
    bridge: NoobVstWebguiFramework,
    /// Audio-thread handle: browser events in, telemetry out. `None` only
    /// if it had already been taken, which never happens here.
    audio: Option<AudioHandle>,
    /// The sound engine.
    synth: Synth,
    /// Telemetry publisher.
    telemetry: Telemetry,
    /// Last settings handed to the synth, to skip unchanged blocks.
    settings: Settings,
    /// Host sample rate from `initialize`.
    sample_rate: f32,
}

/// Runs when the host creates the instance (main thread): builds the
/// parameters, the editor with the embedded SPA and the synth. The synth's
/// wavetables are rendered here, never on the audio thread.
impl Default for NoobWave {
    fn default() -> Self {
        let params = Arc::new(NoobWaveParams::default());
        let (editor, bridge) = NoobVstWebguiFrameworkEditor::with_builder(
            "Noob-Wave",
            params.as_ref(),
            dsp::streams(48_000.0),
            EditorConfig::new(1080, 640).assets(Assets::Lookup(ui_lookup)),
            |b| {
                b.meta(serde_json::json!({
                    "vendor": "Ely Erin Fox",
                    "version": env!("CARGO_PKG_VERSION"),
                    "sample_rate": 48_000.0,
                    "voices": MAX_VOICES,
                    "frames": dsp::FRAMES,
                    "standalone": false,
                }))
            },
        );
        let audio = bridge.take_audio();
        params.ui_store.attach(&bridge);
        NoobWave {
            params,
            editor,
            bridge,
            audio,
            synth: Synth::new(48_000.0),
            telemetry: Telemetry::new(),
            settings: Settings::default(),
            sample_rate: 48_000.0,
        }
    }
}

impl NoobWave {
    /// Apply one event from the page: note on (a velocity of 0 counts as
    /// note off), note off, pitch bend (`value` -1..1 → ±2 semitones), and
    /// CC 120 / 123 → all notes off. Other kinds are ignored.
    fn ui_event(&mut self, e: UiEvent) {
        match e.kind {
            event_kind::NOTE_ON if e.value > 0.0 => self.synth.note_on(e.a, e.value),
            event_kind::NOTE_ON | event_kind::NOTE_OFF => self.synth.note_off(e.a),
            event_kind::PITCH_BEND => self.synth.set_pitch_bend(e.value * 2.0),
            event_kind::CONTROL if e.a == 123 || e.a == 120 => self.synth.all_notes_off(),
            _ => {}
        }
    }
}

impl Plugin for NoobWave {
    const NAME: &'static str = "Noob-Wave";
    const VENDOR: &'static str = "Ely Erin Fox";
    const URL: &'static str = env!("CARGO_PKG_HOMEPAGE");
    const EMAIL: &'static str = "";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    /// Stereo out, no audio input.
    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[AudioIOLayout {
        main_input_channels: None,
        main_output_channels: NonZeroU32::new(2),
        ..AudioIOLayout::const_default()
    }];

    /// Notes, pitch bend and CCs; no MPE or SysEx.
    const MIDI_INPUT: MidiConfig = MidiConfig::Basic;
    const SAMPLE_ACCURATE_AUTOMATION: bool = false;

    type SysExMessage = ();
    type BackgroundTask = ();

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn editor(&mut self, _async_executor: AsyncExecutor<Self>) -> Option<Box<dyn Editor>> {
        Some(Box::new(self.editor.handle()))
    }

    /// Learn the sample rate: retune the synth and tell the page (its scope
    /// and spectrum axes depend on it).
    fn initialize(
        &mut self,
        _layout: &AudioIOLayout,
        buffer_config: &BufferConfig,
        _context: &mut impl InitContext<Self>,
    ) -> bool {
        self.sample_rate = buffer_config.sample_rate;
        self.synth.set_sample_rate(buffer_config.sample_rate);
        self.bridge.send_json(
            "sample_rate",
            serde_json::json!({ "sample_rate": buffer_config.sample_rate }),
        );
        true
    }

    /// Host reset (transport jump, bypass): silence everything.
    fn reset(&mut self) {
        self.synth.all_notes_off();
    }

    /// The audio thread. See the module docs for the event order. Returns
    /// `KeepAlive` while voices sound so hosts do not suspend the tail.
    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        let s = self.params.settings();
        if s != self.settings {
            self.settings = s;
            self.synth.configure(&s);
        }
        // Notes from the page first (they carry no timing).
        let mut pending = [None; 64];
        let mut n = 0;
        if let Some(audio) = self.audio.as_ref() {
            audio.drain_events(|e| {
                if n < pending.len() {
                    pending[n] = Some(e);
                    n += 1;
                }
            });
        }
        for e in pending.iter().take(n).flatten() {
            self.ui_event(*e);
        }

        let frames = buffer.samples();
        let out = buffer.as_slice();
        let (a, b) = out.split_at_mut(1);
        let (l, r) = (&mut *a[0], &mut *b[0]);

        // Host events, sample accurate: render up to each one, then apply it.
        let mut pos = 0usize;
        let mut next = context.next_event();
        while let Some(ev) = next {
            let t = (ev.timing() as usize).min(frames);
            if t > pos {
                self.synth.render(&mut l[pos..t], &mut r[pos..t]);
                pos = t;
            }
            match ev {
                NoteEvent::NoteOn {
                    note,
                    velocity,
                    channel,
                    ..
                } => {
                    self.synth.note_on(note, velocity);
                    if let Some(audio) = self.audio.as_ref() {
                        audio.send_event(UiEvent::note_on(channel, note, velocity));
                    }
                }
                NoteEvent::NoteOff { note, channel, .. }
                | NoteEvent::Choke { note, channel, .. } => {
                    self.synth.note_off(note);
                    if let Some(audio) = self.audio.as_ref() {
                        audio.send_event(UiEvent::note_off(channel, note, 0.0));
                    }
                }
                NoteEvent::MidiPitchBend { value, .. } => {
                    self.synth.set_pitch_bend((value * 2.0 - 1.0) * 2.0)
                }
                NoteEvent::MidiCC { cc, .. } if cc == 120 || cc == 123 => {
                    self.synth.all_notes_off()
                }
                _ => {}
            }
            next = context.next_event();
        }
        if pos < frames {
            self.synth.render(&mut l[pos..frames], &mut r[pos..frames]);
        }
        if let Some(audio) = self.audio.as_mut() {
            self.telemetry.publish(audio, &self.synth, l, r);
        }
        if self.synth.active_voices() == 0 {
            ProcessStatus::Normal
        } else {
            ProcessStatus::KeepAlive
        }
    }
}

/// VST3 identity: a fixed 16-byte class id and the instrument subcategory.
impl Vst3Plugin for NoobWave {
    const VST3_CLASS_ID: [u8; 16] = *b"NoobWaveVst3Web1";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] =
        &[Vst3SubCategory::Instrument, Vst3SubCategory::Synth];
}

/// CLAP identity and feature tags.
impl ClapPlugin for NoobWave {
    const CLAP_ID: &'static str = "io.github.noob-audio-engineering.noob-wave";
    const CLAP_DESCRIPTION: Option<&'static str> =
        Some("Simple wavetable synth with a web-view editor over bridge");
    const CLAP_MANUAL_URL: Option<&'static str> = None;
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[
        ClapFeature::Instrument,
        ClapFeature::Synthesizer,
        ClapFeature::Stereo,
    ];
}
