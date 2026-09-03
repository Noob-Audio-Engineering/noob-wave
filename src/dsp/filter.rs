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

    #[test]
    fn lowpass_passes_low_and_cuts_high() {
        assert!((rms_at(FilterMode::Lp12, 1000.0, 100.0) - 1.0).abs() < 0.05);
        let hi = rms_at(FilterMode::Lp12, 1000.0, 8000.0);
        assert!(hi < 0.03, "{hi}");
        let hi24 = rms_at(FilterMode::Lp24, 1000.0, 8000.0);
        assert!(hi24 < hi * 0.1, "{hi24} vs {hi}");
    }

    #[test]
    fn highpass_and_bandpass() {
        assert!((rms_at(FilterMode::Hp, 1000.0, 10000.0) - 1.0).abs() < 0.05);
        assert!(rms_at(FilterMode::Hp, 1000.0, 50.0) < 0.01);
        let centre = rms_at(FilterMode::Bp, 1000.0, 1000.0);
        assert!(centre > 0.4 && centre <= 1.0, "{centre}");
        assert!(rms_at(FilterMode::Bp, 1000.0, 50.0) < 0.1);
    }
}
