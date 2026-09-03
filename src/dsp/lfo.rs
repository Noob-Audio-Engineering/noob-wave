//! Low-frequency oscillator, bipolar output.
//!
//! One global [`Lfo`] runs at control rate: [`Lfo::advance`] steps the
//! phase by a whole chunk of samples and returns the value for that chunk,
//! so the waveform is evaluated 16 samples at a time (the synth's control
//! chunk). Shapes ([`LFO_SHAPES`]): sine, triangle, rising saw, square
//! (50 %), and sample-and-hold, which draws a new random level each time
//! the phase wraps. [`Lfo::retrigger`] restarts the phase (and draws a new
//! S&H value) when a note starts with `lfo_retrig` on; otherwise the LFO
//! free-runs across notes.

use std::f32::consts::PI;

/// Labels for the `lfo_shape` choice parameter, in shape-index order.
pub const LFO_SHAPES: [&str; 5] = ["Sine", "Triangle", "Saw", "Square", "S&H"];

/// The LFO state.
#[derive(Clone, Copy, Debug)]
pub struct Lfo {
    /// Phase, 0..1.
    phase: f32,
    /// Phase increment per sample.
    inc: f32,
    /// Shape index into [`LFO_SHAPES`].
    shape: usize,
    /// Output of the last `advance`, -1..1.
    value: f32,
    /// Current sample-and-hold level.
    held: f32,
    /// xorshift state for sample-and-hold.
    rng: u32,
}

/// Stopped (rate 0), sine, phase 0.
impl Default for Lfo {
    fn default() -> Self {
        Lfo {
            phase: 0.0,
            inc: 0.0,
            shape: 0,
            value: 0.0,
            held: 0.0,
            rng: 0x2545_F491,
        }
    }
}

impl Lfo {
    /// Rate in Hz and shape index at sample rate `sr`; the shape is clamped
    /// to the known ones and a negative rate stops the LFO.
    pub fn set(&mut self, rate_hz: f32, shape: usize, sr: f32) {
        self.inc = rate_hz.max(0.0) / sr;
        self.shape = shape.min(LFO_SHAPES.len() - 1);
    }

    /// Restart at phase 0 and draw a fresh sample-and-hold level.
    pub fn retrigger(&mut self) {
        self.phase = 0.0;
        self.held = self.white();
    }

    /// xorshift32 white noise in -1..1.
    #[inline]
    fn white(&mut self) -> f32 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        (x as f32 / u32::MAX as f32) * 2.0 - 1.0
    }

    /// Advance by `n` samples and return the new value in -1..1.
    ///
    /// The phase wraps at 1; for sample-and-hold a wrap draws the next
    /// level, and the very first call draws one so the output does not sit
    /// at 0 until the first wrap.
    #[inline]
    pub fn advance(&mut self, n: usize) -> f32 {
        let before = self.phase;
        self.phase += self.inc * n as f32;
        if self.phase >= 1.0 {
            self.phase -= self.phase.floor();
            if self.shape == 4 {
                self.held = self.white();
            }
        }
        let p = self.phase;
        self.value = match self.shape {
            0 => (2.0 * PI * p).sin(),
            1 => 1.0 - 4.0 * (p - 0.5).abs(),
            2 => 2.0 * p - 1.0,
            3 => {
                if p < 0.5 {
                    1.0
                } else {
                    -1.0
                }
            }
            _ => {
                if before == 0.0 && self.held == 0.0 {
                    self.held = self.white();
                }
                self.held
            }
        };
        self.value
    }

    /// Output of the last `advance`.
    #[inline]
    pub fn value(&self) -> f32 {
        self.value
    }
}
