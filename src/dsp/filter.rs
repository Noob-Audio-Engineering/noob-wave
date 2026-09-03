//! State-variable filter (topology-preserving transform), 12 dB/oct per
//! stage; modes combine one or two stages.
//!
//! [`Svf`] is the zero-delay-feedback SVF in Andrew Simper's (Cytomic)
//! formulation: `g = tan(π fc / sr)` warps the cutoff so it is exact at
//! any sample rate, `k = 1/Q` is the damping, and the two integrator
//! states are updated with the `2v − ic` trapezoidal step. One stage
//! yields low-, band- and high-pass outputs at once. The resonance
//! parameter maps `0..1` to `k = 2 − 1.98 r`, i.e. from `Q = 0.5` (no
//! peak) to `Q = 50` (a sharp peak just short of self-oscillation).
//!
//! [`VoiceFilter`] selects a response ([`FilterMode`]): `LP 12`, `BP` and
//! `HP` use one stage; `LP 24` cascades a second stage on the low-pass
//! output with 70 % of the resonance so the peak does not double up.
//! Cutoff is clamped to `10 Hz .. 0.45 × sr` for stability.
//!
//! # Band-pass gain
//!
//! The band output is the raw `s / (s² + ks + 1)`, whose gain **at cutoff
//! is `Q`**, not unity: −6 dB at resonance 0, rising to +34 dB at
//! resonance 1. It is deliberately not normalised, so anything drawing the
//! response has to apply the same convention. A curve that divides the
//! peak out draws a flat line while the filter delivers 34 dB of gain.
//!
//! Coefficients are recomputed by `set` (one `tan` and a few divides), so
//! the synth calls it once per control-rate chunk, not per sample.

use std::f32::consts::PI;

/// Labels for the `filter_mode` choice parameter, in [`FilterMode`] order.
pub const FILTER_MODE_NAMES: [&str; 4] = ["LP 12", "LP 24", "BP", "HP"];

/// Filter response.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum FilterMode {
    /// 12 dB/oct low-pass (one stage).
    #[default]
    Lp12,
    /// 24 dB/oct low-pass (two stages).
    Lp24,
    /// Band-pass (one stage).
    Bp,
    /// High-pass (one stage).
    Hp,
}

impl FilterMode {
    /// From the parameter index; unknown indices fall back to `Lp12`.
    pub fn from_index(i: usize) -> FilterMode {
        match i {
            1 => FilterMode::Lp24,
            2 => FilterMode::Bp,
            3 => FilterMode::Hp,
            _ => FilterMode::Lp12,
        }
    }
}

/// One 12 dB/oct TPT state-variable stage.
#[derive(Clone, Copy, Default, Debug)]
pub struct Svf {
    /// First integrator state.
    ic1: f32,
    /// Second integrator state.
    ic2: f32,
    /// Warped cutoff, `tan(π fc / sr)`.
    g: f32,
    /// Damping, `1/Q`.
    k: f32,
    /// Precomputed step coefficient `1 / (1 + g (g + k))`.
    a1: f32,
    /// Precomputed step coefficient `g · a1`.
    a2: f32,
    /// Precomputed step coefficient `g · a2`.
    a3: f32,
}

impl Svf {
    /// `resonance` 0..1 (1 = self-oscillation edge). `cutoff_hz` is clamped
    /// to `10 .. 0.45 × sr`.
    pub fn set(&mut self, cutoff_hz: f32, resonance: f32, sr: f32) {
        let fc = cutoff_hz.clamp(10.0, sr * 0.45);
        self.g = (PI * fc / sr).tan();
        self.k = 2.0 - 1.98 * resonance.clamp(0.0, 1.0);
        self.a1 = 1.0 / (1.0 + self.g * (self.g + self.k));
        self.a2 = self.g * self.a1;
        self.a3 = self.g * self.a2;
    }

    /// Returns `(low, band, high)` for one input sample; the three always
    /// satisfy `low + k·band + high == x`.
    #[inline]
    pub fn process(&mut self, x: f32) -> (f32, f32, f32) {
        let v3 = x - self.ic2;
        let v1 = self.a1 * self.ic1 + self.a2 * v3;
        let v2 = self.ic2 + self.a2 * self.ic1 + self.a3 * v3;
        self.ic1 = 2.0 * v1 - self.ic1;
        self.ic2 = 2.0 * v2 - self.ic2;
        let low = v2;
        let band = v1;
        let high = x - self.k * v1 - v2;
        (low, band, high)
    }

    /// Clear the integrator states (start of a note).
    pub fn reset(&mut self) {
        self.ic1 = 0.0;
        self.ic2 = 0.0;
    }
}

/// The per-voice filter: one or two [`Svf`] stages and the selected
/// response.
#[derive(Clone, Copy, Default, Debug)]
pub struct VoiceFilter {
    /// First stage, used by every mode.
    s1: Svf,
    /// Second stage, used by `Lp24` only.
    s2: Svf,
    /// Selected response.
    mode: FilterMode,
}

impl VoiceFilter {
    /// Set mode, cutoff (Hz) and resonance (0..1) at sample rate `sr`; the
    /// second stage of `Lp24` gets 70 % of the resonance.
    pub fn set(&mut self, mode: FilterMode, cutoff_hz: f32, resonance: f32, sr: f32) {
        self.mode = mode;
        self.s1.set(cutoff_hz, resonance, sr);
        if mode == FilterMode::Lp24 {
            self.s2.set(cutoff_hz, resonance * 0.7, sr);
        }
    }

    /// Filter one sample according to the mode.
    #[inline]
    pub fn process(&mut self, x: f32) -> f32 {
        let (lp, bp, hp) = self.s1.process(x);
        match self.mode {
            FilterMode::Lp12 => lp,
            FilterMode::Lp24 => self.s2.process(lp).0,
            FilterMode::Bp => bp,
            FilterMode::Hp => hp,
        }
    }

    /// Clear both stages.
    pub fn reset(&mut self) {
        self.s1.reset();
        self.s2.reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rms_at(mode: FilterMode, cutoff: f32, freq: f32) -> f32 {
        let sr = 48000.0;
        let mut f = VoiceFilter::default();
        f.set(mode, cutoff, 0.0, sr);
        let n = 48000;
        let mut sum = 0.0;
        for i in 0..n {
            let x = (2.0 * PI * freq * i as f32 / sr).sin();
            let y = f.process(x);
            if i > n / 2 {
                sum += y * y;
            }
        }
        (sum / (n / 2) as f32).sqrt() * std::f32::consts::SQRT_2
    }

    /// Gain at `freq` in dB.
    fn db_at(mode: FilterMode, cutoff: f32, freq: f32, res: f32) -> f32 {
        let sr = 48000.0;
        let mut f = VoiceFilter::default();
        f.set(mode, cutoff, res, sr);
        let n = 96000;
        let mut sum = 0.0f64;
        for i in 0..n {
            // Double-precision phase: an f32 accumulator floors the deep
            // stop-band measurements this test makes.
            let x = (2.0 * std::f64::consts::PI * freq as f64 * i as f64 / sr as f64).sin() as f32;
            let y = f.process(x);
            if i > n / 2 {
                sum += (y as f64) * (y as f64);
            }
        }
        let rms = (sum / (n / 2) as f64).sqrt() * std::f64::consts::SQRT_2;
        20.0 * rms.max(1e-12).log10() as f32
    }

    #[test]
    fn lowpass_passes_low_and_cuts_high() {
        assert!((rms_at(FilterMode::Lp12, 1000.0, 100.0) - 1.0).abs() < 0.05);
        let hi = rms_at(FilterMode::Lp12, 1000.0, 8000.0);
        assert!(hi < 0.03, "{hi}");
        // Doubling the order doubles the stop-band attenuation in dB. Three
        // octaves above cutoff LP 12 is about -38 dB and LP 24 about -76,
        // so the ratio is far tighter than the old `< 0.1` allowed.
        let hi24 = rms_at(FilterMode::Lp24, 1000.0, 8000.0);
        assert!(hi24 < hi * 0.02, "{hi24} vs {hi}");
    }

    /// The slopes the module claims: 12 and 24 dB/oct nominal, measured
    /// between 4 and 8 kHz on a 1 kHz cutoff. The bilinear warp makes the
    /// measured figure slightly steeper than nominal.
    #[test]
    fn slopes_are_12_and_24_db_per_octave() {
        let lp12 = db_at(FilterMode::Lp12, 1000.0, 8000.0, 0.0)
            - db_at(FilterMode::Lp12, 1000.0, 4000.0, 0.0);
        assert!((-14.0..=-12.0).contains(&lp12), "LP 12 slope {lp12} dB/oct");
        let lp24 = db_at(FilterMode::Lp24, 1000.0, 8000.0, 0.0)
            - db_at(FilterMode::Lp24, 1000.0, 4000.0, 0.0);
        assert!((-28.0..=-24.0).contains(&lp24), "LP 24 slope {lp24} dB/oct");
    }

    #[test]
    fn highpass_rejects_below_cutoff() {
        assert!((rms_at(FilterMode::Hp, 1000.0, 10000.0) - 1.0).abs() < 0.05);
        assert!(rms_at(FilterMode::Hp, 1000.0, 50.0) < 0.01);
    }

    /// The band output is not normalised: its gain at cutoff is `Q`, which
    /// the resonance maps from 0.5 to 50. Asserting the actual figure is
    /// the point — a loose bound here once admitted both this convention
    /// and a peak-normalised one, and the page drew the wrong one for every
    /// resonance setting while the test passed.
    #[test]
    fn bandpass_gain_at_cutoff_is_q() {
        // k = 2 - 1.98 r, Q = 1/k, gain at cutoff = Q.
        for (res, q) in [(0.0f32, 0.5f32), (0.5, 1.0 / 1.01), (0.9, 1.0 / 0.218)] {
            let got = rms_at2(FilterMode::Bp, 1000.0, 1000.0, res);
            let want = q;
            assert!(
                (got / want - 1.0).abs() < 0.05,
                "resonance {res}: gain {got}, expected Q = {want}"
            );
        }
        // ... and it rolls off either side of centre.
        assert!(rms_at(FilterMode::Bp, 1000.0, 50.0) < 0.1);
    }

    /// `rms_at` with a resonance setting.
    fn rms_at2(mode: FilterMode, cutoff: f32, freq: f32, res: f32) -> f32 {
        let sr = 48000.0;
        let mut f = VoiceFilter::default();
        f.set(mode, cutoff, res, sr);
        let n = 96000;
        let mut sum = 0.0f64;
        for i in 0..n {
            let x = (2.0 * std::f64::consts::PI * freq as f64 * i as f64 / sr as f64).sin() as f32;
            let y = f.process(x);
            if i > n / 2 {
                sum += (y as f64) * (y as f64);
            }
        }
        ((sum / (n / 2) as f64).sqrt() * std::f64::consts::SQRT_2) as f32
    }

    /// Resonance 1.0 must not self-oscillate: the impulse tail decays.
    #[test]
    fn stable_at_full_resonance() {
        let mut f = VoiceFilter::default();
        f.set(FilterMode::Lp12, 1000.0, 1.0, 48000.0);
        let mut peak_late = 0.0f32;
        for i in 0..96000 {
            let y = f.process(if i == 0 { 1.0 } else { 0.0 });
            assert!(y.is_finite());
            if i > 48000 {
                peak_late = peak_late.max(y.abs());
            }
        }
        assert!(peak_late < 1e-6, "tail {peak_late} after 1 s");
    }
}
