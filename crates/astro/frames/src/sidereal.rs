//! Greenwich mean sidereal time.
//!
//! Sidereal time is the Earth's rotation angle measured against the stars rather than the Sun,
//! and it is what turns a TEME position into an Earth-fixed one.
//!
//! Source: Vallado, *Fundamentals of Astrodynamics and Applications* (2013), eq. 3-45, the
//! IAU-82 expression, as implemented by the reference `gstime` routine:
//!
//! ```text
//! tut1  = (jdut1 - 2451545.0) / 36525
//! gmst  = -6.2e-6 tut1^3 + 0.093104 tut1^2 + (876600*3600 + 8640184.812866) tut1 + 67310.54841
//! ```
//!
//! in seconds, reduced modulo one day and converted at 240 seconds per degree.

use stkcraft_framework_core::{Error, Result};

/// Seconds in a day, for the reduction to one turn.
const SECONDS_PER_DAY: f64 = 86_400.0;

/// Seconds of sidereal time per degree. There are 86 400 seconds of mean solar time in a day
/// and 360 degrees, so 240 seconds per degree.
const SECONDS_PER_DEGREE: f64 = 240.0;

/// Degrees to radians.
const DEG_TO_RAD: f64 = core::f64::consts::PI / 180.0;

/// Greenwich mean sidereal time at a UT1 Julian Date, radians in `[0, 2pi)`.
///
/// The whole-turns term (`876600 * 3600`) is deliberately kept: it is the number of turns the
/// Earth has made since J2000, and subtracting it before the reduction would lose precision
/// for no gain. The reference implementation keeps it and reduces afterwards, which is also
/// the form that reproduces its published output.
///
/// # Errors
///
/// Returns [`Error::Invalid`] for a non-finite date.
pub fn greenwich_mean(jdut1: f64) -> Result<f64> {
    if !jdut1.is_finite() {
        return Err(Error::Invalid {
            what: "UT1 Julian Date",
            reason: "must be finite".to_owned(),
        });
    }

    let centuries = (jdut1 - 2_451_545.0) / 36_525.0;

    // The whole polynomial is in seconds of sidereal time. Note that the cubic and quadratic
    // coefficients are already in seconds and carry no per-day factor: `-6.2e-6` is seconds
    // of GMST, not seconds per day. Scaling them (which is a natural-looking mistake) moves
    // the answer by about 5 arcminutes.
    let seconds = -6.2e-6 * centuries * centuries * centuries
        + 0.093_104 * centuries * centuries
        + (876_600.0 * 3600.0 + 8_640_184.812_866) * centuries
        + 67_310.548_41;

    // Reduce to one day's worth of angle. `rem_euclid` folds a negative date into `[0, 360)`
    // without the branch a plain `%` would need.
    let degrees = (seconds % SECONDS_PER_DAY) / SECONDS_PER_DEGREE;
    Ok(degrees.rem_euclid(360.0) * DEG_TO_RAD)
}

#[cfg(test)]
#[expect(clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn gmst_matches_the_published_reference() {
        // Source: Vallado, Fundamentals (2013), eq. 3-45, as implemented in
        // `AstroLib.cpp :: gstime`. This is the value that routine prints for the
        // Example 3-15 instant.
        let theta = greenwich_mean(2_453_101.827_406_783).expect("finite UT1");
        assert!(
            (theta - 5.459_562_587_690).abs() < 1e-12,
            "GMST was {theta}"
        );
    }

    #[test]
    fn gmst_is_always_inside_one_turn() {
        for jd in [
            2_400_000.5,
            2_451_545.0,
            2_453_101.827_406_783,
            2_460_000.0,
            2_500_000.0,
        ] {
            let theta = greenwich_mean(jd).expect("finite UT1");
            assert!(
                (0.0..core::f64::consts::TAU).contains(&theta),
                "GMST at {jd} was {theta}"
            );
        }
    }

    #[test]
    fn gmst_advances_one_extra_turn_per_year() {
        // The Earth turns 366.2422 times per tropical year and the Sun crosses the meridian
        // 365.2422 times, so sidereal time gains exactly one turn per year. Note that GMST
        // does *not* return to its starting angle after a year: the angle after 365.2422 days
        // is 0.2422 of a turn from where it was. That residual is the whole reason a sidereal
        // day is shorter than a solar day, and asserting it keeps the rate honest without
        // pretending a year is a whole number of turns.
        let start = 2_451_545.0;
        let tropical_year = 365.242_189;
        let a = greenwich_mean(start).expect("finite UT1");
        let b = greenwich_mean(start + tropical_year).expect("finite UT1");

        // Unwrap by counting turns: the expected advance is one, times the ratio.
        let raw_turns = (b - a) / core::f64::consts::TAU;
        let whole = raw_turns.floor();
        let expected_fraction = (tropical_year + 1.0) - (tropical_year + 1.0).floor();
        let fraction = raw_turns - whole;
        assert!(
            (fraction - expected_fraction).abs() < 1e-6
                || (fraction - (expected_fraction - 1.0)).abs() < 1e-6,
            "a tropical year advanced {raw_turns} turns ({fraction} of a turn)"
        );
    }

    #[test]
    fn gmst_advances_one_turn_per_sidereal_day() {
        // The sidereal-to-solar day ratio, 1.00273790935, is the defining property of sidereal
        // time: a day of one sidereal day's length must advance GMST by exactly one turn.
        // Since that is a whole turn, the angle *wraps back to where it started* — so the
        // invariant is that the two values are equal, not that they differ by one turn.
        // Reading it the other way round is the mistake this test now guards against.
        let sidereal_day = 86_164.090_5 / 86_400.0;
        let a = greenwich_mean(2_451_545.0).expect("finite UT1");
        let b = greenwich_mean(2_451_545.0 + sidereal_day).expect("finite UT1");
        let mut difference = (b - a).abs();
        if difference > core::f64::consts::PI {
            difference = core::f64::consts::TAU - difference;
        }
        assert!(
            difference < 1e-6,
            "one sidereal day moved GMST by {difference} rad short of a full turn"
        );
    }

    #[test]
    fn a_non_finite_date_is_refused() {
        assert!(greenwich_mean(f64::NAN).is_err());
        assert!(greenwich_mean(f64::INFINITY).is_err());
        assert!(greenwich_mean(f64::NEG_INFINITY).is_err());
    }
}
