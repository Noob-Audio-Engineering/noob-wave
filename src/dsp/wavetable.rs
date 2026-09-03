//! Mipmapped wavetables. A table is `FRAMES` single cycles of `FRAME_LEN`
//! samples; each frame exists in `MIPS` band-limited versions (level `m`
//! keeps the harmonics up to `FRAME_LEN / 2 >> m`), so playback picks the
//! level whose highest harmonic stays below Nyquist.
//!
//! The ladder reaches a single harmonic, so any fundamental below Nyquist
//! has an alias-free level to play from — the whole MIDI range and then
//! some. Only tuning a note *above* Nyquist folds, which no setting makes
//! musically useful; see [`super::synth`] for the clamp that keeps that
//! case bounded.
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
//! | 0 | 1023 | 23 Hz |
//! | 1 | 512 | 47 Hz |
//! | 2 | 256 | 94 Hz |
//! | 3 | 128 | 188 Hz |
//! | 4 | 64 | 375 Hz |
//! | 5 | 32 | 750 Hz |
//! | 6 | 16 | 1.5 kHz |
//! | 7 | 8 | 3 kHz |
//! | 8 | 4 | 6 kHz |
//! | 9 | 2 | 12 kHz |
//! | 10 | 1 | 24 kHz |
//!
//! Level 0 holds 1023 rather than 1024 because harmonic 1024 lands on the
//! Nyquist bin, which a Hermitian spectrum cannot carry as a conjugate
//! pair. At a 23 Hz fundamental that harmonic is itself at Nyquist, so
//! nothing audible is lost.
//!
//! [`Wavetable::mip_for`] picks the first level whose top harmonic is under
//! Nyquist for a given fundamental. The top of the ladder keeps one
//! harmonic, so the highest MIDI note (12.5 kHz) still plays a clean
//! fundamental at 48 kHz; before the ladder was extended it stopped at four
//! harmonics and the top octave folded loudly, on some tables above the
//! wanted note.
//!
//! # Memory and playback
//!
//! Every frame is stored with one extra sample (the first repeated), so a
//! linear interpolation between sample `i` and `i + 1` never wraps. A
//! lookup ([`Wavetable::sample`]) is bilinear: linear in phase within a
//! frame and linear between the two frames around the morph position. A
//! full table is `11 × 32 × 2049` floats (about 2.9 MB); all six factory
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
///
/// Eleven levels take the ladder down to a single harmonic
/// (`1024 >> 10 == 1`), which is what makes the top of the keyboard
/// alias-free: a level keeping `h` harmonics is clean only up to
/// `sr / 2 / h`, so four harmonics (the old top level) ran out at 6 kHz,
/// well below the 12.5 kHz of MIDI 127.
pub const MIPS: usize = 11;
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
                // `HARMONICS - 1`: harmonic 1024 would land on the Nyquist
                // bin, where `buf[k]` and `buf[FRAME_LEN - k]` are the same
                // slot and the conjugate would overwrite the value.
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
    /// FFT per table, frame and level (about 2.1 k transforms of 2048
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

    /// Magnitude spectrum of one stored cycle, bin `k` = harmonic `k`.
    fn harmonics_of(frame: &[f32]) -> Vec<f32> {
        let mut planner = FftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(FRAME_LEN);
        let mut buf: Vec<Complex<f32>> = frame[..FRAME_LEN]
            .iter()
            .map(|v| Complex::new(*v, 0.0))
            .collect();
        fft.process(&mut buf);
        let g = 2.0 / FRAME_LEN as f32;
        buf[..=HARMONICS].iter().map(|c| c.norm() * g).collect()
    }

    /// Every level really is band-limited to its stated harmonic count.
    /// The old version measured a smoothness proxy chosen to pass and never
    /// looked at the harmonic content at all.
    #[test]
    fn mip_levels_remove_high_harmonics() {
        for table in 0..TABLE_NAMES.len() {
            let t = Wavetable::factory(table);
            for m in 0..MIPS {
                let limit = HARMONICS >> m;
                let mag = harmonics_of(t.frame(m, FRAMES - 1));
                let peak = mag.iter().cloned().fold(0.0f32, f32::max).max(1e-9);
                for (k, v) in mag.iter().enumerate().skip(limit + 1) {
                    let rel = 20.0 * (v / peak).max(1e-12).log10();
                    assert!(
                        rel < -80.0,
                        "table {table} level {m}: harmonic {k} is {rel:.1} dB \
                         below peak, above the level's limit of {limit}"
                    );
                }
            }
        }
    }

    /// The ladder covers the whole audible range: for any fundamental up to
    /// Nyquist there is a level whose top harmonic still fits underneath.
    ///
    /// The old version asserted `mip_for(10000, 48000) >= 8`, which was the
    /// cap that *caused* the top octave to fold, and checked the inequality
    /// by feeding `mip_for`'s own answer back into it.
    #[test]
    fn every_fundamental_below_nyquist_has_an_alias_free_level() {
        let sr = 48000.0;
        let nyquist = sr * 0.5;
        // 12 543.85 Hz is MIDI 127, the top of the keyboard.
        for f0 in [
            20.0f32, 55.0, 440.0, 3000.0, 6000.0, 8000.0, 12_543.85, 23_900.0,
        ] {
            let m = Wavetable::mip_for(f0, sr);
            let top = (HARMONICS >> m) as f32 * f0;
            assert!(
                top <= nyquist,
                "f0 {f0}: level {m} keeps {} harmonics, topmost at {top} Hz, above Nyquist",
                HARMONICS >> m
            );
        }
        // Low notes must not be robbed of harmonics by an over-eager level.
        assert_eq!(Wavetable::mip_for(20.0, sr), 0);
        // The ladder has to reach a single harmonic for the above to hold.
        assert_eq!(HARMONICS >> (MIPS - 1), 1);
    }

    /// Play a table at `f0` and measure the loudest component that is not
    /// near a harmonic, relative to the loudest that is. This is the test
    /// that was missing: the plug-in shipped with the top octave folding at
    /// up to +68 dBc on the Digital table, and nothing measured it.
    #[test]
    fn top_of_the_keyboard_does_not_alias() {
        let sr = 48000.0f32;
        let n = 1 << 15;
        let mut planner = FftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(n);
        for table in 0..TABLE_NAMES.len() {
            let t = Wavetable::factory(table);
            for &note in &[110.0f32, 120.0, 127.0] {
                let f0 = 440.0 * 2f32.powf((note - 69.0) / 12.0);
                let mip = Wavetable::mip_for(f0, sr);
                let inc = f0 / sr;
                let mut phase = 0.0f32;
                let mut buf: Vec<Complex<f32>> = Vec::with_capacity(n);
                for i in 0..n {
                    let x = t.sample(mip, 0.5, phase);
                    phase = (phase + inc).fract();
                    // Blackman-Harris: -92 dB sidelobes, so window leakage
                    // cannot be mistaken for a spur.
                    let a = 2.0 * PI * i as f32 / n as f32;
                    let w = 0.35875 - 0.48829 * a.cos() + 0.14128 * (2.0 * a).cos()
                        - 0.01168 * (3.0 * a).cos();
                    buf.push(Complex::new(x * w, 0.0));
                }
                fft.process(&mut buf);
                let bin_hz = sr / n as f32;
                let (mut wanted, mut spur, mut spur_hz) = (0.0f32, 0.0f32, 0.0f32);
                for (k, c) in buf[..n / 2].iter().enumerate() {
                    let f = k as f32 * bin_hz;
                    if f < 20.0 || f > sr * 0.5 - 200.0 {
                        continue;
                    }
                    let m = c.norm();
                    let h = (f / f0).round().max(1.0);
                    let near = (1200.0 * (f / (h * f0)).log2()).abs() < 25.0
                        || (f - h * f0).abs() < 6.0 * bin_hz;
                    if near {
                        wanted = wanted.max(m);
                    } else if m > spur {
                        spur = m;
                        spur_hz = f;
                    }
                }
                let dbc = 20.0 * (spur / wanted.max(1e-12)).max(1e-12).log10();
                assert!(
                    dbc < -60.0,
                    "table {} ({}) at MIDI {note}: worst spur {dbc:.1} dBc at {spur_hz:.0} Hz",
                    table,
                    TABLE_NAMES[table]
                );
            }
        }
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
