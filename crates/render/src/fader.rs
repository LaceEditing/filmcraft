//! Premiere's fader law for a clip's Volume level.
//!
//! Between two Level keyframes Premiere Pro moves the fader in a straight line (with the
//! keyframes' easing): not the level in dB, and not the amplitude. Measured from Premiere Pro
//! 2027's renders of keyframed tones (eleven ramps between +15 dB and silence, which this law
//! follows within 0.001 dB away from silence): the fader's travel runs 0…1, 0 dB sits at 0.65 and
//! +15 dB at the top; below 0 dB the gain is a power of the travel, and above it the curve is the
//! mirror image, so +15 dB at 1 answers −15 dB at 0.3. Half way down a 0 → −20 dB ramp the level
//! is −7.53 dB, half way down a fade to silence −13.45 dB, and half way from +6 to −6 dB it is
//! 0 dB.

/// The fader's travel at 0 dB.
const UNITY: f64 = 0.65;
/// The travel at −15 dB: the mirror image of +15 dB at the top of the travel (1 = 2 × 0.65 − 0.3).
const MINUS_15: f64 = 0.3;
/// The Level's lowest value, Premiere's −∞: silence.
const SILENT_DB: f64 = -287.5;

/// dB per decade of travel below 0 dB (44.67): −15 dB between 0.65 and 0.3.
fn db_per_decade() -> f64 {
    15.0 / (UNITY / MINUS_15).log10()
}

/// The fader's travel (0 = silence, 0.65 = 0 dB, 1 = +15 dB) for a level in dB.
pub fn position(db: f64) -> f64 {
    if db.is_nan() || db <= SILENT_DB {
        return 0.0;
    }
    let below = UNITY * 10f64.powf(-db.abs() / db_per_decade());
    let x = if db <= 0.0 { below } else { 2.0 * UNITY - below };
    x.clamp(0.0, 1.0)
}

/// The gain (1 = 0 dB) at a fader travel, the inverse of [`position`]. A travel outside 0…1 is
/// taken as its nearest end.
pub fn gain(position: f64) -> f64 {
    if position.is_nan() || position <= 0.0 {
        return 0.0;
    }
    let x = position.min(1.0);
    let power = db_per_decade() / 20.0;
    if x <= UNITY { (x / UNITY).powf(power) } else { ((2.0 * UNITY - x) / UNITY).powf(-power) }
}

/// The level in dB at a fader travel (silence is the Level's lowest value).
pub fn db(position: f64) -> f64 {
    let g = gain(position);
    if g <= 0.0 { SILENT_DB } else { (20.0 * g.log10()).max(SILENT_DB) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gain_db(x: f64) -> f64 {
        20.0 * gain(x).log10()
    }

    #[test]
    fn the_travel_has_premieres_marks() {
        assert!((position(0.0) - 0.65).abs() < 1e-12);
        assert!((position(15.0) - 1.0).abs() < 1e-12);
        assert!((position(-15.0) - 0.3).abs() < 1e-12);
        assert_eq!(position(-287.5), 0.0);
        assert_eq!(gain(0.0), 0.0);
        assert!((gain(0.65) - 1.0).abs() < 1e-12);
        assert!((gain_db(1.0) - 15.0).abs() < 1e-9);
        assert!((gain_db(0.3) + 15.0).abs() < 1e-9);
    }

    #[test]
    fn position_and_gain_are_inverses() {
        for db10 in -900..=150 {
            let d = f64::from(db10) / 10.0;
            assert!((gain_db(position(d)) - d).abs() < 1e-9, "{d} dB");
            assert!((db(position(d)) - d).abs() < 1e-9, "{d} dB");
        }
    }

    /// Levels Premiere Pro 2027 rendered part of the way along a straight ramp between two Level
    /// keyframes (from dB, to dB, how far along, the level there in dB).
    #[test]
    fn ramps_follow_premieres_renders() {
        let measured = [
            (0.0, -20.0, 0.1, -1.290),
            (0.0, -20.0, 0.5, -7.529),
            (0.0, -20.0, 0.9, -16.782),
            (0.0, -287.5, 0.2, -4.328),
            (0.0, -287.5, 0.5, -13.446),
            (0.0, -287.5, 0.8, -31.217),
            (6.0, -6.0, 0.1, 4.643),
            (6.0, -6.0, 0.5, 0.0),
            (6.0, -6.0, 0.8, -3.373),
            (15.0, -287.5, 0.1, 9.420),
            (15.0, -287.5, 0.4, -1.552),
            (15.0, -287.5, 0.6, -9.418),
            (-6.0, -60.0, 0.5, -18.282),
            (0.0, -12.0, 0.5, -5.086),
        ];
        for (from, to, u, want) in measured {
            let x = position(from) + (position(to) - position(from)) * u;
            assert!((gain_db(x) - want).abs() < 0.01, "{from} to {to} dB at {u}: {} vs {want}", gain_db(x));
        }
    }

    #[test]
    fn hostile_values_are_silence_or_the_ends() {
        assert_eq!(position(f64::NAN), 0.0);
        assert_eq!(position(f64::NEG_INFINITY), 0.0);
        assert_eq!(position(f64::INFINITY), 1.0);
        assert_eq!(position(1e9), 1.0);
        assert_eq!(gain(f64::NAN), 0.0);
        assert_eq!(gain(-3.0), 0.0);
        assert!((gain(7.0) - gain(1.0)).abs() < 1e-12);
        assert_eq!(db(0.0), -287.5);
        assert_eq!(db(f64::NAN), -287.5);
    }
}
