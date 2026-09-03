//! Mipmapped wavetables. A table is `FRAMES` single cycles of `FRAME_LEN`
//! samples; each frame exists in `MIPS` band-limited versions (level `m`
//! keeps the harmonics up to `FRAME_LEN / 2 >> m`), so playback picks the
//! level whose highest harmonic stays below Nyquist and never aliases.
//!
//! Factory tables are defined as harmonic spectra and rendered with an
//! inverse FFT, which makes the mipmaps exact: level `m` is the same
//! spectrum with every bin above `HARMONICS >> m` zeroed, not a filtered
//! copy of level 0. One table (`Digital`) is defined in the time domain and
//! analysed with a forward FFT first, so it goes through the same path.
//!
//! # Levels
//!
//! | level | harmonics kept | alias-free up to (48 kHz) |
//! |---|---|---|
//! | 0 | 1024 | 23 Hz |
//! | 1 | 512 | 47 Hz |
//! | 2 | 256 | 94 Hz |
//! | 3 | 128 | 188 Hz |
//! | 4 | 64 | 375 Hz |
//! | 5 | 32 | 750 Hz |
//! | 6 | 16 | 1.5 kHz |
//! | 7 | 8 | 3 kHz |
//! | 8 | 4 | 6 kHz |
//!
//! [`Wavetable::mip_for`] picks the first level whose top harmonic is
//! under Nyquist for a given fundamental; above 6 kHz level 8 is used and
//! its few harmonics fold, which is inaudible at that pitch.
//!
//! # Memory and playback
//!
//! Every frame is stored with one extra sample (the first repeated), so a
//! linear interpolation between sample `i` and `i + 1` never wraps. A
//! lookup ([`Wavetable::sample`]) is bilinear: linear in phase within a
//! frame and linear between the two frames around the morph position. A
//! full table is `9 × 32 × 2049` floats (about 2.4 MB); all six factory
//! tables are built once in [`Synth::new`](super::Synth::new), off the
//! audio thread, and shared read-only by every voice.

use std::f32::consts::PI;
use std::sync::Arc;

use rustfft::num_complex::Complex;
use rustfft::{Fft, FftPlanner};

/// Samples per single-cycle frame at level 0.
pub const FRAME_LEN: usize = 2048;
/// Harmonics a level-0 frame can hold (`FRAME_LEN / 2`).
pub const HARMONICS: usize = FRAME_LEN / 2;
/// Mip levels per frame; level `m` keeps `HARMONICS >> m` harmonics.
pub const MIPS: usize = 9;
/// Frames (morph steps) per table.
pub const FRAMES: usize = 32;
/// Samples per frame in the preview the UI draws.
pub const PREVIEW_LEN: usize = 256;

/// Factory table names, in `wt_table` index order.
pub const TABLE_NAMES: [&str; 6] = [
    "Basic Shapes",
    "Harmonics",
    "PWM",
    "Formant",
    "Digital",
    "Bells",
];

/// One wavetable: `frames` single cycles in [`MIPS`] band-limited versions
/// plus a display preview. Immutable after construction; shared by every
/// voice.
pub struct Wavetable {
    /// Display name (one of [`TABLE_NAMES`] for factory tables).
    pub name: &'static str,
    /// Number of frames (morph positions); [`FRAMES`] for factory tables.
    pub frames: usize,
    /// `[mip][frame][FRAME_LEN + 1]`; the extra sample repeats the first so
    /// linear interpolation never needs a wrap.
    data: Vec<f32>,
    /// Level-0 frames downsampled for display: `frames × PREVIEW_LEN`.
    pub preview: Vec<f32>,
}

/// Floats per stored frame: the cycle plus the repeated first sample.
const STRIDE: usize = FRAME_LEN + 1;

impl Wavetable {
    /// Build from one spectrum per frame (bins `0..=HARMONICS`, bin `k` =
    /// harmonic `k`). Frames are normalised to a peak of 1 at level 0.
    ///
    /// For every frame and level the spectrum is copied into a Hermitian
    /// buffer (bin `k` and its conjugate at `FRAME_LEN - k`) up to the
    /// level's harmonic limit, inverse-transformed, and scaled by the
    /// level-0 peak — the same scale for every level, so band-limited
    /// versions keep their natural (lower) amplitude and a pitch sweep does
    /// not jump in loudness between levels. DC (bin 0) is always dropped.
    /// The preview takes every `FRAME_LEN / PREVIEW_LEN`-th level-0 sample.
    pub fn from_spectra(name: &'static str, spectra: &[Vec<Complex<f32>>]) -> Self {
        let frames = spectra.len().max(1);
        let mut planner = FftPlanner::<f32>::new();
        let ifft: Arc<dyn Fft<f32>> = planner.plan_fft_inverse(FRAME_LEN);
        let mut scratch = vec![Complex::default(); ifft.get_inplace_scratch_len()];
        let mut buf = vec![Complex::default(); FRAME_LEN];
        let mut data = vec![0.0f32; MIPS * frames * STRIDE];
        let mut preview = vec![0.0f32; frames * PREVIEW_LEN];
        for (f, spec) in spectra.iter().enumerate() {
            let mut scale = 1.0f32;
            for m in 0..MIPS {
                let limit = HARMONICS >> m;
                buf.iter_mut().for_each(|c| *c = Complex::default());
                for k in 1..=limit.min(HARMONICS - 1) {
                    let c = spec.get(k).copied().unwrap_or_default();
                    buf[k] = c;
                    buf[FRAME_LEN - k] = c.conj();
                }
                ifft.process_with_scratch(&mut buf, &mut scratch);
                let base = (m * frames + f) * STRIDE;
                if m == 0 {
                    let peak = buf.iter().fold(0.0f32, |p, c| p.max(c.re.abs()));
                    scale = if peak > 1e-9 { 1.0 / peak } else { 1.0 };
                }
                for i in 0..FRAME_LEN {
                    data[base + i] = buf[i].re * scale;
                }
                data[base + FRAME_LEN] = data[base];
                if m == 0 {
                    let step = FRAME_LEN / PREVIEW_LEN;
                    for i in 0..PREVIEW_LEN {
                        preview[f * PREVIEW_LEN + i] = data[base + i * step];
                    }
                }
            }
        }
        Wavetable {
            name,
            frames,
            data,
            preview,
        }
    }

    /// The stored cycle for `mip` / `frame`, `FRAME_LEN + 1` samples long
    /// (the last equals the first). Out-of-range indices are clamped.
    #[inline]
    pub fn frame(&self, mip: usize, frame: usize) -> &[f32] {
        let base = (mip.min(MIPS - 1) * self.frames + frame.min(self.frames - 1)) * STRIDE;
        &self.data[base..base + STRIDE]
    }

    /// Bilinear lookup: `position` morphs between frames, `phase` is 0..1.
    ///
    /// The phase is interpolated linearly inside the frame (the repeated
    /// last sample makes the wrap free), then the two frames either side
    /// of `position` are cross-faded. When `position` lands exactly on a
    /// frame the second lookup is skipped.
    #[inline]
    pub fn sample(&self, mip: usize, position: f32, phase: f32) -> f32 {
        let fp = position.clamp(0.0, 1.0) * (self.frames - 1) as f32;
        let fa = fp as usize;
        let fb = (fa + 1).min(self.frames - 1);
        let k = fp - fa as f32;
        let p = phase.rem_euclid(1.0) * FRAME_LEN as f32;
        let i = p as usize;
        let t = p - i as f32;
        let a = self.frame(mip, fa);
        let sa = a[i] + (a[i + 1] - a[i]) * t;
        if k <= 0.0 {
            return sa;
        }
        let b = self.frame(mip, fb);
        let sb = b[i] + (b[i + 1] - b[i]) * t;
        sa + (sb - sa) * k
    }

    /// Mip level whose harmonics all stay below Nyquist for `f0`: the
    /// smallest `m` with `(HARMONICS >> m) × f0 ≤ sr / 2`, capped at
    /// `MIPS - 1`. `f0` is clamped to at least 1 Hz.
    #[inline]
    pub fn mip_for(f0: f32, sr: f32) -> usize {
        let allowed = (sr * 0.5) / f0.max(1.0);
        let mut m = 0;
        while m + 1 < MIPS && (HARMONICS >> m) as f32 > allowed {
            m += 1;
        }
        m
    }

    /// The factory table with the given index into [`TABLE_NAMES`].
    /// Unknown indices build Basic Shapes.
    pub fn factory(index: usize) -> Self {
        let spectra = match index {
            1 => harmonics_table(),
            2 => pwm_table(),
            3 => formant_table(),
            4 => digital_table(),
            5 => bells_table(),
            _ => basic_shapes_table(),
        };
        Wavetable::from_spectra(TABLE_NAMES[index.min(TABLE_NAMES.len() - 1)], &spectra)
    }

    /// Every factory table, in [`TABLE_NAMES`] order. Costs one inverse
    /// FFT per table, frame and level (about 1.7 k transforms of 2048
    /// points); do it off the audio thread.
    pub fn all_factory() -> Vec<Wavetable> {
        (0..TABLE_NAMES.len()).map(Wavetable::factory).collect()
    }
}

// ---------------------------------------------------------------------------
// Factory tables (sine series: bin k = (0, -a_k))
// ---------------------------------------------------------------------------

/// Spectrum of a sine series: bin `k` = `-i·a_k`, so the inverse FFT yields
/// `Σ a_k sin(2π k t)`. Amplitudes beyond [`HARMONICS`] are ignored and DC
/// is left at zero.
fn sine_series(amps: &[f32]) -> Vec<Complex<f32>> {
    let mut v = vec![Complex::default(); HARMONICS + 1];
    for (k, a) in amps.iter().enumerate().take(HARMONICS) {
        if k > 0 {
            v[k] = Complex::new(0.0, -a);
        }
    }
    v
}

/// Harmonic amplitudes (`k = 0..=512`) of the five anchor shapes of the
/// Basic Shapes table: 0 sine, 1 triangle (odd harmonics, alternating
/// sign, `1/k²`), 2 saw (`1/k`, alternating sign), 3 square (odd
/// harmonics, `1/k`), 4 a 25 % pulse (`sin(kπ/4) / k`).
fn shape_amps(shape: usize) -> Vec<f32> {
    let n = 512;
    let mut a = vec![0.0f32; n + 1];
    for k in 1..=n {
        let kf = k as f32;
        a[k] = match shape {
            0 => {
                if k == 1 {
                    1.0
                } else {
                    0.0
                }
            }
            1 => {
                if k % 2 == 1 {
                    (8.0 / (PI * PI)) * if ((k - 1) / 2) % 2 == 0 { 1.0 } else { -1.0 } / (kf * kf)
                } else {
                    0.0
                }
            }
            2 => (2.0 / PI) * if k % 2 == 1 { 1.0 } else { -1.0 } / kf,
            3 => {
                if k % 2 == 1 {
                    (4.0 / PI) / kf
                } else {
                    0.0
                }
            }
            _ => (2.0 / (kf * PI)) * (kf * PI * 0.25).sin(),
        };
    }
    a
}

/// Basic Shapes: 32 frames morphing linearly through the five anchors
/// (sine → triangle → saw → square → pulse). The harmonic amplitudes are
/// interpolated, not the waveforms, so every intermediate frame is still
/// exactly band-limited.
fn basic_shapes_table() -> Vec<Vec<Complex<f32>>> {
    let anchors: Vec<Vec<f32>> = (0..5).map(shape_amps).collect();
    (0..FRAMES)
        .map(|f| {
            let t = f as f32 / (FRAMES - 1) as f32 * (anchors.len() - 1) as f32;
            let i = (t as usize).min(anchors.len() - 2);
            let k = t - i as f32;
            let amps: Vec<f32> = anchors[i]
                .iter()
                .zip(&anchors[i + 1])
                .map(|(a, b)| a + (b - a) * k)
                .collect();
            sine_series(&amps)
        })
        .collect()
}

/// Harmonics: frame `f` contains the first `1 + 2f` harmonics at `1/k`,
/// so the morph adds partials two at a time, from a sine towards a saw.
fn harmonics_table() -> Vec<Vec<Complex<f32>>> {
    (0..FRAMES)
        .map(|f| {
            let count = 1 + f * 2;
            let amps: Vec<f32> = (0..=count)
                .map(|k| if k == 0 { 0.0 } else { 1.0 / k as f32 })
                .collect();
            sine_series(&amps)
        })
        .collect()
}

/// PWM: a pulse whose duty cycle narrows from 50 % to 3 % across the
/// frames (the `sin(kπd) / k` series of a rectangular pulse).
fn pwm_table() -> Vec<Vec<Complex<f32>>> {
    (0..FRAMES)
        .map(|f| {
            let d = 0.5 - 0.47 * f as f32 / (FRAMES - 1) as f32;
            let amps: Vec<f32> = (0..=512)
                .map(|k| {
                    if k == 0 {
                        0.0
                    } else {
                        (2.0 / (k as f32 * PI)) * (k as f32 * PI * d).sin()
                    }
                })
                .collect();
            sine_series(&amps)
        })
        .collect()
}

/// Formant: two resonant peaks (Lorentzian in harmonic number) that rise
/// with the frame — the first from harmonic 2 to 16, the second at about
/// 2.6× that — over a gentle `1/√k` tilt with a small floor, giving
/// vowel-like tones as the position moves.
fn formant_table() -> Vec<Vec<Complex<f32>>> {
    (0..FRAMES)
        .map(|f| {
            let t = f as f32 / (FRAMES - 1) as f32;
            let c1 = 2.0 + 14.0 * t;
            let c2 = c1 * 2.6 + 3.0;
            let amps: Vec<f32> = (0..=128)
                .map(|k| {
                    if k == 0 {
                        return 0.0;
                    }
                    let kf = k as f32;
                    let r1 = 1.0 / (1.0 + ((kf - c1) / 1.4).powi(2));
                    let r2 = 0.6 / (1.0 + ((kf - c2) / 2.2).powi(2));
                    (r1 + r2 + 0.02) / kf.sqrt()
                })
                .collect();
            sine_series(&amps)
        })
        .collect()
}

/// Digital: a hard-synced saw, defined in the time domain and analysed
/// with a forward FFT so the mipmaps come out right. The sync ratio rises
/// from 1 to 4 across the frames, and a slight cosine window softens the
/// reset edge so the top end is not all noise.
fn digital_table() -> Vec<Vec<Complex<f32>>> {
    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(FRAME_LEN);
    let mut scratch = vec![Complex::default(); fft.get_inplace_scratch_len()];
    (0..FRAMES)
        .map(|f| {
            let ratio = 1.0 + 3.0 * f as f32 / (FRAMES - 1) as f32;
            let mut buf: Vec<Complex<f32>> = (0..FRAME_LEN)
                .map(|i| {
                    let ph = i as f32 / FRAME_LEN as f32;
                    let s = (ph * ratio).fract() * 2.0 - 1.0;
                    // Soften the sync edge a little so the top end is not all noise.
                    let w = 1.0 - 0.15 * (2.0 * PI * ph).cos();
                    Complex::new(s * w, 0.0)
                })
                .collect();
            fft.process_with_scratch(&mut buf, &mut scratch);
            let scale = 2.0 / FRAME_LEN as f32;
            let mut spec = vec![Complex::default(); HARMONICS + 1];
            for (k, s) in spec.iter_mut().enumerate().skip(1) {
                *s = buf[k] * scale;
            }
            spec
        })
        .collect()
}

/// Bells: eight partials at harmonic numbers 1 2 3 4 5 7 10 13 with a
/// `1/k^p` roll-off that brightens with the frame (`p` from 1.35 down to
/// 0.6), every second partial at 70 %.
fn bells_table() -> Vec<Vec<Complex<f32>>> {
    const PARTIALS: [usize; 8] = [1, 2, 3, 4, 5, 7, 10, 13];
    (0..FRAMES)
        .map(|f| {
            let t = f as f32 / (FRAMES - 1) as f32;
            let mut amps = vec![0.0f32; 16];
            for (i, &k) in PARTIALS.iter().enumerate() {
                let bright = 1.0 - 0.75 * (1.0 - t);
                amps[k] =
                    (1.0 / (k as f32).powf(1.6 - bright)) * if i % 2 == 0 { 1.0 } else { 0.7 };
            }
            sine_series(&amps)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_are_normalised_and_wrap() {
        let t = Wavetable::factory(0);
        assert_eq!(t.frames, FRAMES);
        for f in 0..t.frames {
            let fr = t.frame(0, f);
            let peak = fr.iter().fold(0.0f32, |p, v| p.max(v.abs()));
            assert!((peak - 1.0).abs() < 1e-3, "frame {f} peak {peak}");
            assert_eq!(fr[0], fr[FRAME_LEN]);
        }
        // Frame 0 of Basic Shapes is a sine.
        let s = t.sample(0, 0.0, 0.25);
        assert!((s.abs() - 1.0).abs() < 0.02, "{s}");
        assert!(t.sample(0, 0.0, 0.5).abs() < 0.02);
    }

    #[test]
    fn mip_levels_remove_high_harmonics() {
        let t = Wavetable::factory(0);
        // Last frame (pulse) has lots of harmonics at level 0 ...
        let level0: f32 = (0..FRAME_LEN)
            .map(|i| t.frame(0, FRAMES - 1)[i].abs())
            .sum::<f32>();
        // ... the top mip keeps only the first few, so it is much smoother.
        let top = t.frame(MIPS - 1, FRAMES - 1);
        let mut max_step = 0.0f32;
        for i in 0..FRAME_LEN {
            max_step = max_step.max((top[i + 1] - top[i]).abs());
        }
        assert!(max_step < 0.05, "top mip has step {max_step}");
        assert!(level0 > 0.0);
    }

    #[test]
    fn mip_selection_tracks_pitch() {
        assert_eq!(Wavetable::mip_for(20.0, 48000.0), 0);
        let m = Wavetable::mip_for(440.0, 48000.0);
        assert!((HARMONICS >> m) as f32 * 440.0 <= 24000.0 + 1.0);
        assert!(Wavetable::mip_for(10000.0, 48000.0) >= 8);
    }

    #[test]
    fn all_factory_tables_build() {
        for (i, t) in Wavetable::all_factory().iter().enumerate() {
            assert_eq!(t.name, TABLE_NAMES[i]);
            assert_eq!(t.preview.len(), t.frames * PREVIEW_LEN);
            assert!(t.preview.iter().all(|v| v.is_finite()));
        }
    }
}
