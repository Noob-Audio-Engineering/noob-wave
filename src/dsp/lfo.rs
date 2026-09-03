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

#[cfg(test)]
mod tests {
    use super::*;

    /// Step the LFO in control chunks for `secs` and return every value.
    fn run(shape: usize, rate: f32, secs: f32) -> Vec<f32> {
        let sr = 48000.0;
        let chunk = 16;
        let mut lfo = Lfo::default();
        lfo.set(rate, shape, sr);
        let steps = (secs * sr / chunk as f32) as usize;
        (0..steps).map(|_| lfo.advance(chunk)).collect()
    }

    /// The rate dial is in Hz. Measured from the span between the first and
    /// last rising zero crossing, not from a crossing count over the whole
    /// render: counting is off by one whenever the render does not end
    /// exactly on a cycle boundary, which is a fault in the measurement
    /// rather than in the LFO.
    #[test]
    fn rate_is_in_hertz() {
        let chunk_secs = 16.0 / 48000.0;
        for rate in [0.02f32, 0.5, 2.0, 8.0, 20.0] {
            let secs = (8.0 / rate).clamp(1.0, 500.0);
            let v = run(0, rate, secs);
            let idx: Vec<usize> = v
                .windows(2)
                .enumerate()
                .filter(|(_, w)| w[0] <= 0.0 && w[1] > 0.0)
                .map(|(i, _)| i)
                .collect();
            assert!(idx.len() >= 3, "rate {rate}: only {} crossings", idx.len());
            let cycles = (idx.len() - 1) as f32;
            let span = (idx[idx.len() - 1] - idx[0]) as f32 * chunk_secs;
            let measured = cycles / span;
            assert!(
                (measured / rate - 1.0).abs() < 0.02,
                "rate {rate} Hz measured {measured:.4} Hz"
            );
        }
    }

    /// Every shape is bipolar and stays inside -1..1.
    #[test]
    fn every_shape_is_bipolar_and_bounded() {
        for shape in 0..LFO_SHAPES.len() {
            let v = run(shape, 4.0, 2.0);
            let lo = v.iter().cloned().fold(f32::INFINITY, f32::min);
            let hi = v.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
            assert!(
                v.iter().all(|x| x.is_finite()),
                "{} not finite",
                LFO_SHAPES[shape]
            );
            assert!(
                (-1.001..=1.001).contains(&lo) && (-1.001..=1.001).contains(&hi),
                "{} ranges {lo}..{hi}",
                LFO_SHAPES[shape]
            );
            assert!(
                lo < -0.5 && hi > 0.5,
                "{} is not bipolar: {lo}..{hi}",
                LFO_SHAPES[shape]
            );
        }
    }

    /// The shapes are what they claim: a triangle peaks mid-cycle, a saw
    /// rises monotonically, a square takes only two values.
    #[test]
    fn shapes_have_their_stated_form() {
        let tri = run(1, 1.0, 1.0);
        let peak_at = tri
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .unwrap()
            .0;
        let frac = peak_at as f32 / tri.len() as f32;
        assert!(
            (0.4..0.6).contains(&frac),
            "triangle peaks at {frac} of a cycle"
        );

        let saw = run(2, 1.0, 0.9);
        assert!(
            saw.windows(2).all(|w| w[1] >= w[0] - 1e-6),
            "saw should rise monotonically within a cycle"
        );

        let sq = run(3, 1.0, 1.0);
        assert!(
            sq.iter().all(|v| (v.abs() - 1.0).abs() < 1e-6),
            "square should only ever be +1 or -1"
        );
    }

    /// Rate 0 (and a negative rate) stops the LFO rather than running it
    /// backwards or dividing by zero.
    #[test]
    fn zero_or_negative_rate_stops_the_lfo() {
        for rate in [0.0f32, -5.0] {
            let v = run(2, rate, 1.0);
            assert!(
                v.iter().all(|x| (x - v[0]).abs() < 1e-9),
                "rate {rate} moved"
            );
            assert!(v.iter().all(|x| x.is_finite()));
        }
    }

    /// Sample-and-hold holds a level for a whole cycle and draws a new one
    /// when the phase wraps.
    #[test]
    fn sample_and_hold_changes_once_per_cycle() {
        let v = run(4, 4.0, 2.0); // 8 cycles
        let mut changes = 0;
        for w in v.windows(2) {
            if (w[1] - w[0]).abs() > 1e-9 {
                changes += 1;
            }
        }
        assert!(
            (6..=10).contains(&changes),
            "{changes} changes over 8 cycles"
        );
    }

    /// Retriggering restarts the phase, which is what `lfo_retrig` promises.
    #[test]
    fn retrigger_restarts_the_phase() {
        let mut lfo = Lfo::default();
        lfo.set(1.0, 2, 48000.0); // rising saw
        for _ in 0..1000 {
            lfo.advance(16);
        }
        let mid = lfo.value();
        lfo.retrigger();
        let after = lfo.advance(16);
        assert!(after < mid, "saw should restart low: {mid} -> {after}");
        assert!(after < -0.9, "phase did not reset: {after}");
    }
}
