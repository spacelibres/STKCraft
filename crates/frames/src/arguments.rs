//! The Delaunay fundamental arguments the IAU-80 nutation series is written in.
//!
//! Source: Seidelmann (ed.), *Explanatory Supplement to the Astronomical Almanac* (1992),
//! section 3.222, p111, which defines the IAU-80 series. The polynomial form below is the one
//! ERFA uses in `eraNut80` (`src/nut80.c`) and Vallado uses in `AstroLib.cpp :: fundarg` for
//! the 1980 theory; both were checked against each other and their coefficients are identical.
//!
//! Each argument is a polynomial in Julian centuries of TT plus a whole-revolution term taken
//! separately. That split is not cosmetic: it keeps the large integer part of `1325 * t` exact,
//! so the fractional precision of the polynomial is not destroyed by adding it to a number of
//! order 1e4 arcseconds.

use stkcraft_core::{Error, Result};

/// Arcseconds to radians, `pi / (180 * 3600)`.
const ARCSEC_TO_RAD: f64 = core::f64::consts::PI / 648_000.0;

/// One revolution in radians.
const TAU: f64 = core::f64::consts::TAU;

/// The five Delaunay fundamental arguments the IAU-80 nutation series needs, radians.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FundamentalArguments {
    /// Mean longitude of the Moon minus mean longitude of the Moon's perigee (`l`).
    pub moon_mean_longitude: f64,
    /// Mean longitude of the Sun minus mean longitude of the Sun's perigee (`l'`).
    pub sun_mean_longitude: f64,
    /// Mean longitude of the Moon minus mean longitude of the Moon's node (`F`).
    pub moon_argument_of_latitude: f64,
    /// Mean elongation of the Moon from the Sun (`D`).
    pub moon_elongation: f64,
    /// Longitude of the mean ascending node of the lunar orbit (`Omega`).
    pub moon_node_longitude: f64,
}

/// Fold an angle into `(-pi, pi]`.
///
/// This is ERFA's `eraAnpm`. A plain `%` is not enough: it leaves the result with the sign of
/// the dividend, so an angle just below zero stays negative rather than wrapping to just below
/// `+pi`, and the two conventions differ in the last bits the series cares about.
fn normalise(angle: f64) -> f64 {
    let wrapped = angle % TAU;
    if wrapped.abs() >= core::f64::consts::PI {
        wrapped - TAU.copysign(angle)
    } else {
        wrapped
    }
}

/// Fundamental arguments from the IERS 1980 (IAU-80) expressions.
///
/// # Errors
///
/// Returns [`Error::Invalid`] for a non-finite time, so a NaN can never reach the series.
pub fn iau_1980(ttt: f64) -> Result<FundamentalArguments> {
    if !ttt.is_finite() {
        return Err(Error::Invalid {
            what: "Julian centuries of TT",
            reason: "must be finite".to_owned(),
        });
    }

    // `(c0 + (c1 + (c2 + c3 t) t) t)` arcseconds, plus a whole-revolution term in turns.
    let moon_mean_longitude = normalise(
        (485_866.733 + (715_922.633 + (31.310 + 0.064 * ttt) * ttt) * ttt) * ARCSEC_TO_RAD
            + (1_325.0 * ttt).fract() * TAU,
    );
    let sun_mean_longitude = normalise(
        (1_287_099.804 + (1_292_581.224 + (-0.577 - 0.012 * ttt) * ttt) * ttt) * ARCSEC_TO_RAD
            + (99.0 * ttt).fract() * TAU,
    );
    let moon_argument_of_latitude = normalise(
        (335_778.877 + (295_263.137 + (-13.257 + 0.011 * ttt) * ttt) * ttt) * ARCSEC_TO_RAD
            + (1_342.0 * ttt).fract() * TAU,
    );
    let moon_elongation = normalise(
        (1_072_261.307 + (1_105_601.328 + (-6.891 + 0.019 * ttt) * ttt) * ttt) * ARCSEC_TO_RAD
            + (1_236.0 * ttt).fract() * TAU,
    );
    let moon_node_longitude = normalise(
        (450_160.280 + (-482_890.539 + (7.455 + 0.008 * ttt) * ttt) * ttt) * ARCSEC_TO_RAD
            + (-5.0 * ttt).fract() * TAU,
    );

    Ok(FundamentalArguments {
        moon_mean_longitude,
        sun_mean_longitude,
        moon_argument_of_latitude,
        moon_elongation,
        moon_node_longitude,
    })
}

#[cfg(test)]
#[expect(clippy::expect_used)]
mod tests {
    use super::*;

    const TTT: f64 = 0.042_623_631_9;

    #[test]
    fn the_five_arguments_match_two_independent_implementations() {
        // Source: Seidelmann (ed.), Explanatory Supplement to the Astronomical Almanac
        // (1992), section 3.222. The values below were produced by ERFA `eraNut80`
        // (`src/nut80.c`) and cross-checked against Vallado `AstroLib.cpp :: fundarg` with
        // `opt == e80`; the two agree to better than 1e-13 rad at this instant.
        let a = iau_1980(TTT).expect("finite time");
        assert!(
            (a.moon_mean_longitude - -0.786_936_422_200_554_4).abs() < 1e-13,
            "l was {}",
            a.moon_mean_longitude
        );
        assert!(
            (a.sun_mean_longitude - 1.604_620_693_966_198_9).abs() < 1e-13,
            "l' was {}",
            a.sun_mean_longitude
        );
        assert!(
            (a.moon_argument_of_latitude - 2.951_296_481_245_345_5).abs() < 1e-13,
            "F was {}",
            a.moon_argument_of_latitude
        );
        assert!(
            (a.moon_elongation - -2.849_218_300_356_398).abs() < 1e-13,
            "D was {}",
            a.moon_elongation
        );
        assert!(
            (a.moon_node_longitude - 0.743_590_790_115_704_4).abs() < 1e-13,
            "Omega was {}",
            a.moon_node_longitude
        );
    }

    #[test]
    fn arguments_stay_inside_one_turn_for_six_millennia() {
        // The series is written in terms of these arguments and assumes they are small. An
        // unnormalised argument would be thousands of radians and lose the precision the
        // small terms carry, which shows up as a nutation error of tenths of an arcsecond.
        for centuries in [-60.0, -3.0, -0.5, 0.0, 0.5, 3.0, 60.0] {
            let a = iau_1980(centuries).expect("finite time");
            for (name, value) in [
                ("l", a.moon_mean_longitude),
                ("l'", a.sun_mean_longitude),
                ("F", a.moon_argument_of_latitude),
                ("D", a.moon_elongation),
                ("Omega", a.moon_node_longitude),
            ] {
                assert!(
                    value > -core::f64::consts::PI && value <= core::f64::consts::PI,
                    "{name} at {centuries} centuries was {value}"
                );
            }
        }
    }

    #[test]
    fn a_non_finite_time_is_refused() {
        assert!(iau_1980(f64::NAN).is_err());
        assert!(iau_1980(f64::INFINITY).is_err());
        assert!(iau_1980(f64::NEG_INFINITY).is_err());
    }
}
