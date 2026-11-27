//! Second-order IIR sections: RBJ "Audio EQ Cookbook" designs, transposed direct form II.

use crate::flush;
use std::f64::consts::PI;

/// Filter shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FilterType {
    Peaking,
    LowShelf,
    HighShelf,
    LowPass,
    HighPass,
    Notch,
    BandPass,
    AllPass,
    /// Constant-bandwidth cut: `1 + (g − 1)·BP(s)` — the −3 dB width of the band-pass stays
    /// `f/Q` however deep the cut (`gain_db` ≤ 0), unlike the RBJ peaking filter whose cut
    /// widens with depth. Used by the Notch Filter.
    Cut,
    /// First-order (6 dB/oct) Butterworth low-pass: −3 dB at `freq`.
    LowPass1,
    /// First-order (6 dB/oct) Butterworth high-pass: −3 dB at `freq`.
    HighPass1,
    /// Premiere's Parametric Equalizer shelves (measured): a Butterworth shelf of the first
    /// (gentle) or second (steep) order, placed so the curve at `freq` is 3 dB inside the full
    /// gain ([`Coeffs::shelf`]); a cut mirrors a boost in dB. `q` is unused.
    GentleLowShelf,
    GentleHighShelf,
    SteepLowShelf,
    SteepHighShelf,
}

impl FilterType {
    /// Order used by the parametric EQ's `type` choice parameter.
    pub const ALL: [FilterType; 7] =
        [FilterType::Peaking, FilterType::LowShelf, FilterType::HighShelf, FilterType::LowPass, FilterType::HighPass, FilterType::Notch, FilterType::BandPass];
    pub fn from_index(i: usize) -> FilterType {
        Self::ALL[i.min(Self::ALL.len() - 1)]
    }
}

/// Normalised biquad coefficients (`a0 == 1`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Coeffs {
    pub b0: f64,
    pub b1: f64,
    pub b2: f64,
    pub a1: f64,
    pub a2: f64,
}

impl Coeffs {
    /// Pass-through.
    pub const IDENTITY: Coeffs = Coeffs { b0: 1.0, b1: 0.0, b2: 0.0, a1: 0.0, a2: 0.0 };

    fn norm(b0: f64, b1: f64, b2: f64, a0: f64, a1: f64, a2: f64) -> Coeffs {
        Coeffs { b0: b0 / a0, b1: b1 / a0, b2: b2 / a0, a1: a1 / a0, a2: a2 / a0 }
    }

    /// Bilinear transform (pre-warped to `w0`) of an analog section normalised to ω0 = 1:
    /// `(n0·s² + n1·s + n2) / (d0·s² + d1·s + d2)` (n0 = d0 = 0 for a first-order one).
    fn analog(w0: f64, n: [f64; 3], d: [f64; 3]) -> Coeffs {
        let k = (w0 / 2.0).tan().max(1e-9);
        let k2 = k * k;
        let z = |c: [f64; 3]| [c[0] + c[1] * k + c[2] * k2, 2.0 * (c[2] * k2 - c[0]), c[0] - c[1] * k + c[2] * k2];
        let (b, a) = (z(n), z(d));
        if a[0].abs() < 1e-300 {
            return Coeffs::IDENTITY;
        }
        Self::norm(b[0], b[1], b[2], a[0], a[1], a[2])
    }

    /// One of Premiere's shelves: a boost's (numerator, denominator); a cut swaps them.
    ///
    /// A Butterworth shelf of order N (1 gentle, 2 steep) with gain G has
    /// |H|² = (x²ᴺ + G²) / (x²ᴺ + 1) at x = f / pole (a low shelf; a high one is its mirror in
    /// frequency). Premiere places the pole so the curve at `freq` is [`edge_gain`]: 3 dB
    /// inside the full gain. Measured on Premiere Pro 27.0 renders of 30 tones through the
    /// Parametric Equalizer's L (40 Hz) and H (18 kHz) at ±9 dB, gentle and steep: every tone
    /// within 0.012 dB (+9 dB gentle at 40 Hz: 7.46 dB at 25 Hz, 6.00 at 40, 4.18 at 63).
    fn shelf(w0: f64, gain_db: f64, low: bool, steep: bool) -> Coeffs {
        let g = 10f64.powf(gain_db.abs() / 20.0);
        if g <= 1.0 + 1e-9 || !g.is_finite() {
            return Coeffs::IDENTITY;
        }
        let edge2 = edge_gain(g).powi(2);
        // x²ᴺ at `freq`, for the pole at 1
        let ratio = (g * g - edge2) / (edge2 - 1.0);
        if ratio <= 0.0 || !ratio.is_finite() {
            return Coeffs::IDENTITY;
        }
        // the pole, with `freq` at 1 (a high shelf's is above it)
        let x = if steep { ratio.powf(0.25) } else { ratio.sqrt() };
        let p = if low { 1.0 / x } else { x };
        let (r, s2) = (g.sqrt(), std::f64::consts::SQRT_2);
        let (n, d) = match (low, steep) {
            (true, false) => ([0.0, 1.0, g * p], [0.0, 1.0, p]),
            (false, false) => ([0.0, g, p], [0.0, 1.0, p]),
            (true, true) => ([1.0, s2 * r * p, g * p * p], [1.0, s2 * p, p * p]),
            (false, true) => ([g, s2 * r * p, p * p], [1.0, s2 * p, p * p]),
        };
        if gain_db < 0.0 { Self::analog(w0, d, n) } else { Self::analog(w0, n, d) }
    }

    /// RBJ cookbook design. `freq` in Hz (clamped below Nyquist), `q` > 0, `gain_db` used by
    /// peaking and shelving types (shelves use `q` as the shelf "Q", S=1 ≙ Q=1/√2).
    pub fn design(kind: FilterType, sample_rate: f64, freq: f64, q: f64, gain_db: f64) -> Coeffs {
        let f = freq.clamp(1.0, sample_rate * 0.49);
        let q = q.max(1e-3);
        let w0 = 2.0 * PI * f / sample_rate;
        let (sn, cs) = w0.sin_cos();
        let alpha = sn / (2.0 * q);
        let a = 10f64.powf(gain_db / 40.0);
        match kind {
            FilterType::LowPass => Self::norm((1.0 - cs) / 2.0, 1.0 - cs, (1.0 - cs) / 2.0, 1.0 + alpha, -2.0 * cs, 1.0 - alpha),
            FilterType::HighPass => Self::norm((1.0 + cs) / 2.0, -(1.0 + cs), (1.0 + cs) / 2.0, 1.0 + alpha, -2.0 * cs, 1.0 - alpha),
            FilterType::BandPass => Self::norm(alpha, 0.0, -alpha, 1.0 + alpha, -2.0 * cs, 1.0 - alpha),
            FilterType::Notch => Self::norm(1.0, -2.0 * cs, 1.0, 1.0 + alpha, -2.0 * cs, 1.0 - alpha),
            FilterType::AllPass => Self::norm(1.0 - alpha, -2.0 * cs, 1.0 + alpha, 1.0 + alpha, -2.0 * cs, 1.0 - alpha),
            FilterType::Cut => {
                let g = 10f64.powf(gain_db / 20.0);
                Self::norm(1.0 + g * alpha, -2.0 * cs, 1.0 - g * alpha, 1.0 + alpha, -2.0 * cs, 1.0 - alpha)
            }
            FilterType::LowPass1 => Self::analog(w0, [0.0, 0.0, 1.0], [0.0, 1.0, 1.0]),
            FilterType::HighPass1 => Self::analog(w0, [0.0, 1.0, 0.0], [0.0, 1.0, 1.0]),
            FilterType::GentleLowShelf => Self::shelf(w0, gain_db, true, false),
            FilterType::GentleHighShelf => Self::shelf(w0, gain_db, false, false),
            FilterType::SteepLowShelf => Self::shelf(w0, gain_db, true, true),
            FilterType::SteepHighShelf => Self::shelf(w0, gain_db, false, true),
            FilterType::Peaking => Self::norm(1.0 + alpha * a, -2.0 * cs, 1.0 - alpha * a, 1.0 + alpha / a, -2.0 * cs, 1.0 - alpha / a),
            FilterType::LowShelf => {
                let k = 2.0 * a.sqrt() * alpha;
                Self::norm(
                    a * ((a + 1.0) - (a - 1.0) * cs + k),
                    2.0 * a * ((a - 1.0) - (a + 1.0) * cs),
                    a * ((a + 1.0) - (a - 1.0) * cs - k),
                    (a + 1.0) + (a - 1.0) * cs + k,
                    -2.0 * ((a - 1.0) + (a + 1.0) * cs),
                    (a + 1.0) + (a - 1.0) * cs - k,
                )
            }
            FilterType::HighShelf => {
                let k = 2.0 * a.sqrt() * alpha;
                Self::norm(
                    a * ((a + 1.0) + (a - 1.0) * cs + k),
                    -2.0 * a * ((a - 1.0) + (a + 1.0) * cs),
                    a * ((a + 1.0) + (a - 1.0) * cs - k),
                    (a + 1.0) - (a - 1.0) * cs + k,
                    2.0 * ((a - 1.0) - (a + 1.0) * cs),
                    (a + 1.0) - (a - 1.0) * cs - k,
                )
            }
        }
    }

    /// |H(e^{jω})| at `freq` Hz.
    pub fn magnitude(&self, freq: f64, sample_rate: f64) -> f64 {
        let w = 2.0 * PI * freq / sample_rate;
        let (s1, c1) = w.sin_cos();
        let (s2, c2) = (2.0 * w).sin_cos();
        // H = (b0 + b1 z^-1 + b2 z^-2) / (1 + a1 z^-1 + a2 z^-2), z^-1 = e^{-jω}.
        let nr = self.b0 + self.b1 * c1 + self.b2 * c2;
        let ni = -(self.b1 * s1 + self.b2 * s2);
        let dr = 1.0 + self.a1 * c1 + self.a2 * c2;
        let di = -(self.a1 * s1 + self.a2 * s2);
        ((nr * nr + ni * ni) / (dr * dr + di * di)).sqrt()
    }

    /// Magnitude in dB.
    pub fn magnitude_db(&self, freq: f64, sample_rate: f64) -> f64 {
        20.0 * self.magnitude(freq, sample_rate).log10()
    }
}

/// The gain at which Premiere's Parametric Equalizer measures a band's width and places a
/// shelf's corner, for a boost of `g` (linear, ≥ 1): 3 dB inside the full gain, and half the
/// gain in dB up to 6 dB (where the two meet).
fn edge_gain(g: f64) -> f64 {
    const INSIDE: f64 = 0.707_945_784_384_137_9; // −3 dB
    (g * INSIDE).max(g.sqrt())
}

/// The cookbook Q ([`FilterType::Peaking`]) of a Parametric Equalizer band set to `q` at `freq`
/// in Premiere.
///
/// Premiere's band is `freq / q` wide where its curve is [`edge_gain`]: 3 dB inside the peak
/// (or the dip), not at half the gain as the cookbook's, so above 6 dB it is wider than the
/// cookbook band of the same Q, and the more so the larger the gain. And that width goes into
/// the filter as tan(width / 2) in radians per sample, where the cookbook's shrinks towards
/// the top of the spectrum: at 12.8 kHz Premiere's band is 1.8 times as wide.
///
/// Measured on Premiere Pro 27.0 renders of 30 tones (the audio effects calibration kits):
/// band 3 at 800 Hz from ±1.5 to ±6 dB is the cookbook's curve; +9 dB at Q 0.5, 1, 2, 4 and 8,
/// +12 dB and ±18 dB at Q 2 are this rule's (+12 dB Q 2 is the cookbook's Q 1.07: 6.06 dB at
/// 1250 Hz where Q 2 gives 3.04); band 5 at 12.8 kHz, +12 dB Q 2, is 6.91 dB at 8 kHz where
/// the cookbook's width gives 3.96. The filter this gives is within 0.02 dB of every tone.
pub fn premiere_band_q(q: f64, gain_db: f64, freq: f64, sample_rate: f64) -> f64 {
    if q <= 0.0 || !q.is_finite() || sample_rate <= 0.0 || !sample_rate.is_finite() || !freq.is_finite() {
        return q;
    }
    let w0 = 2.0 * PI * freq.clamp(1.0, sample_rate * 0.49) / sample_rate;
    let g = 10f64.powf(if gain_db.is_finite() { gain_db.abs().min(120.0) } else { 0.0 } / 20.0);
    let edge2 = edge_gain(g).powi(2);
    // √G × the band's width at the edge gain, for (s² + G·b·s + 1) / (s² + b·s + 1): 1 up to
    // 6 dB, where the edge is half the gain
    let wide = if g > 1.0 && edge2 > g * (1.0 + 1e-12) { g.sqrt() * ((edge2 - 1.0) / (g * g - edge2)).sqrt() } else { 1.0 };
    // (a band wider than the spectrum stays short of tan's pole)
    let alpha = wide * (w0 / (2.0 * q)).min(1.5).tan();
    if alpha <= 0.0 || !alpha.is_finite() {
        return q;
    }
    w0.sin() / (2.0 * alpha)
}

/// One biquad section's state (TDF-II), f64 internally.
#[derive(Clone, Copy, Debug, Default)]
pub struct Biquad {
    s1: f64,
    s2: f64,
}

impl Biquad {
    pub const fn new() -> Self {
        Biquad { s1: 0.0, s2: 0.0 }
    }
    #[inline(always)]
    pub fn tick(&mut self, c: &Coeffs, x: f64) -> f64 {
        let y = c.b0 * x + self.s1;
        self.s1 = flush(c.b1 * x - c.a1 * y + self.s2);
        self.s2 = flush(c.b2 * x - c.a2 * y);
        y
    }
    pub fn reset(&mut self) {
        *self = Biquad::new();
    }
    /// Exact zero state, including the sign of zero, for identity-filter fast paths.
    pub(crate) fn is_clear(&self) -> bool {
        self.s1.to_bits() == 0 && self.s2.to_bits() == 0
    }
    /// Filter a buffer in place.
    pub fn process(&mut self, c: &Coeffs, buf: &mut [f32]) {
        for v in buf {
            *v = self.tick(c, *v as f64) as f32;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::*;

    const SR: f64 = 48000.0;

    fn measured_db(c: &Coeffs, freq: f64) -> f64 {
        let n = SR as usize;
        let mut x = sine(freq, 0.5, SR, n, 0.0);
        let mut bq = Biquad::new();
        bq.process(c, &mut x);
        // Skip the transient; measure over an integer number of periods-ish long tail.
        let tail = &x[n / 2..];
        db(tone_amplitude(tail, freq, SR) / 0.5)
    }

    #[test]
    fn peaking_hits_gain_at_centre() {
        let c = Coeffs::design(FilterType::Peaking, SR, 1000.0, 1.0, 6.0);
        assert!((c.magnitude_db(1000.0, SR) - 6.0).abs() < 1e-9);
        assert!(c.magnitude_db(20.0, SR).abs() < 0.05);
        assert!((measured_db(&c, 1000.0) - 6.0).abs() < 0.05);
    }

    #[test]
    fn lowpass_highpass_minus_3db_at_cutoff() {
        let q = std::f64::consts::FRAC_1_SQRT_2;
        let lp = Coeffs::design(FilterType::LowPass, SR, 2000.0, q, 0.0);
        let hp = Coeffs::design(FilterType::HighPass, SR, 2000.0, q, 0.0);
        assert!((lp.magnitude_db(2000.0, SR) + 3.0103).abs() < 0.01);
        assert!((hp.magnitude_db(2000.0, SR) + 3.0103).abs() < 0.01);
        assert!(lp.magnitude_db(100.0, SR).abs() < 0.01);
        assert!(hp.magnitude_db(15000.0, SR).abs() < 0.1);
        // 12 dB/oct slope well past cutoff.
        assert!(lp.magnitude_db(8000.0, SR) < -22.0);
        assert!((measured_db(&lp, 2000.0) + 3.0103).abs() < 0.05);
        assert!((measured_db(&hp, 500.0) - hp.magnitude_db(500.0, SR)).abs() < 0.05);
    }

    #[test]
    fn notch_is_deep_at_centre() {
        let c = Coeffs::design(FilterType::Notch, SR, 60.0, 10.0, 0.0);
        assert!(c.magnitude(60.0, SR) < 1e-6);
        assert!(c.magnitude_db(1000.0, SR).abs() < 0.01);
        assert!(measured_db(&c, 60.0) < -40.0);
    }

    #[test]
    fn premiere_shelves_match_the_measured_curves() {
        // Premiere, Low Shelf 99 Hz +12.1 dB, gentle: ≈ +12 at 20 Hz, +9.3 at 100, +2 at 400,
        // ≈ 0 at 1 kHz (read off its editor's graph, ±1 dB)
        let ls = Coeffs::design(FilterType::GentleLowShelf, SR, 99.0, 1.0, 12.1);
        for (f, want) in [(20.0, 12.0), (100.0, 9.3), (400.0, 2.0), (1000.0, 0.3)] {
            assert!((ls.magnitude_db(f, SR) - want).abs() < 1.0, "{f} Hz: {}", ls.magnitude_db(f, SR));
        }
        // steeper: flat to ~50 Hz, ≈ +2 at 200, 0 by 300
        let st = Coeffs::design(FilterType::SteepLowShelf, SR, 99.0, 1.0, 12.1);
        for (f, want) in [(40.0, 11.8), (99.0, 9.3), (200.0, 2.5), (400.0, 0.3)] {
            assert!((st.magnitude_db(f, SR) - want).abs() < 1.0, "{f} Hz: {}", st.magnitude_db(f, SR));
        }
        // High Shelf 5010 Hz −12.1 dB: ≈ −9.3 at 5 kHz, the full cut by 20 kHz; a cut mirrors a boost
        let hs = Coeffs::design(FilterType::GentleHighShelf, SR, 5010.0, 1.0, -12.1);
        assert!((hs.magnitude_db(5000.0, SR) + 9.3).abs() < 1.0);
        assert!(hs.magnitude_db(20000.0, SR) < -11.0);
        let hb = Coeffs::design(FilterType::GentleHighShelf, SR, 5010.0, 1.0, 12.1);
        assert!((hb.magnitude_db(3000.0, SR) + hs.magnitude_db(3000.0, SR)).abs() < 1e-6);
        assert!(Coeffs::design(FilterType::SteepHighShelf, SR, 5010.0, 1.0, 0.0).magnitude_db(5000.0, SR).abs() < 1e-9);
    }

    /// Premiere Pro 27.0's own renders (the audio effects calibration kit): the shelves pass
    /// 3 dB inside their gain at the set frequency.
    #[test]
    fn premiere_shelves_sit_3_db_inside_their_gain_at_the_frequency() {
        let cases: [(FilterType, f64, f64, &[(f64, f64)]); 4] = [
            (FilterType::GentleLowShelf, 40.0, 9.0, &[(25.0, 7.46), (40.0, 6.00), (63.0, 4.18), (100.0, 2.42), (250.0, 0.54)]),
            (FilterType::SteepLowShelf, 40.0, 9.0, &[(25.0, 8.30), (40.0, 5.99), (63.0, 2.44), (100.0, 0.53)]),
            (FilterType::GentleHighShelf, 18000.0, 9.0, &[(8000.0, 1.09), (12500.0, 2.77), (16000.0, 4.68), (20000.0, 7.37)]),
            (FilterType::SteepHighShelf, 18000.0, 9.0, &[(10000.0, 0.22), (12500.0, 0.77), (16000.0, 3.32), (20000.0, 8.21)]),
        ];
        for (kind, freq, gain, points) in cases {
            let boost = Coeffs::design(kind, SR, freq, 1.0, gain);
            let cut = Coeffs::design(kind, SR, freq, 1.0, -gain);
            for (f, want) in points {
                assert!((boost.magnitude_db(*f, SR) - want).abs() < 0.02, "{kind:?} at {f} Hz: {}", boost.magnitude_db(*f, SR));
                assert!((cut.magnitude_db(*f, SR) + want).abs() < 0.02, "{kind:?} cut at {f} Hz");
            }
        }
        // no gain, a hostile gain: nothing, never a NaN
        for g in [0.0, 1e-12, f64::NAN, f64::INFINITY, 1e9] {
            let c = Coeffs::design(FilterType::SteepLowShelf, SR, 100.0, 1.0, g);
            assert!(c.b0.is_finite() && c.a2.is_finite(), "{g}");
        }
    }

    /// Premiere Pro 27.0's own renders: a band's width is taken 3 dB inside its peak.
    #[test]
    fn premiere_bands_are_as_wide_3_db_inside_the_peak() {
        // up to 6 dB the cookbook's own band
        for g in [1.5, 3.0, 6.0, -4.5] {
            assert!((premiere_band_q(2.0, g, 200.0, SR) - 2.0).abs() < 0.001, "{g}");
        }
        // (band, gain, Q set, tone, Premiere's level there)
        let cases = [
            (800.0, 12.0, 2.0, 1250.0, 6.06),
            (800.0, 18.0, 2.0, 1250.0, 11.70),
            (800.0, -18.0, 2.0, 1250.0, -11.70),
            (800.0, 9.0, 0.5, 315.0, 5.75),
            (800.0, 9.0, 8.0, 1000.0, 1.40),
            (800.0, 6.0, 2.0, 630.0, 3.11),
            (50.0, 12.0, 1.0, 125.0, 5.40),
            // towards the top of the spectrum the band keeps its width
            (3200.0, 18.0, 4.0, 5000.0, 6.98),
            (12800.0, 12.0, 2.0, 8000.0, 6.91),
            (12800.0, 6.0, 2.0, 10000.0, 3.48),
            (12800.0, -12.0, 8.0, 10000.0, -2.91),
        ];
        for (band, gain, q, f, want) in cases {
            let c = Coeffs::design(FilterType::Peaking, SR, band, premiere_band_q(q, gain, band, SR), gain);
            assert!((c.magnitude_db(f, SR) - want).abs() < 0.02, "{band} Hz {gain} dB Q {q} at {f} Hz: {}", c.magnitude_db(f, SR));
            assert!((c.magnitude_db(band, SR) - gain).abs() < 1e-6);
        }
        // hostile settings give a usable Q, never a NaN
        for (q, g, f, sr) in [
            (2.0, f64::NAN, 800.0, SR),
            (2.0, f64::INFINITY, 800.0, SR),
            (0.0, 12.0, 800.0, SR),
            (2.0, 1e9, 800.0, SR),
            (1e-9, 12.0, 20000.0, SR),
            (2.0, 12.0, f64::NAN, SR),
            (2.0, 12.0, 800.0, 0.0),
            (f64::NAN, 3.0, 800.0, SR),
        ] {
            let got = premiere_band_q(q, g, f, sr);
            assert!(got.is_finite() || q.is_nan(), "{q} {g} {f} {sr}: {got}");
        }
    }

    #[test]
    fn first_order_passes_fall_6_db_per_octave() {
        // Premiere's HP at 197 Hz, 6 dB/Oct: −3 dB at the point, ≈ −20 dB at 20 Hz
        let hp = Coeffs::design(FilterType::HighPass1, SR, 197.0, 1.0, 0.0);
        assert!((hp.magnitude_db(197.0, SR) + 3.0103).abs() < 0.01);
        assert!((hp.magnitude_db(20.0, SR) + 19.9).abs() < 0.5);
        let lp = Coeffs::design(FilterType::LowPass1, SR, 1000.0, 1.0, 0.0);
        assert!((lp.magnitude_db(1000.0, SR) + 3.0103).abs() < 0.01);
        assert!(lp.magnitude_db(8000.0, SR) < -18.0 && lp.magnitude_db(8000.0, SR) > -19.5);
        assert!((measured_db(&lp, 1000.0) + 3.0103).abs() < 0.05);
    }

    #[test]
    fn shelves_reach_their_gain() {
        let q = std::f64::consts::FRAC_1_SQRT_2;
        let ls = Coeffs::design(FilterType::LowShelf, SR, 200.0, q, 9.0);
        let hs = Coeffs::design(FilterType::HighShelf, SR, 5000.0, q, -9.0);
        assert!((ls.magnitude_db(10.0, SR) - 9.0).abs() < 0.05);
        assert!((ls.magnitude_db(200.0, SR) - 4.5).abs() < 0.01);
        assert!(ls.magnitude_db(10000.0, SR).abs() < 0.05);
        assert!((hs.magnitude_db(23000.0, SR) + 9.0).abs() < 0.1);
        assert!((hs.magnitude_db(5000.0, SR) + 4.5).abs() < 0.01);
        assert!((measured_db(&ls, 40.0) - ls.magnitude_db(40.0, SR)).abs() < 0.05);
        assert!((measured_db(&hs, 12000.0) - hs.magnitude_db(12000.0, SR)).abs() < 0.05);
    }

    #[test]
    fn bandpass_and_allpass() {
        let bp = Coeffs::design(FilterType::BandPass, SR, 1000.0, 2.0, 0.0);
        assert!(bp.magnitude_db(1000.0, SR).abs() < 1e-9);
        let ap = Coeffs::design(FilterType::AllPass, SR, 1000.0, 2.0, 0.0);
        for f in [50.0, 1000.0, 9000.0] {
            assert!(ap.magnitude_db(f, SR).abs() < 1e-9);
        }
    }

    #[test]
    fn state_flushes_to_zero_no_denormals() {
        let c = Coeffs::design(FilterType::LowPass, SR, 100.0, 0.7, 0.0);
        let mut bq = Biquad::new();
        bq.tick(&c, 1.0);
        let mut last = 1.0;
        for _ in 0..2_000_000 {
            last = bq.tick(&c, 0.0);
            assert!(!(last as f32).is_subnormal());
        }
        assert_eq!(last, 0.0);
    }

    // ----------------------------------------------------------------- non-finite input

    fn flush_reference(x: f64) -> f64 {
        if x.abs() < 1e-30 { 0.0 } else { x }
    }

    fn flush32_reference(x: f32) -> f32 {
        if x.abs() < 1e-25 { 0.0 } else { x }
    }

    #[test]
    fn flush_is_unchanged_for_finite_values() {
        let mut rng = Rng::new(21);
        let mut xs = vec![0.0, -0.0, 1e-30, -1e-30, 1e-300, f64::MIN_POSITIVE, 5e-324, f64::MAX, f64::MIN, 1.0, -1.0];
        xs.extend([1e-30f64.next_down(), 1e-30f64.next_up(), (-1e-30f64).next_down(), (-1e-30f64).next_up()]);
        for _ in 0..100_000 {
            let m = rng.uniform() as f64;
            let e = (rng.uniform() * 300.0) as i32;
            xs.push(m * 10f64.powi(e));
        }
        for &x in &xs {
            assert_eq!(crate::flush(x).to_bits(), flush_reference(x).to_bits(), "{x:e}");
            let y = x as f32;
            if y.is_finite() {
                assert_eq!(crate::flush32(y).to_bits(), flush32_reference(y).to_bits(), "{y:e}");
            }
        }
        for y in [1e-25f32, 1e-25f32.next_down(), -1e-25f32, f32::MAX, f32::MIN, 1e-45] {
            assert_eq!(crate::flush32(y).to_bits(), flush32_reference(y).to_bits(), "{y:e}");
        }
        for x in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert_eq!(crate::flush(x), 0.0);
            assert_eq!(crate::flush32(x as f32), 0.0);
        }
    }

    /// Run `tick` over noise with one non-finite sample at index 100 and return the output.
    fn run_with_bad_sample(bad: f32, mut tick: impl FnMut(f32) -> f32) -> Vec<f32> {
        let mut rng = Rng::new(4);
        (0..4800)
            .map(|i| {
                let x = if i == 100 { bad } else { 0.5 * rng.uniform() };
                tick(x)
            })
            .collect()
    }

    #[test]
    fn one_non_finite_sample_does_not_poison_filter_state() {
        use crate::effects::util::{AllPass1, DcBlock, OnePole};
        for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let c = Coeffs::design(FilterType::Peaking, SR, 1000.0, 1.0, 6.0);
            let mut bq = Biquad::new();
            let mut lp = OnePole::default();
            let mut hp = OnePole::default();
            let a = OnePole::alpha(500.0, SR as f32);
            let mut ap = AllPass1::default();
            let apc = AllPass1::coef(1000.0, SR as f32);
            let mut dc = DcBlock::default();
            let r = DcBlock::r(SR as f32);
            let outputs = [
                ("Biquad", run_with_bad_sample(bad, |x| bq.tick(&c, x as f64) as f32)),
                ("OnePole lp", run_with_bad_sample(bad, |x| lp.lp(a, x))),
                ("OnePole hp", run_with_bad_sample(bad, |x| hp.hp(a, x))),
                ("AllPass1", run_with_bad_sample(bad, |x| ap.tick(apc, x as f64) as f32)),
                ("DcBlock", run_with_bad_sample(bad, |x| dc.tick(r, x))),
            ];
            for (name, y) in outputs {
                assert!(y[..100].iter().all(|v| v.is_finite()), "{name} {bad}");
                assert!(y[102..].iter().all(|v| v.is_finite()), "{name} stays non-finite after {bad}");
            }
        }
    }

    #[test]
    fn effects_recover_after_one_nan_sample() {
        for id in ["parametric_eq", "simple_eq", "graphic_eq_10", "notch_filter"] {
            let mut fx = crate::create_effect(id, SR as f32, 2).unwrap_or_else(|| panic!("{id}"));
            for p in fx.params() {
                if p.unit == crate::Unit::Decibels && p.id != "output" {
                    fx.set_param(p.id, 6.0);
                }
            }
            let mut rng = Rng::new(8);
            let mut l: Vec<f32> = (0..4800).map(|_| 0.5 * rng.uniform()).collect();
            let mut r: Vec<f32> = (0..4800).map(|_| 0.5 * rng.uniform()).collect();
            l[100] = f32::NAN;
            fx.process(&mut [&mut l, &mut r]);
            let mut l: Vec<f32> = (0..4800).map(|_| 0.5 * rng.uniform()).collect();
            let mut r: Vec<f32> = (0..4800).map(|_| 0.5 * rng.uniform()).collect();
            fx.process(&mut [&mut l, &mut r]);
            assert!(l.iter().chain(&r).all(|v| v.is_finite()), "{id} stays NaN after one NaN input sample");
        }
    }

    const RUN_LEN: usize = 2 * 48000;

    /// Parameter settings for the non-finite input sweeps.
    #[derive(Clone, Copy, Debug)]
    enum Setting {
        Default,
        /// Toggles on, last choice, other parameters at 37 % of their range.
        Moved,
        /// Toggles off, every other parameter at its minimum (e.g. no gate hold, multiband
        /// bands neither bypassed nor linked).
        Min,
    }

    /// Process 2 s of noise in 512-sample blocks, with `bad` (if any) at input sample 1000 of
    /// the left channel. Returns both output channels and the position of the bad sample's own
    /// latency-delayed output.
    fn run_effect(id: &str, setting: Setting, bad: Option<f32>) -> (Vec<f32>, Vec<f32>, usize) {
        let mut fx = crate::create_effect(id, SR as f32, 2).unwrap_or_else(|| panic!("{id}"));
        for p in fx.params() {
            let v = match (setting, p.unit) {
                (Setting::Default, _) => continue,
                (Setting::Moved, crate::Unit::Toggle) => 1.0,
                (Setting::Moved, crate::Unit::Choice) => p.max,
                (Setting::Moved, _) => p.min + (p.max - p.min) * 0.37,
                (Setting::Min, _) => p.min,
            };
            fx.set_param(p.id, v);
        }
        run_fx(fx, bad)
    }

    /// [`run_effect`] for an already configured effect.
    fn run_fx(mut fx: Box<dyn crate::AudioEffect>, bad: Option<f32>) -> (Vec<f32>, Vec<f32>, usize) {
        const BAD_AT: usize = 1000;
        let mut rng = Rng::new(8);
        let mut l: Vec<f32> = (0..RUN_LEN).map(|_| 0.5 * rng.uniform()).collect();
        let mut r: Vec<f32> = (0..RUN_LEN).map(|_| 0.5 * rng.uniform()).collect();
        if let Some(bad) = bad {
            l[BAD_AT] = bad;
        }
        for (a, b) in l.chunks_mut(512).zip(r.chunks_mut(512)) {
            fx.process(&mut [a, b]);
        }
        (l, r, BAD_AT + fx.latency())
    }

    /// Last output sample index (either channel) that is not finite, ignoring `own`.
    fn last_non_finite_in(l: &[f32], r: &[f32], own: usize) -> Option<usize> {
        l.iter().chain(r).enumerate().map(|(i, v)| (i % l.len(), v)).filter(|&(i, v)| i != own && !v.is_finite()).map(|(i, _)| i).max()
    }

    /// RMS of the last 0.5 s of both channels.
    fn tail_rms(l: &[f32], r: &[f32]) -> f64 {
        let t = l.len() - 24000;
        let s: f64 = l[t..].iter().chain(&r[t..]).map(|&v| (v as f64) * (v as f64)).sum();
        (s / 48000.0).sqrt()
    }

    /// Every registered effect, at default, moved and minimum parameters, produces finite output
    /// again within 0.5 s after one NaN, +inf or -inf input sample, and is not left muted: the
    /// level of its last 0.5 s is at least half that of a run without the bad sample.
    #[test]
    fn every_effect_recovers_after_one_non_finite_sample() {
        let mut failures = Vec::new();
        for e in crate::effects() {
            for setting in [Setting::Default, Setting::Moved, Setting::Min] {
                let (l, r, _) = run_effect(e.id, setting, None);
                let clean = tail_rms(&l, &r);
                for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
                    let (l, r, own) = run_effect(e.id, setting, Some(bad));
                    let last = last_non_finite_in(&l, &r, own);
                    if last.is_some_and(|i| i >= own + 24000) {
                        failures.push(format!("{} ({setting:?}) still non-finite at sample {last:?} after {bad}", e.id));
                    }
                    let level = tail_rms(&l, &r);
                    if level < 0.5 * clean {
                        failures.push(format!("{} ({setting:?}) level {level:e} after {bad}, {clean:e} without", e.id));
                    }
                }
            }
        }
        assert!(failures.is_empty(), "{failures:#?}");
    }

    /// Effects whose state is cleaned outside `flush` recover from NaN and ±inf within two blocks.
    #[test]
    fn phaser_click_remover_binauralizer_recover_from_non_finite_input() {
        for id in ["phaser", "click_remover", "binauralizer"] {
            for setting in [Setting::Default, Setting::Moved] {
                for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
                    let (l, r, own) = run_effect(id, setting, Some(bad));
                    let last = last_non_finite_in(&l, &r, own);
                    assert!(last.is_none_or(|i| i < own + 1024), "{id} ({setting:?}) still non-finite at sample {last:?} after {bad}");
                }
            }
        }
    }

    /// With the loudness maximiser on and the exciter off (which the sweep's settings do not
    /// combine), an inf sample used to hold the limiter envelope at -inf dB: silence for good.
    #[test]
    fn mastering_limiter_recovers_from_inf() {
        let make = || {
            let mut fx = crate::create_effect("mastering", SR as f32, 2).unwrap();
            fx.set_param("loudness", 50.0);
            fx
        };
        let (l, r, _) = run_fx(make(), None);
        let clean = tail_rms(&l, &r);
        for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let (l, r, own) = run_fx(make(), Some(bad));
            let last = last_non_finite_in(&l, &r, own);
            assert!(last.is_none_or(|i| i < own + 1024), "still non-finite at sample {last:?} after {bad}");
            assert!(tail_rms(&l, &r) >= 0.5 * clean, "level {:e} after {bad}, {clean:e} without", tail_rms(&l, &r));
        }
    }

    /// A NaN written to any parameter of any effect never reaches the output (`set_param` ignores NaN).
    #[test]
    fn nan_parameter_values_never_produce_non_finite_output() {
        let mut bad = Vec::new();
        for e in crate::effects() {
            let ids: Vec<&str> = crate::create_effect(e.id, SR as f32, 2).unwrap().params().iter().map(|p| p.id).collect();
            for id in ids {
                let mut fx = crate::create_effect(e.id, SR as f32, 2).unwrap();
                fx.set_param(id, f32::NAN);
                let (l, r, _) = run_fx(fx, None);
                if l.iter().chain(&r).any(|v| !v.is_finite()) {
                    bad.push(format!("{}.{id}", e.id));
                }
            }
        }
        assert!(bad.is_empty(), "{bad:?}");
    }

    /// Subnormal input samples (a decoder or a fade-out can deliver them) never make an effect
    /// produce non-finite output, and once the input is silent again no effect keeps emitting
    /// subnormals in its last 0.5 s.
    #[test]
    fn subnormal_input_stays_finite_and_decays() {
        let mut bad = Vec::new();
        for e in crate::effects() {
            for setting in [Setting::Default, Setting::Min] {
                let mut fx = crate::create_effect(e.id, SR as f32, 2).unwrap();
                for p in fx.params() {
                    if let Setting::Min = setting {
                        fx.set_param(p.id, p.min);
                    }
                }
                let mut rng = Rng::new(5);
                let (mut l, mut r) = (vec![0.0f32; RUN_LEN], vec![0.0f32; RUN_LEN]);
                for (a, b) in l.iter_mut().zip(r.iter_mut()).take(RUN_LEN / 2) {
                    *a = 1e-40 * rng.uniform();
                    *b = -1e-41 * rng.uniform();
                }
                for (a, b) in l.chunks_mut(512).zip(r.chunks_mut(512)) {
                    fx.process(&mut [a, b]);
                }
                let t = RUN_LEN - 24000;
                if l.iter().chain(&r).any(|v| !v.is_finite()) || l[t..].iter().chain(&r[t..]).any(|v| v.is_subnormal()) {
                    bad.push(format!("{} ({setting:?})", e.id));
                }
            }
        }
        assert!(bad.is_empty(), "{bad:?}");
    }
}
