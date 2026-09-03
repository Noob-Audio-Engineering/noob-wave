//! ADSR envelope: linear attack, exponential decay and release.
//!
//! Stages ([`Stage`]):
//!
//! 1. **Attack** — rises linearly from the current value to 1 in
//!    `attack_s` seconds (`1 / (attack × sr)` per sample).
//! 2. **Decay** — falls exponentially towards `sustain` with the
//!    coefficient `exp(-SETTLE / (decay × sr))`, so it is within
//!    `e⁻⁵ ≈ 0.7 %` of the target after `decay_s`; it snaps to the sustain
//!    level once within `1e-4`.
//! 3. **Sustain** — holds `sustain` until note off.
//! 4. **Release** — multiplies by `exp(-SETTLE / (release × sr))` per
//!    sample from wherever the value is (a note off during the attack or
//!    decay releases from that level, no jump), and becomes idle below
//!    `1e-4`.
//!
//! # Release time and the voice slot
//!
//! `release_s` is the time to fall within about 0.7 % of zero, not the time
//! to reach the `1e-4` idle threshold: that takes roughly **1.7×** longer,
//! and a voice only frees its slot once it is idle. A 10 second release
//! therefore occupies a slot for about 17 seconds, which matters when
//! `poly` is low. The gap buys an inaudible tail rather than a click, so it
//! is deliberate, but it is not what the dial says.
//!
//! [`Adsr::reset`] drops to idle at 0 immediately; the synth calls it
//! before `note_on` on a stolen voice, so a steal restarts from silence.
//! Everything is per-sample state with precomputed coefficients;
//! [`Adsr::set`] is the only method that does transcendental math, and the
//! synth calls it only when the settings change.

/// Envelope times and level, as the parameters expose them.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct AdsrParams {
    /// Attack time in seconds (0 → 1).
    pub attack_s: f32,
    /// Decay time in seconds (1 → sustain).
    pub decay_s: f32,
    /// Sustain level, 0..1.
    pub sustain: f32,
    /// Release time in seconds (sustain → 0).
    pub release_s: f32,
}

/// 5 ms attack, 200 ms decay, 80 % sustain, 300 ms release: the amplitude
/// envelope's defaults.
impl Default for AdsrParams {
    fn default() -> Self {
        AdsrParams {
            attack_s: 0.005,
            decay_s: 0.2,
            sustain: 0.8,
            release_s: 0.3,
        }
    }
}

/// Envelope stage.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Stage {
    /// Not sounding; output is 0.
    #[default]
    Idle,
    /// Rising linearly to 1.
    Attack,
    /// Falling exponentially to the sustain level.
    Decay,
    /// Holding the sustain level.
    Sustain,
    /// Falling exponentially to 0, then idle.
    Release,
}

/// Envelope state for one voice. `Copy` so voices can be reset by
/// assignment; coefficients come from [`set`](Self::set).
#[derive(Clone, Copy, Debug, Default)]
pub struct Adsr {
    /// Current stage.
    stage: Stage,
    /// Current output, 0..1.
    value: f32,
    /// Attack increment per sample.
    att_inc: f32,
    /// Decay coefficient per sample (towards sustain).
    dec_coef: f32,
    /// Release coefficient per sample (towards 0).
    rel_coef: f32,
    /// Sustain level, 0..1.
    sustain: f32,
}

/// Exponential stages reach their target within this factor of the time
/// (`e⁻⁵`, about 0.7 % remaining).
const SETTLE: f32 = 5.0;

impl Adsr {
    /// Recompute the coefficients for `p` at sample rate `sr`. Times are
    /// floored (0.5 ms attack, 1 ms decay / release) so a zero never
    /// divides. Safe to call while the envelope runs.
    pub fn set(&mut self, p: &AdsrParams, sr: f32) {
        self.att_inc = 1.0 / (p.attack_s.max(0.0005) * sr);
        self.dec_coef = (-SETTLE / (p.decay_s.max(0.001) * sr)).exp();
        self.rel_coef = (-SETTLE / (p.release_s.max(0.001) * sr)).exp();
        self.sustain = p.sustain.clamp(0.0, 1.0);
    }

    /// Enter the attack stage from the current value (0 after `reset`).
    pub fn note_on(&mut self) {
        self.stage = Stage::Attack;
    }

    /// Enter the release stage, unless idle.
    pub fn note_off(&mut self) {
        if self.stage != Stage::Idle {
            self.stage = Stage::Release;
        }
    }

    /// Idle at 0 immediately, without a release.
    pub fn reset(&mut self) {
        self.stage = Stage::Idle;
        self.value = 0.0;
    }

    /// Advance one sample and return the new value.
    // Not an iterator: an envelope has no end and yields by reference to its stage.
    #[allow(clippy::should_implement_trait)]
    #[inline]
    pub fn next(&mut self) -> f32 {
        match self.stage {
            Stage::Idle => {}
            Stage::Attack => {
                self.value += self.att_inc;
                if self.value >= 1.0 {
                    self.value = 1.0;
                    self.stage = Stage::Decay;
                }
            }
            Stage::Decay => {
                self.value = self.sustain + (self.value - self.sustain) * self.dec_coef;
                if (self.value - self.sustain).abs() < 1e-4 {
                    self.value = self.sustain;
                    self.stage = Stage::Sustain;
                }
            }
            Stage::Sustain => self.value = self.sustain,
            Stage::Release => {
                self.value *= self.rel_coef;
                if self.value < 1e-4 {
                    self.value = 0.0;
                    self.stage = Stage::Idle;
                }
            }
        }
        self.value
    }

    /// Current value without advancing.
    #[inline]
    pub fn value(&self) -> f32 {
        self.value
    }
    /// Current stage.
    #[inline]
    pub fn stage(&self) -> Stage {
        self.stage
    }
    /// `true` before any note and once the release has finished.
    #[inline]
    pub fn is_idle(&self) -> bool {
        self.stage == Stage::Idle
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stages_progress_and_release_reaches_zero() {
        let sr = 48000.0;
        let mut e = Adsr::default();
        e.set(
            &AdsrParams {
                attack_s: 0.01,
                decay_s: 0.05,
                sustain: 0.5,
                release_s: 0.02,
            },
            sr,
        );
        e.note_on();
        for _ in 0..480 {
            e.next();
        }
        assert!((e.value() - 1.0).abs() < 1e-3);
        for _ in 0..4800 {
            e.next();
        }
        assert_eq!(e.stage(), Stage::Sustain);
        assert!((e.value() - 0.5).abs() < 1e-3);
        e.note_off();
        for _ in 0..4800 {
            e.next();
        }
        assert!(e.is_idle());
        assert_eq!(e.value(), 0.0);
    }
}
