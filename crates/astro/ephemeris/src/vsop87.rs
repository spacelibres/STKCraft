//! VSOP87: an analytic planetary theory, read from its published coefficients.
//!
//! VSOP87 (Bretagnon & Francou 1988) gives each planet's heliocentric coordinates as Poisson
//! series — a sum of terms `A·cos(B + C·T)·T^power` with `T` in thousands of Julian years from
//! J2000. It is the analytic theory the IAU and IMCCE publish, and the tables live outside this
//! repository: [`generated_vsop87`] carries a truncated copy with the provenance in its header.
//!
//! # Why a second analytic backend
//!
//! `KeplerianEphemeris` uses JPL's approximate elements and is good to arcminutes. VSOP87 is the
//! full theory, and with the truncation used here it agrees with CSPICE to tens of kilometres —
//! two orders of magnitude better — at the cost of a much larger table to carry. Both exist
//! because they answer different needs: one is small enough to paste anywhere, the other is what a
//! caller reaches for when the answer matters.
//!
//! # Frames and time
//!
//! The published coordinates are in the **J2000 ecliptic and equinox**, heliocentric. This module
//! rotates them into the ICRF with the J2000 obliquity (`84381.406″`, IAU 2006) and reports
//! [`Origin::Sun`]. The theory's own time argument is dynamical time, which the documentation says
//! may be taken as TT; TDB differs by under 2 ms, which is 60 m at Mercury's speed and far below
//! this backend's truncation error.
//!
//! # What it does not carry
//!
//! The Moon. VSOP87 is a planetary theory; the lunar motion needs ELP2000, and a Keplerian or
//! VSOP-style guess for it would be wrong by degrees. The Moon is refused by name.

use stkcraft_astro_body::Body;
use stkcraft_core::{Error, Result};
use stkcraft_time::{Epoch, JD_J2000, TimeScale};

use crate::generated_vsop;
use crate::spk::{Ephemeris, Origin, StateVector};

/// One Poisson series: a coordinate, a power of `T` and its terms.
pub(crate) struct Series {
    /// Which coordinate: 0 = x, 1 = y, 2 = z.
    pub(crate) axis: u8,
    /// The power of `T` multiplying the series.
    pub(crate) power: u8,
    /// `(A, B, C)` in au, radians and radians per thousand years.
    pub(crate) terms: &'static [(f64, f64, f64)],
}

/// One planet's series.
pub(crate) struct Vsop87Planet {
    /// The name the table uses.
    pub(crate) name: &'static str,
    /// Every series for that planet.
    pub(crate) series: &'static [Series],
}

/// Kilometres in an astronomical unit (IAU 2012 resolution B2).
const KM_PER_AU: f64 = 149_597_870.7;

/// Days in a thousand Julian years — the unit of `T` in the published series.
const DAYS_PER_MILLENNIUM: f64 = 365_250.0;

/// Seconds in a day.
const SECONDS_PER_DAY: f64 = 86_400.0;

/// The obliquity of the ecliptic at J2000, in arcseconds (IAU 2006): `84381.406″`.
const OBLIQUITY_ARCSECONDS: f64 = 84_381.406;

/// VSOP87's heliocentric positions and velocities, no kernel needed.
#[derive(Debug, Clone, Copy, Default)]
pub struct Vsop87Ephemeris;

impl Vsop87Ephemeris {
    /// The series for a body, if VSOP87 carries it.
    fn planet(body: Body) -> Option<&'static Vsop87Planet> {
        generated_vsop::PLANETS
            .iter()
            .find(|planet| planet.name.eq_ignore_ascii_case(body.name()))
    }

    /// Heliocentric position and velocity in au and au per day, J2000 ecliptic.
    fn ecliptic_state(planet: &Vsop87Planet, julian_day: f64) -> ([f64; 3], [f64; 3]) {
        let centuries = (julian_day - JD_J2000) / DAYS_PER_MILLENNIUM;
        let mut position = [0.0_f64; 3];
        let mut velocity = [0.0_f64; 3];
        for series in planet.series {
            let axis = usize::from(series.axis);
            let (Some(position_slot), Some(velocity_slot)) =
                (position.get_mut(axis), velocity.get_mut(axis))
            else {
                // An axis outside x, y, z is a corrupt table, not a trajectory.
                continue;
            };
            let power = i32::from(series.power);
            let scale = centuries.powi(power);
            // `d/dT (T^power)`, and the `T^power` factor itself; both are needed because the
            // velocity is the derivative of the position with respect to time.
            let scale_derivative = if power == 0 {
                0.0
            } else {
                f64::from(power) * centuries.powi(power - 1)
            };
            for &(amplitude, phase, frequency) in series.terms {
                let angle = phase + frequency * centuries;
                let (sin, cos) = angle.sin_cos();
                *position_slot += amplitude * cos * scale;
                // d/dT of `A·cos(B + C·T)·T^p` = A·(-C·sin)·T^p + A·cos·p·T^(p-1), with `C` in
                // radians per thousand years — so the result is per thousand years and is
                // converted to per day after the sum.
                let derivative =
                    -amplitude * frequency * sin * scale + amplitude * cos * scale_derivative;
                *velocity_slot += derivative;
            }
        }
        for slot in &mut velocity {
            *slot /= DAYS_PER_MILLENNIUM;
        }
        (position, velocity)
    }
}

impl Ephemeris for Vsop87Ephemeris {
    fn origin(&self) -> Origin {
        Origin::Sun
    }

    fn state(&self, body: Body, t: Epoch) -> Result<StateVector> {
        if t.scale() != TimeScale::Tdb {
            return Err(Error::invalid(
                "ephemeris epoch",
                format!(
                    "VSOP87's time argument is dynamical time, taken here to be the TDB/ TT pair; \
                     convert the epoch, which is on the {} scale",
                    t.scale().id()
                ),
            ));
        }
        let Some(planet) = Self::planet(body) else {
            return Err(Error::unsupported(
                "this body in VSOP87, which is a planetary theory (the Moon needs ELP2000)",
            ));
        };
        let julian = t.julian();
        let julian_day = (julian.day - JD_J2000) + julian.fraction + JD_J2000;
        let (position, velocity) = Self::ecliptic_state(planet, julian_day);

        // The ecliptic-to-ICRF rotation about the x axis by the obliquity, and the unit changes.
        let obliquity = (OBLIQUITY_ARCSECONDS / 3_600.0_f64).to_radians();
        let (sin_obliquity, cos_obliquity) = obliquity.sin_cos();
        let rotate = |v: [f64; 3]| {
            [
                v[0],
                cos_obliquity * v[1] - sin_obliquity * v[2],
                sin_obliquity * v[1] + cos_obliquity * v[2],
            ]
        };
        let position_icrf = rotate(position);
        let velocity_icrf = rotate(velocity);
        Ok(StateVector {
            position_km: [
                position_icrf[0] * KM_PER_AU,
                position_icrf[1] * KM_PER_AU,
                position_icrf[2] * KM_PER_AU,
            ],
            velocity_km_s: [
                velocity_icrf[0] * KM_PER_AU / SECONDS_PER_DAY,
                velocity_icrf[1] * KM_PER_AU / SECONDS_PER_DAY,
                velocity_icrf[2] * KM_PER_AU / SECONDS_PER_DAY,
            ],
        })
    }
}

#[cfg(test)]
#[expect(clippy::expect_used)]
#[allow(clippy::indexing_slicing)]
mod tests {
    use super::*;
    use stkcraft_time::JulianDate;

    fn tdb(et: f64) -> Epoch {
        Epoch::from_julian(
            JulianDate::from_parts(JD_J2000, et / 86_400.0),
            TimeScale::Tdb,
        )
    }

    /// Heliocentric reference states from CSPICE `spkezr(id, et, 'J2000', 'NONE', '10')` over
    /// `de440s.bsp` and the satellite-system kernels, taken 2026-11-05.
    #[allow(clippy::unreadable_literal, clippy::excessive_precision)]
    const CSPICE_HELIOCENTRIC: [(i32, f64, [f64; 3]); 21] = [
        (
            199,
            -1577000000.0,
            [9918912.282022, 40042897.280941, 20355811.062922],
        ),
        (
            199,
            0.0,
            [-19461726.355854, -59927967.773480, -29992772.679831],
        ),
        (
            199,
            1577000000.0,
            [22423559.928551, 36670536.168353, 17268552.213633],
        ),
        (
            299,
            -1577000000.0,
            [-16711974.420891, 96531628.719929, 44469989.374697],
        ),
        (
            299,
            0.0,
            [-107456494.052191, -6922528.774883, 3686187.045621],
        ),
        (
            299,
            1577000000.0,
            [-9252027.890409, -98993939.967515, -43967146.042250],
        ),
        (
            399,
            -1577000000.0,
            [-52932559.311749, 125933032.238561, 54615438.091015],
        ),
        (
            399,
            0.0,
            [-26499033.677425, 132757417.338339, 57556718.470538],
        ),
        (
            399,
            1577000000.0,
            [844253.436784, 135033926.774736, 58526221.528907],
        ),
        (
            499,
            -1577000000.0,
            [-218852211.298484, 105843587.996857, 54486851.707433],
        ),
        (499, 0.0, [208048140.641842, 209619.173539, -5529162.313155]),
        (
            499,
            1577000000.0,
            [-237440087.637165, -53809273.057500, -18291362.408645],
        ),
        (
            599,
            -1577000000.0,
            [517948326.363204, -504782773.403904, -229019347.157438],
        ),
        (
            599,
            0.0,
            [598567624.657064, 409386305.984162, 160894326.877569],
        ),
        (
            599,
            1577000000.0,
            [-347269739.693159, 642499851.823708, 283820679.445440],
        ),
        (
            699,
            -1577000000.0,
            [-1350179056.010355, 316874879.229573, 188852394.111728],
        ),
        (
            699,
            0.0,
            [958385358.915722, 923715471.242273, 340300858.458376],
        ),
        (
            699,
            1577000000.0,
            [706005417.539951, -1205792928.314793, -528570479.869385],
        ),
        (
            799,
            -1577000000.0,
            [-191735875.877377, 2588917181.863132, 1136616002.680740],
        ),
        (
            799,
            0.0,
            [2158975038.179658, -1870910813.311458, -849968711.476374],
        ),
        (
            799,
            1577000000.0,
            [-2664924642.308645, 549813611.656335, 278464987.709489],
        ),
    ];

    #[test]
    fn the_truncated_series_reproduces_cspice_to_an_arcsecond() {
        // Measured against CSPICE over fifteen states; the truncation at 1e-7 au is what sets the
        // floor, and an arcsecond is a comfortable bound over it (Mercury comes out at 0.04″).
        let ephemeris = Vsop87Ephemeris;
        let mut worst: f64 = 0.0;
        for (naif, et, expected) in CSPICE_HELIOCENTRIC {
            let Some(body) = Body::from_naif_id(naif) else {
                continue;
            };
            let state = ephemeris
                .state(body, tdb(et))
                .expect("VSOP87 carries the planets");
            let error = distance_km(state.position_km, expected);
            let radius =
                (expected[0] * expected[0] + expected[1] * expected[1] + expected[2] * expected[2])
                    .sqrt();
            let arcsec = (error / radius).atan().to_degrees() * 3_600.0;
            worst = worst.max(arcsec);
            assert!(
                arcsec < 1.0,
                "{naif} at {et}: {arcsec:.3} arcsec ({error:.0} km) from CSPICE"
            );
        }
        assert!(worst > 0.0, "no state was compared");
    }

    fn distance_km(a: [f64; 3], b: [f64; 3]) -> f64 {
        let mut sum = 0.0;
        for axis in 0..3 {
            let delta = a[axis] - b[axis];
            sum += delta * delta;
        }
        sum.sqrt()
    }

    #[test]
    fn the_velocity_is_the_time_derivative_of_the_position() {
        // A numerical check on the analytic derivative: it catches a factor or a sign in the series
        // differentiation, which a position-only test cannot see.
        let ephemeris = Vsop87Ephemeris;
        let step = 10.0;
        let state = ephemeris.state(Body::Mars, tdb(0.0)).expect("valid");
        let before = ephemeris.state(Body::Mars, tdb(-step)).expect("valid");
        let after = ephemeris.state(Body::Mars, tdb(step)).expect("valid");
        for axis in 0..3 {
            let numeric = (after.position_km[axis] - before.position_km[axis]) / (2.0 * step);
            assert!(
                (numeric - state.velocity_km_s[axis]).abs() < 1e-4,
                "axis {axis}: numeric {numeric} against analytic {}",
                state.velocity_km_s[axis]
            );
        }
    }

    #[test]
    fn the_moon_and_the_sun_are_refused_by_name() {
        // VSOP87 is a planetary theory: the Moon needs ELP2000 and the Sun has no heliocentric
        // series of its own.
        let ephemeris = Vsop87Ephemeris;
        for body in [Body::Moon, Body::Sun] {
            let error = ephemeris
                .state(body, tdb(0.0))
                .expect_err("not in the table");
            assert!(matches!(error, Error::Unsupported { .. }), "{error:?}");
        }
    }

    #[test]
    fn the_origin_is_the_sun() {
        assert!(matches!(Vsop87Ephemeris.origin(), Origin::Sun));
    }
}
