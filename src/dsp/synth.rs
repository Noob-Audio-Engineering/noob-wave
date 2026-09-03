//! Voices and the synth: wavetable oscillator with unison, sub oscillator,
//! per-voice filter and envelopes, one global LFO.
//!
//! # Voice architecture
//!
//! [`Synth`] owns [`MAX_VOICES`] voice slots. A note claims a free slot, or
//! steals the oldest sounding one when the `poly` limit is reached (see
//! [`Synth::note_on`]). Each voice runs:
//!
//! * up to [`MAX_UNISON`] wavetable oscillators, spread symmetrically in
//!   pitch (`detune` cents between the outermost two) and in the stereo
//!   field (`width`, equal-power panning), summed with a `1/√n` gain so
//!   loudness stays roughly constant with the count;
//! * an optional sine sub oscillator one or two octaves down, mixed into
//!   the mono path;
//! * one [`VoiceFilter`] (mono), driven by cutoff, key tracking, the
//!   filter envelope and the LFO;
//! * two [`Adsr`] envelopes, amplitude and filter; a voice frees itself
//!   when its amplitude envelope goes idle.
//!
//! Stereo is kept with a mid/side trick: the unison oscillators are summed
//! to a mid signal, which goes through the filter, and a side signal, which
//! bypasses it; the output is `mid ± side`. One filter per voice is enough
//! and the stereo spread survives.
//!
//! Glide moves a voice's pitch linearly (in semitones) from the previous
//! note to the new one over `glide_s` seconds. Pitch bend is a global
//! offset in semitones. With `phase_random` each oscillator starts at a
//! random phase, otherwise at zero (deterministic, useful for tests).
//!
//! # Rendering
//!
//! [`Synth::render`] works in control-rate chunks of `CHUNK` (16) samples:
//! per chunk it advances the LFO, updates every active voice's pitch
//! (glide, tuning, LFO, bend), unison increments and pans, mip level,
//! sub increment and filter coefficients; then it renders the chunk at
//! audio rate. Coefficient updates every 16 samples keep the filter cheap
//! while modulation still sounds smooth.
//!
//! # Real-time rules
//!
//! Everything except [`Synth::new`] runs on the audio thread: no
//! allocation, no locking, no I/O. [`Settings`] is `Copy` and compared by
//! value, so a host can hand a fresh snapshot to [`Synth::configure`] every
//! block for free.

use super::env::{Adsr, AdsrParams};
use super::filter::{FilterMode, VoiceFilter};
use super::lfo::Lfo;
use super::wavetable::{TABLE_NAMES, Wavetable};

/// Voice slots (polyphony ceiling; `poly` chooses how many are used).
pub const MAX_VOICES: usize = 16;
/// Oscillators per voice at the highest unison setting.
pub const MAX_UNISON: usize = 7;
/// Control-rate chunk: filters and modulation update this often.
const CHUNK: usize = 16;

/// Every parameter the synth needs, as plain data. Built by the plug-in
/// from its nih-plug parameters and by the standalone from noob-vst-webgui-framework's
/// atomics ([`read_settings`](super::read_settings)); either way the same
/// values reach [`Synth::configure`]. Units are already scaled (percent
/// parameters are 0–1 here).
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Settings {
    /// Factory table index into [`TABLE_NAMES`].
    pub table: usize,
    /// 0..1 morph position.
    pub position: f32,
    /// Coarse tuning, -3..3 octaves.
    pub octave: i32,
    /// Coarse tuning, -12..12 semitones.
    pub semi: i32,
    /// Cents.
    pub fine: f32,
    /// Oscillators per voice, 1..[`MAX_UNISON`].
    pub unison: usize,
    /// Cents between the outermost unison voices.
    pub detune: f32,
    /// 0..1 stereo spread of the unison voices.
    pub width: f32,
    /// 0..1 oscillator level before the filter.
    pub osc_level: f32,
    /// Random oscillator start phases on each note.
    pub phase_random: bool,
    /// 0..1 level of the sine sub oscillator.
    pub sub_level: f32,
    /// 1 or 2 octaves below.
    pub sub_octave: u8,
    /// Filter response.
    pub filter_mode: FilterMode,
    /// Filter cutoff in Hz before modulation.
    pub cutoff: f32,
    /// 0..1 filter resonance.
    pub resonance: f32,
    /// -1..1: filter envelope amount, in units of 6 octaves.
    pub filter_env: f32,
    /// 0..1 keyboard tracking of the cutoff.
    pub key_track: f32,
    /// Amplitude envelope.
    pub amp: AdsrParams,
    /// Filter envelope.
    pub filt: AdsrParams,
    /// LFO rate in Hz.
    pub lfo_rate: f32,
    /// LFO shape, index into [`LFO_SHAPES`](super::LFO_SHAPES).
    pub lfo_shape: usize,
    /// -1..1 -> wavetable position.
    pub lfo_pos: f32,
    /// Octaves -> cutoff.
    pub lfo_cutoff: f32,
    /// Semitones -> pitch.
    pub lfo_pitch: f32,
    /// Restart the LFO phase on every note.
    pub lfo_retrig: bool,
    /// 0..1 how much velocity drives level.
    pub vel_amp: f32,
    /// Seconds of portamento between notes.
    pub glide_s: f32,
    /// Output gain, dB.
    pub master_db: f32,
    /// Polyphony limit.
    pub poly: usize,
}

/// The initial sound: one oscillator on Basic Shapes, an 8 kHz LP 12 with
/// a little envelope and key tracking, 8 voices, -6 dB. Both hosts derive
/// their parameter defaults from this.
impl Default for Settings {
    fn default() -> Self {
        Settings {
            table: 0,
            position: 0.0,
            octave: 0,
            semi: 0,
            fine: 0.0,
            unison: 1,
            detune: 15.0,
            width: 0.5,
            osc_level: 0.8,
            phase_random: true,
            sub_level: 0.0,
            sub_octave: 1,
            filter_mode: FilterMode::Lp12,
            cutoff: 8000.0,
            resonance: 0.15,
            filter_env: 0.4,
            key_track: 0.5,
            amp: AdsrParams::default(),
            filt: AdsrParams {
                attack_s: 0.005,
                decay_s: 0.4,
                sustain: 0.3,
                release_s: 0.4,
            },
            lfo_rate: 2.0,
            lfo_shape: 0,
            lfo_pos: 0.0,
            lfo_cutoff: 0.0,
            lfo_pitch: 0.0,
            lfo_retrig: false,
            vel_amp: 0.7,
            glide_s: 0.0,
            master_db: -6.0,
            poly: 8,
        }
    }
}

/// One voice slot. Plain `Copy` data so the voice vector can be built
/// with `vec![Voice::default(); MAX_VOICES]` and reset by assignment.
#[derive(Clone, Copy, Debug)]
struct Voice {
    /// Sounding (any envelope stage but idle).
    active: bool,
    /// MIDI note that started the voice; `note_off` matches on it.
    note: u8,
    /// 0..1 velocity, applied through `vel_amp`.
    velocity: f32,
    /// Allocation counter, lower = older; used for stealing.
    age: u64,
    /// Oscillator phases, 0..1.
    phases: [f32; MAX_UNISON],
    /// Sub oscillator phase, 0..1.
    sub_phase: f32,
    /// Amplitude envelope.
    amp: Adsr,
    /// Filter envelope.
    filt: Adsr,
    /// Per-voice filter (mono path).
    filter: VoiceFilter,
    /// Current (gliding) note, fractional.
    pitch: f32,
    /// Semitones per sample while gliding, 0 when settled.
    glide_rate: f32,
    /// Last applied gain (envelope × velocity), for the `voices` stream.
    level: f32,
    /// Phase increments per sample for each unison oscillator.
    incs: [f32; MAX_UNISON],
    /// Equal-power `(left, right)` gains for each unison oscillator.
    pans: [(f32, f32); MAX_UNISON],
    /// Mip level chosen for the current pitch.
    mip: usize,
    /// Sub oscillator phase increment per sample.
    sub_inc: f32,
}

impl Default for Voice {
    fn default() -> Self {
        Voice {
            active: false,
            note: 0,
            velocity: 0.0,
            age: 0,
            phases: [0.0; MAX_UNISON],
            sub_phase: 0.0,
            amp: Adsr::default(),
            filt: Adsr::default(),
            filter: VoiceFilter::default(),
            pitch: 60.0,
            glide_rate: 0.0,
            level: 0.0,
            incs: [0.0; MAX_UNISON],
            pans: [(
                std::f32::consts::FRAC_1_SQRT_2,
                std::f32::consts::FRAC_1_SQRT_2,
            ); MAX_UNISON],
            mip: 0,
            sub_inc: 0.0,
        }
    }
}

/// The synthesizer: wavetables, voices, LFO and the current [`Settings`].
/// Create it off the audio thread with [`new`](Self::new); everything else
/// is real-time safe.
pub struct Synth {
    /// Sample rate in Hz.
    sr: f32,
    /// Every factory wavetable, in [`TABLE_NAMES`] order.
    tables: Vec<Wavetable>,
    /// The voice slots, `MAX_VOICES` of them.
    voices: Vec<Voice>,
    /// The global LFO.
    lfo: Lfo,
    /// Current settings.
    s: Settings,
    /// Next value for `Voice::age`.
    next_age: u64,
    /// xorshift state for random phases.
    rng: u32,
    /// Last note-on pitch, the glide start for the next note.
    last_note: f32,
    /// Pitch bend in semitones.
    pitch_bend: f32,
    /// Wavetable position after LFO modulation, for the `modulation` stream.
    live_position: f32,
    /// LFO output of the last chunk, for the `modulation` stream.
    lfo_value: f32,
}

/// Equal-tempered MIDI note (fractional allowed) to Hz, A4 = 440.
#[inline]
fn midi_to_hz(note: f32) -> f32 {
    440.0 * (2.0f32).powf((note - 69.0) / 12.0)
}

/// Highest oscillator frequency, as a fraction of the sample rate.
///
/// The tuning controls stack: ±3 octaves, ±12 semitones, ±100 cents, ±12
/// semitones of pitch LFO and ±2 of bend all add to the note, so a high key
/// with everything at maximum asks for a fundamental far above the sample
/// rate. Nothing up there is musically useful — the output is fold-down
/// whatever we do — but leaving it unclamped let the phase increment exceed
/// one, and an `f32` accumulator advancing by more than a cycle per sample
/// loses its fractional bits and eventually freezes, silencing the voice
/// for good while it still holds its slot. Clamping just under Nyquist
/// keeps the increment below one and the voice alive and merely wrong.
const MAX_PITCH_RATIO: f32 = 0.499;

impl Synth {
    /// Builds every factory wavetable; do this off the audio thread. Starts
    /// with [`Settings::default`] and no sounding voices.
    pub fn new(sr: f32) -> Self {
        let mut s = Synth {
            sr,
            tables: Wavetable::all_factory(),
            voices: vec![Voice::default(); MAX_VOICES],
            lfo: Lfo::default(),
            s: Settings::default(),
            next_age: 1,
            rng: 0x1234_5678,
            last_note: 60.0,
            pitch_bend: 0.0,
            live_position: 0.0,
            lfo_value: 0.0,
        };
        s.apply_settings();
        s
    }

    /// Current sample rate in Hz.
    pub fn sample_rate(&self) -> f32 {
        self.sr
    }

    /// Change the sample rate: silences every voice and recomputes the
    /// envelope and LFO rates.
    pub fn set_sample_rate(&mut self, sr: f32) {
        self.sr = sr;
        self.all_notes_off();
        self.apply_settings();
    }

    /// The selected wavetable.
    pub fn table(&self) -> &Wavetable {
        &self.tables[self.s.table.min(self.tables.len() - 1)]
    }

    /// Factory table names, in `wt_table` index order.
    pub fn table_names(&self) -> &'static [&'static str] {
        &TABLE_NAMES
    }

    /// The settings currently in effect.
    pub fn settings(&self) -> &Settings {
        &self.s
    }

    /// Apply a new snapshot. Returns `true` when the wavetable selection
    /// changed (the caller may want to republish the preview).
    ///
    /// Compares by value first, so calling this every block with an
    /// unchanged snapshot costs one struct comparison. Envelope and LFO
    /// coefficients are recomputed here; voice pitch, filter and unison
    /// data are refreshed in the next control-rate chunk of
    /// [`render`](Self::render).
    pub fn configure(&mut self, s: &Settings) -> bool {
        if *s == self.s {
            return false;
        }
        let table_changed = s.table != self.s.table;
        self.s = *s;
        self.apply_settings();
        table_changed
    }

    /// Recompute the per-sample coefficients that depend on settings and
    /// sample rate: LFO rate and shape, and both envelopes of every voice
    /// (sounding ones included, so envelope edits are heard immediately).
    fn apply_settings(&mut self) {
        let sr = self.sr;
        self.lfo.set(self.s.lfo_rate, self.s.lfo_shape, sr);
        for v in &mut self.voices {
            v.amp.set(&self.s.amp, sr);
            v.filt.set(&self.s.filt, sr);
        }
    }

    /// xorshift32 white noise in 0..1, for random start phases.
    #[inline]
    fn white(&mut self) -> f32 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        x as f32 / u32::MAX as f32
    }

    /// Start a note.
    ///
    /// Allocation: if fewer than `poly` voices are sounding, the first free
    /// slot is used; otherwise the oldest sounding voice is stolen and
    /// restarts from silence (a hard steal, no crossfade). The new voice
    /// gets random or zero phases, reset envelopes and filter, and glides
    /// from the previous note when `glide_s` is above 1 ms and the note
    /// differs. With `lfo_retrig` the LFO restarts. Velocity is clamped to
    /// 0..1.
    pub fn note_on(&mut self, note: u8, velocity: f32) {
        let poly = self.s.poly.clamp(1, MAX_VOICES);
        let active = self.voices.iter().filter(|v| v.active).count();
        // Pick a free voice, or steal the oldest when at the polyphony limit.
        let idx = if active < poly {
            self.voices
                .iter()
                .position(|v| !v.active)
                .unwrap_or_else(|| self.oldest_voice())
        } else {
            self.oldest_voice()
        };
        let glide_from = self.last_note;
        let phase_random = self.s.phase_random;
        let mut phases = [0.0f32; MAX_UNISON];
        if phase_random {
            for p in &mut phases {
                *p = self.white();
            }
        }
        let v = &mut self.voices[idx];
        v.active = true;
        v.note = note;
        v.velocity = velocity.clamp(0.0, 1.0);
        v.age = self.next_age;
        self.next_age += 1;
        v.phases = phases;
        v.sub_phase = 0.0;
        v.amp.reset();
        v.filt.reset();
        v.amp.note_on();
        v.filt.note_on();
        v.filter.reset();
        if self.s.glide_s > 0.001 && glide_from != note as f32 {
            v.pitch = glide_from;
            v.glide_rate = (note as f32 - glide_from) / (self.s.glide_s * self.sr);
        } else {
            v.pitch = note as f32;
            v.glide_rate = 0.0;
        }
        self.last_note = note as f32;
        if self.s.lfo_retrig {
            self.lfo.retrigger();
        }
    }

    /// The active voice that started longest ago (the one to steal). Only
    /// sounding voices count; slot 0 if none is sounding.
    fn oldest_voice(&self) -> usize {
        let mut best = 0;
        let mut age = u64::MAX;
        for (i, v) in self.voices.iter().enumerate() {
            if v.active && v.age < age {
                age = v.age;
                best = i;
            }
        }
        best
    }

    /// Release every sounding voice that plays `note`: both envelopes enter
    /// their release stage, and the voice frees itself once the amplitude
    /// envelope reaches zero.
    pub fn note_off(&mut self, note: u8) {
        for v in self
            .voices
            .iter_mut()
            .filter(|v| v.active && v.note == note)
        {
            v.amp.note_off();
            v.filt.note_off();
        }
    }

    /// Silence everything immediately, without a release (reset, CC 120 /
    /// 123, sample-rate change).
    pub fn all_notes_off(&mut self) {
        for v in &mut self.voices {
            v.active = false;
            v.amp.reset();
            v.filt.reset();
            v.level = 0.0;
        }
    }

    /// Pitch bend in semitones, applied to every voice from the next
    /// control-rate chunk on.
    pub fn set_pitch_bend(&mut self, semitones: f32) {
        self.pitch_bend = semitones;
    }

    /// Number of sounding voices.
    pub fn active_voices(&self) -> usize {
        self.voices.iter().filter(|v| v.active).count()
    }

    /// Per-voice `[level..., note...]` (2 × MAX_VOICES) for the UI: level is
    /// the last applied gain (0 for idle slots), note is the MIDI note or
    /// `-1` for idle slots.
    pub fn voice_state(&self, out: &mut [f32]) {
        for (i, v) in self.voices.iter().enumerate() {
            if i < out.len() {
                out[i] = if v.active { v.level } else { 0.0 };
            }
            if MAX_VOICES + i < out.len() {
                out[MAX_VOICES + i] = if v.active { v.note as f32 } else { -1.0 };
            }
        }
    }

    /// Wavetable position of the last chunk, LFO included (0..1).
    pub fn live_position(&self) -> f32 {
        self.live_position
    }

    /// LFO output of the last chunk (-1..1).
    pub fn lfo_value(&self) -> f32 {
        self.lfo_value
    }

    /// Render one block (overwrites `l` and `r`).
    ///
    /// Per control-rate chunk of `CHUNK` samples:
    ///
    /// 1. advance the LFO and derive the modulated wavetable position;
    /// 2. for every active voice: step the glide, compute the note
    ///    (`pitch + octave·12 + semi + fine/100 + lfo_pitch·lfo + bend`),
    ///    pick the mip level for the highest detuned oscillator, set each
    ///    unison oscillator's increment (`spread · detune / 2` cents) and
    ///    equal-power pan (`spread · width`), the sub increment, and the
    ///    filter cutoff `cutoff · 2^(filter_env·6·env + lfo_cutoff·lfo +
    ///    key_track·(pitch − 60)/12)` clamped to 20 Hz – 20 kHz.
    ///
    /// Then per sample: advance both envelopes (a voice whose amplitude
    /// envelope went idle is freed), sum the unison oscillators into mid
    /// and side with the `1/√n` gain and `osc_level`, add the sub to mid,
    /// filter mid, apply `envelope × (1 − vel_amp·(1 − velocity))`, and
    /// write `mid ± side`. The master gain is applied once per sample to
    /// the mix.
    pub fn render(&mut self, l: &mut [f32], r: &mut [f32]) {
        let n = l.len().min(r.len());
        l[..n].iter_mut().for_each(|v| *v = 0.0);
        r[..n].iter_mut().for_each(|v| *v = 0.0);
        let sr = self.sr;
        let s = self.s;
        let master = 10f32.powf(s.master_db / 20.0);
        let unison = s.unison.clamp(1, MAX_UNISON);
        let unison_gain = 1.0 / (unison as f32).sqrt();
        let table_index = s.table.min(self.tables.len() - 1);
        let sub_ratio = if s.sub_octave >= 2 { 0.25 } else { 0.5 };

        let mut start = 0;
        while start < n {
            let len = CHUNK.min(n - start);
            // Control rate: LFO, per-voice pitch / filter / position.
            let lfo = self.lfo.advance(len);
            self.lfo_value = lfo;
            let position = (s.position + s.lfo_pos * lfo).clamp(0.0, 1.0);
            self.live_position = position;
            let table = &self.tables[table_index];

            for v in self.voices.iter_mut().filter(|v| v.active) {
                if v.glide_rate != 0.0 {
                    v.pitch += v.glide_rate * len as f32;
                    if (v.glide_rate > 0.0 && v.pitch >= v.note as f32)
                        || (v.glide_rate < 0.0 && v.pitch <= v.note as f32)
                    {
                        v.pitch = v.note as f32;
                        v.glide_rate = 0.0;
                    }
                }
                let note = v.pitch
                    + s.octave as f32 * 12.0
                    + s.semi as f32
                    + s.fine / 100.0
                    + s.lfo_pitch * lfo
                    + self.pitch_bend;
                let f0 = midi_to_hz(note).clamp(1.0, sr * MAX_PITCH_RATIO);
                // The mip level has to cover the highest unison voice, which
                // sits at half the detune (see the `spread` below) and whose
                // frequency ratio is `2^(cents/1200)`.
                let top = f0 * (2.0f32).powf(s.detune * 0.5 / 1200.0);
                v.mip = Wavetable::mip_for(top, sr);
                for k in 0..unison {
                    let spread = if unison > 1 {
                        (2.0 * k as f32 / (unison - 1) as f32) - 1.0
                    } else {
                        0.0
                    };
                    let cents = spread * s.detune * 0.5;
                    v.incs[k] = f0 * (2.0f32).powf(cents / 1200.0) / sr;
                    let pan = spread * s.width;
                    let theta = (pan + 1.0) * std::f32::consts::FRAC_PI_4;
                    v.pans[k] = (theta.cos(), theta.sin());
                }
                v.sub_inc = f0 * sub_ratio / sr;
                let fenv = v.filt.value();
                let key = (v.pitch - 60.0) / 12.0 * s.key_track;
                let oct = s.filter_env * 6.0 * fenv + s.lfo_cutoff * lfo + key;
                let cutoff = (s.cutoff * (2.0f32).powf(oct)).clamp(20.0, 20_000.0);
                v.filter.set(s.filter_mode, cutoff, s.resonance, sr);
            }

            // Audio rate.
            for i in start..start + len {
                let mut sl = 0.0f32;
                let mut sr_ = 0.0f32;
                for v in self.voices.iter_mut().filter(|v| v.active) {
                    let amp = v.amp.next();
                    v.filt.next();
                    if v.amp.is_idle() {
                        v.active = false;
                        v.level = 0.0;
                        continue;
                    }
                    let mut osc_l = 0.0f32;
                    let mut osc_r = 0.0f32;
                    for k in 0..unison {
                        let x = table.sample(v.mip, position, v.phases[k]);
                        // `fract` rather than one subtraction: a single
                        // subtraction cannot bring an increment of a whole
                        // cycle or more back into range, and the accumulator
                        // then grows until it loses its fractional bits.
                        v.phases[k] = (v.phases[k] + v.incs[k]).fract();
                        osc_l += x * v.pans[k].0;
                        osc_r += x * v.pans[k].1;
                    }
                    // `SQRT_2` calibrates the dB dial. The equal-power pan
                    // puts a centred voice at 0.7071 in each channel, and
                    // averaging them back to mid loses that 0.7071 again,
                    // so without this the master dial read 3.01 dB below
                    // its own label at every setting.
                    let g = std::f32::consts::SQRT_2 * 0.5 * unison_gain * s.osc_level;
                    let mut mono = (osc_l + osc_r) * g;
                    let side = (osc_l - osc_r) * g;
                    if s.sub_level > 0.0 {
                        mono += (2.0 * std::f32::consts::PI * v.sub_phase).sin() * s.sub_level;
                        v.sub_phase = (v.sub_phase + v.sub_inc).fract();
                    }
                    let filtered = v.filter.process(mono);
                    let vel = 1.0 - s.vel_amp * (1.0 - v.velocity);
                    let g = amp * vel;
                    v.level = g;
                    sl += (filtered + side) * g;
                    sr_ += (filtered - side) * g;
                }
                l[i] = sl * master;
                r[i] = sr_ * master;
            }
            start += len;
        }
    }
}

#[cfg(test)]
#[allow(clippy::field_reassign_with_default)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn render_seconds(synth: &mut Synth, secs: f32) -> (Vec<f32>, Vec<f32>) {
        let n = (secs * 48000.0) as usize;
        let mut l = vec![0.0; n];
        let mut r = vec![0.0; n];
        for c in (0..n).step_by(256) {
            let e = (c + 256).min(n);
            let (ls, rs) = (&mut l[c..e], &mut r[c..e]);
            synth.render(ls, rs);
        }
        (l, r)
    }

    #[test]
    fn a_note_makes_sound_at_its_pitch_and_releases() {
        let mut s = Synth::new(48000.0);
        let mut st = Settings::default();
        st.cutoff = 20000.0;
        st.filter_env = 0.0;
        st.amp.release_s = 0.01;
        st.phase_random = false;
        st.vel_amp = 0.0;
        s.configure(&st);
        s.note_on(69, 1.0);
        let (l, _) = render_seconds(&mut s, 0.5);
        let rms = (l.iter().map(|v| v * v).sum::<f32>() / l.len() as f32).sqrt();
        assert!(rms > 0.1, "{rms}");
        // Zero crossings of a 440 Hz sine over the last 0.25 s: ~220.
        let tail = &l[l.len() / 2..];
        let crossings = tail
            .windows(2)
            .filter(|w| w[0] <= 0.0 && w[1] > 0.0)
            .count();
        assert!((crossings as i32 - 110).abs() <= 3, "{crossings}");
        s.note_off(69);
        let (l2, _) = render_seconds(&mut s, 0.2);
        assert!(l2[l2.len() - 1].abs() < 1e-4);
        assert_eq!(s.active_voices(), 0);
    }

    #[test]
    fn polyphony_limit_steals_the_oldest() {
        let mut s = Synth::new(48000.0);
        let mut st = Settings::default();
        st.poly = 2;
        s.configure(&st);
        s.note_on(60, 1.0);
        s.note_on(64, 1.0);
        s.note_on(67, 1.0);
        assert_eq!(s.active_voices(), 2);
        let mut state = vec![0.0; MAX_VOICES * 2];
        s.voice_state(&mut state);
        let notes: Vec<i32> = state[MAX_VOICES..]
            .iter()
            .map(|v| *v as i32)
            .filter(|n| *n >= 0)
            .collect();
        assert!(notes.contains(&64) && notes.contains(&67) && !notes.contains(&60));
    }

    #[test]
    fn unison_and_width_produce_a_stereo_signal() {
        let mut s = Synth::new(48000.0);
        let mut st = Settings::default();
        st.unison = 5;
        st.detune = 40.0;
        st.width = 1.0;
        st.phase_random = false;
        s.configure(&st);
        s.note_on(48, 1.0);
        let (l, r) = render_seconds(&mut s, 0.3);
        let diff: f32 = l.iter().zip(&r).map(|(a, b)| (a - b).abs()).sum::<f32>() / l.len() as f32;
        assert!(diff > 0.01, "{diff}");
        // Width 0 must collapse to mono, which the old test never checked.
        let mut s2 = Synth::new(48000.0);
        st.width = 0.0;
        s2.configure(&st);
        s2.note_on(48, 1.0);
        let (l2, r2) = render_seconds(&mut s2, 0.3);
        let mono: f32 =
            l2.iter().zip(&r2).map(|(a, b)| (a - b).abs()).sum::<f32>() / l2.len() as f32;
        assert!(mono < 1e-6, "width 0 should be mono, channel diff {mono}");
    }

    /// The tuning controls stack high enough to ask for a fundamental far
    /// above Nyquist. Nothing up there is musical, but the voice must stay
    /// bounded and alive rather than freezing: an `f32` phase advanced by
    /// more than a cycle per sample used to lose its fractional bits and
    /// stick at a constant, silencing the voice for good while it still
    /// held its slot.
    #[test]
    fn extreme_tuning_does_not_freeze_a_voice() {
        let mut s = Synth::new(48000.0);
        let mut st = Settings::default();
        st.octave = 3;
        st.semi = 12;
        st.fine = 100.0;
        st.lfo_pitch = 12.0;
        st.phase_random = false;
        s.configure(&st);
        s.note_on(127, 1.0);
        // Long enough that the old accumulator had frozen solid.
        let (l, _) = render_seconds(&mut s, 30.0);
        assert!(l.iter().all(|v| v.is_finite()));
        let w = 4800;
        let tail = &l[l.len() - w..];
        let distinct = tail
            .iter()
            .map(|v| v.to_bits())
            .collect::<HashSet<_>>()
            .len();
        assert!(
            distinct > 100,
            "voice froze: only {distinct} distinct sample values in the last 0.1 s"
        );
        assert_eq!(s.active_voices(), 1);
    }

    /// The sub oscillator sounds an octave (or two) below the note.
    #[test]
    fn sub_oscillator_tracks_the_note_an_octave_down() {
        for (sub_octave, ratio) in [(1u8, 0.5f32), (2, 0.25)] {
            let mut s = Synth::new(48000.0);
            let mut st = Settings::default();
            st.osc_level = 0.0;
            st.sub_level = 1.0;
            st.sub_octave = sub_octave;
            st.cutoff = 20000.0;
            st.filter_env = 0.0;
            st.key_track = 0.0;
            st.vel_amp = 0.0;
            st.phase_random = false;
            s.configure(&st);
            s.note_on(69, 1.0); // 440 Hz
            let (l, _) = render_seconds(&mut s, 0.5);
            let tail = &l[l.len() / 2..];
            let crossings = tail
                .windows(2)
                .filter(|w| w[0] <= 0.0 && w[1] > 0.0)
                .count();
            let want = (440.0 * ratio * 0.25) as usize;
            assert!(
                (crossings as i32 - want as i32).abs() <= 3,
                "sub octave {sub_octave}: {crossings} crossings, expected about {want}"
            );
        }
    }

    /// Glide is linear in semitones and reaches the target in the stated
    /// time; measured by where the pitch has got to half way through.
    #[test]
    fn glide_takes_the_stated_time() {
        let mut s = Synth::new(48000.0);
        let mut st = Settings::default();
        st.glide_s = 0.5;
        st.cutoff = 20000.0;
        st.filter_env = 0.0;
        st.key_track = 0.0;
        st.vel_amp = 0.0;
        st.phase_random = false;
        s.configure(&st);
        s.note_on(60, 1.0);
        let _ = render_seconds(&mut s, 0.3);
        s.note_on(72, 1.0); // an octave up, 0.5 s of glide
        let _ = render_seconds(&mut s, 0.25); // half way: 6 semitones
        let (l, _) = render_seconds(&mut s, 0.05);
        let crossings = l.windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count();
        let hz = crossings as f32 / 0.05;
        let want = 440.0 * 2f32.powf((66.0 - 69.0) / 12.0); // MIDI 66
        assert!(
            (hz / want - 1.0).abs() < 0.08,
            "half way through a 0.5 s glide: {hz:.0} Hz, expected about {want:.0}"
        );
    }

    /// Pitch bend of +2 semitones raises the pitch by that much.
    #[test]
    fn pitch_bend_moves_the_pitch_two_semitones() {
        let mut s = Synth::new(48000.0);
        let mut st = Settings::default();
        st.cutoff = 20000.0;
        st.filter_env = 0.0;
        st.key_track = 0.0;
        st.vel_amp = 0.0;
        st.phase_random = false;
        s.configure(&st);
        s.set_pitch_bend(2.0);
        s.note_on(69, 1.0);
        let (l, _) = render_seconds(&mut s, 0.4);
        let tail = &l[l.len() / 2..];
        let secs = tail.len() as f32 / 48000.0;
        let hz = tail
            .windows(2)
            .filter(|w| w[0] <= 0.0 && w[1] > 0.0)
            .count() as f32
            / secs;
        let want = 440.0 * 2f32.powf(2.0 / 12.0);
        assert!(
            (hz / want - 1.0).abs() < 0.02,
            "bend +2 st: {hz:.1} Hz, expected {want:.1}"
        );
    }

    /// Master gain is a dB dial: every 6 dB doubles the output, and the
    /// label is absolute, not merely relative. A full-scale frame at unity
    /// oscillator level and 0 dB master must peak at full scale; it used to
    /// come out 3.01 dB shy because the pan law's centre attenuation was
    /// never compensated.
    #[test]
    fn master_gain_is_in_decibels() {
        let level = |db: f32| {
            let mut s = Synth::new(48000.0);
            let mut st = Settings::default();
            st.master_db = db;
            st.cutoff = 20000.0;
            st.filter_env = 0.0;
            st.key_track = 0.0;
            st.vel_amp = 0.0;
            st.phase_random = false;
            s.configure(&st);
            s.note_on(60, 1.0);
            let (l, _) = render_seconds(&mut s, 0.3);
            l.iter().fold(0.0f32, |m, v| m.max(v.abs()))
        };
        let a = level(-12.0);
        let b = level(-6.0);
        let c = level(0.0);
        assert!((b / a / 2.0 - 1.0).abs() < 0.05, "-12 to -6 dB: {a} -> {b}");
        assert!((c / b / 2.0 - 1.0).abs() < 0.05, "-6 to 0 dB: {b} -> {c}");

        // Absolute calibration: a full-scale sine frame, unity oscillator
        // level, centred, 0 dB master.
        let mut s = Synth::new(48000.0);
        let mut st = Settings::default();
        st.master_db = 0.0;
        st.osc_level = 1.0;
        st.width = 0.0;
        st.cutoff = 20000.0;
        st.filter_env = 0.0;
        st.key_track = 0.0;
        st.resonance = 0.0;
        st.vel_amp = 0.0;
        st.phase_random = false;
        st.amp.sustain = 1.0;
        st.amp.decay_s = 0.001;
        s.configure(&st);
        s.note_on(45, 1.0);
        let (l, _) = render_seconds(&mut s, 0.5);
        let peak = l[l.len() / 2..].iter().fold(0.0f32, |m, v| m.max(v.abs()));
        assert!(
            (20.0 * peak.log10()).abs() < 0.6,
            "0 dB master should peak near full scale, got {:.2} dBFS",
            20.0 * peak.log10()
        );
    }

    #[test]
    fn output_stays_finite_under_load() {
        let mut s = Synth::new(48000.0);
        let mut st = Settings::default();
        st.unison = 7;
        st.poly = 16;
        st.resonance = 1.0;
        st.lfo_rate = 20.0;
        st.lfo_pos = 1.0;
        st.lfo_cutoff = 4.0;
        s.configure(&st);
        for n in 0..16 {
            s.note_on(40 + n * 3, 1.0);
        }
        let (l, r) = render_seconds(&mut s, 0.5);
        assert!(l.iter().chain(&r).all(|v| v.is_finite()));
        // The old bound of 20.0 was 26 dB above anything this patch can
        // produce, so it constrained nothing. Sixteen voices of seven
        // detuned oscillators through a resonant filter peaks near 2; allow
        // headroom for the resonant peak but not for a runaway.
        let peak = l.iter().chain(&r).fold(0.0f32, |m, v| m.max(v.abs()));
        assert!((0.5..=6.0).contains(&peak), "peak {peak}");
    }

    /// Key tracking scales the cutoff with the note, pivoting at MIDI 60:
    /// full tracking two octaves below quarters it, which removes most of
    /// the output; the pivot note itself must not move at all.
    #[test]
    fn key_tracking_scales_the_cutoff_about_middle_c() {
        // Energy above 1 kHz relative to the whole signal. Total RMS will
        // not do: it is dominated by the fundamental, which passes at any
        // of these cutoffs, so it barely moves when the filter closes.
        let energy = |note: u8, key_track: f32| {
            let mut s = Synth::new(48000.0);
            let mut st = Settings::default();
            st.cutoff = 2000.0;
            st.filter_env = 0.0;
            st.key_track = key_track;
            st.position = 0.5; // a harmonically rich frame
            st.vel_amp = 0.0;
            st.phase_random = false;
            s.configure(&st);
            s.note_on(note, 1.0);
            let (l, _) = render_seconds(&mut s, 0.4);
            let n = 1 << 14;
            let tail = &l[l.len() - n..];
            let mut planner = rustfft::FftPlanner::<f32>::new();
            let fft = planner.plan_fft_forward(n);
            let mut buf: Vec<rustfft::num_complex::Complex<f32>> = tail
                .iter()
                .enumerate()
                .map(|(i, v)| {
                    let a = 2.0 * std::f32::consts::PI * i as f32 / n as f32;
                    rustfft::num_complex::Complex::new(v * (0.5 - 0.5 * a.cos()), 0.0)
                })
                .collect();
            fft.process(&mut buf);
            let bin_hz = 48000.0 / n as f32;
            let (mut hi, mut all) = (0.0f32, 0.0f32);
            for (k, c) in buf[..n / 2].iter().enumerate() {
                let p = c.norm_sqr();
                all += p;
                if k as f32 * bin_hz > 1000.0 {
                    hi += p;
                }
            }
            hi / all.max(1e-12)
        };
        // MIDI 60 is the pivot: tracking must not change it.
        let pivot_off = energy(60, 0.0);
        let pivot_on = energy(60, 1.0);
        assert!(
            (pivot_on / pivot_off - 1.0).abs() < 0.02,
            "MIDI 60 is the pivot and must not move: {pivot_off} -> {pivot_on}"
        );
        // Two octaves below, full tracking takes 2 kHz down to 500 Hz.
        let off_low = energy(36, 0.0);
        let on_low = energy(36, 1.0);
        assert!(
            on_low < off_low * 0.5,
            "key tracking should darken MIDI 36: {off_low} -> {on_low}"
        );
    }
}
