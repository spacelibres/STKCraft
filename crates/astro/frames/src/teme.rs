//! Converting between TEME and the frames the rest of the product works in.
//!
//! SGP4 is defined in **TEME** (True Equator, Mean Equinox): it shares the true equator with
//! the equator-of-date frame but takes the *mean* equinox, so it differs from TOD by exactly
//! the equation of the equinoxes. Getting back to an inertial frame therefore means
//!
//! ```text
//! r_GCRF = P * N * E(T) * r_TEME
//! ```
//!
//! where `E` rotates about the z axis through the geometric part of the equation of the
//! equinoxes, `N` is nutation and `P` is precession. Getting to an Earth-fixed frame instead
//! means
//!
//! ```text
//! r_ITRF = W * R3(gmst) * r_TEME
//! ```
//!
//! with `W` the polar-motion rotation.
//!
//! Source: Vallado, *Fundamentals of Astrodynamics and Applications* (2013), Algorithms 23 and
//! 26, eq. 3-77 and eq. 3-84.
//!
//! # Why the state type is defined here
//!
//! [`CartesianState`] is declared in this crate rather than reusing `stkcraft_astro_sgp4::StateVector`
//! because `stkcraft-astro-frames` and `stkcraft-astro-sgp4` are on the same layer and `cargo xtask layers`
//! forbids a same-layer dependency. That is the right call on its own terms as well: a frame
//! conversion has no business knowing how a state vector was produced. Callers convert
//! explicitly, which keeps the boundary visible.

use stkcraft_core::{Distance, Error, Result, Velocity};

use crate::precession;
use crate::rotation::Rotation3;
use crate::{nutation, sidereal};

/// A position and velocity in some frame, in kilometres and kilometres per second.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CartesianState {
    /// Position, km.
    pub position: [Distance; 3],
    /// Velocity, km/s.
    pub velocity: [Velocity; 3],
}

impl CartesianState {
    /// The distance from the origin, km.
    #[must_use]
    pub fn radius(&self) -> f64 {
        let mut sum = 0.0;
        for component in self.position {
            let km = component.as_km();
            sum += km * km;
        }
        sum.sqrt()
    }

    /// The speed, km/s.
    #[must_use]
    pub fn speed(&self) -> f64 {
        let mut sum = 0.0;
        for component in self.velocity {
            let km_per_s = component.as_km_s();
            sum += km_per_s * km_per_s;
        }
        sum.sqrt()
    }
}

/// The Earth-orientation parameters a TEME to Earth-fixed conversion needs.
///
/// These are real inputs, not refinements: `UT1 - UTC` reaches 0.9 s, which is 13 arcseconds of
/// Earth rotation and therefore hundreds of metres at orbital altitude. They are never
/// defaulted silently — providing them is the caller's job, and until there is an Earth
/// orientation file loader this type is how they arrive.
///
/// Source for the ranges: IERS Conventions (2010), chapter 8. Polar motion stays below about
/// 0.5 arcsecond, and `UT1 - UTC` below 0.9 s by construction of the leap-second system.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EarthOrientation {
    /// `UT1 - UTC`, seconds.
    pub dut1_seconds: f64,
    /// Polar motion `x` coordinate, radians.
    pub x_polar_rad: f64,
    /// Polar motion `y` coordinate, radians.
    pub y_polar_rad: f64,
}

impl EarthOrientation {
    /// Earth orientation with the effect of `UT1 - UTC` and polar motion ignored.
    ///
    /// Named rather than provided as `Default` so that choosing to ignore the terms is a
    /// visible decision at the call site. It costs up to about a kilometre of ground position.
    #[must_use]
    pub const fn negligible() -> Self {
        Self {
            dut1_seconds: 0.0,
            x_polar_rad: 0.0,
            y_polar_rad: 0.0,
        }
    }

    /// Whether the values are finite and within the range the IERS conventions allow.
    fn validate(&self) -> Result<()> {
        if !self.dut1_seconds.is_finite() {
            return Err(Error::invalid("UT1 - UTC", "must be finite"));
        }
        if self.dut1_seconds.abs() > 1.0 {
            return Err(Error::out_of_range(
                "UT1 - UTC",
                self.dut1_seconds,
                "within one second, per the IERS conventions",
            ));
        }
        for (name, value) in [
            ("polar motion x", self.x_polar_rad),
            ("polar motion y", self.y_polar_rad),
        ] {
            if !value.is_finite() {
                return Err(Error::invalid(name, "must be finite"));
            }
            // A pole coordinate is under an arcsecond, so 1e-4 rad (about 20") is already
            // a unit mistake. Applying it silently would tilt the Earth by kilometres.
            if value.abs() > 1.0e-4 {
                return Err(Error::out_of_range(
                    name,
                    format!("{value} rad"),
                    "under 1e-4 rad (about 20 arcseconds)",
                ));
            }
        }
        Ok(())
    }
}

/// How the equation-of-equinoxes terms are handled in the Earth-fixed conversion.
///
/// This is not a preference. The standard changed in 1997: before that the geometric
/// expression was in use, afterwards the kinematic terms were added. Applying the wrong one
/// costs about 15 metres of ground position, so the choice is an explicit argument.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EquinoxTerms {
    /// Geometric terms only, correct for dates before 1997.
    Geometric,
    /// Geometric plus kinematic terms, required for dates after 1997.
    Kinematic,
}

/// The Julian Date after which the kinematic equinox terms apply, `1997 January 1 00:00 UT1`.
///
/// Source: the reference implementation's `if (jdut1 > 2450449.5)` test in `teme_ecef`.
const KINEMATIC_TERMS_FROM_JD: f64 = 2_450_449.5;

/// The IAU-80 nutation rotation, taking true-of-date to mean-of-date (TOD to MOD).
///
/// Source: Vallado (2013), eq. 3-72 through 3-74, as formed in the reference `nutation`
/// routine. `true_eps = mean_eps + delta_eps`.
fn nutation_matrix(delta_psi: f64, delta_eps: f64, mean_eps: f64) -> Result<Rotation3> {
    let true_eps = mean_eps + delta_eps;
    let (sin_psi, cos_psi) = delta_psi.sin_cos();
    let (sin_mean, cos_mean) = mean_eps.sin_cos();
    let (sin_true, cos_true) = true_eps.sin_cos();

    Rotation3::from_rows([
        [cos_psi, cos_true * sin_psi, sin_true * sin_psi],
        [
            -cos_mean * sin_psi,
            cos_true * cos_mean * cos_psi + sin_true * sin_mean,
            sin_true * cos_mean * cos_psi - sin_mean * cos_true,
        ],
        [
            -sin_mean * sin_psi,
            cos_true * sin_mean * cos_psi - sin_true * cos_mean,
            sin_true * sin_mean * cos_psi + cos_true * cos_mean,
        ],
    ])
}

/// Apply a rotation to a position and velocity.
///
/// Velocity needs no special handling for an inertial-to-inertial rotation: the rotation is
/// constant over the instant, so it acts on the velocity exactly as it acts on the position.
/// The Earth-fixed conversion is different — see [`teme_to_itrf`].
fn rotate(rotation: Rotation3, state: &CartesianState) -> CartesianState {
    let mut position = [Distance::from_km(0.0); 3];
    let mut velocity = [Velocity::from_km_s(0.0); 3];

    // `zip` over two fixed three-element arrays yields exactly three rows, so the outputs are
    // filled completely and `into_iter` hands them back without an index.
    let rows = rotation.rows();
    for (row, (out_p, out_v)) in rows
        .into_iter()
        .zip(position.iter_mut().zip(velocity.iter_mut()))
    {
        let mut p = 0.0;
        let mut v = 0.0;
        for (weight, (x, w)) in row
            .into_iter()
            .zip(state.position.iter().zip(state.velocity.iter()))
        {
            p += weight * x.as_km();
            v += weight * w.as_km_s();
        }
        *out_p = Distance::from_km(p);
        *out_v = Velocity::from_km_s(v);
    }

    CartesianState { position, velocity }
}

/// TEME to GCRF: the inertial frame the rest of the product works in.
///
/// # Errors
///
/// Returns [`Error::Invalid`] for a non-finite time, or if the constructed rotation is not a
/// proper rotation.
pub fn teme_to_gcrf(ttt: f64, state: &CartesianState) -> Result<CartesianState> {
    let delta_psi = nutation::iau_1980(ttt)?.longitude;
    let nutation = nutation::iau_1980(ttt)?;
    let mean_eps = nutation::mean_obliquity(ttt)?;

    // TEME is defined against the *mean* equinox of date, so the whole difference between it
    // and true-of-date is the equation of the equinoxes. Only the geometric part belongs here:
    // the kinematic part was a later refinement for Earth-fixed work and is not part of the
    // TEME definition.
    let equation_of_equinoxes = delta_psi * mean_eps.cos();

    // `Rotation3::about_z` is an active rotation, so `about_z(angle)` has the same rows as the
    // reference's `eqe` matrix, which the reference then transposes to build the conversion.
    // Using it un-transposed is therefore the correct form, not an omission.
    let to_gcrf = precession::iau_1976_matrix(ttt)?
        * nutation_matrix(nutation.longitude, nutation.obliquity, mean_eps)?
        * Rotation3::about_z(equation_of_equinoxes);

    Ok(rotate(to_gcrf, state))
}

/// TEME to ITRF, the Earth-fixed frame a ground station is fixed in.
///
/// The velocity is not simply rotated. The Earth is turning at `omega = 7.292115e-5 rad/s`, so
/// a velocity carried into the rotating frame gains `-omega * r`, and the conversion has to
/// remove it before the polar-motion rotation is applied. That cross product is what makes
/// this function longer than the GCRF one.
///
/// # Errors
///
/// Returns [`Error::Invalid`] for a non-finite time, or [`Error::OutOfRange`] for Earth
/// orientation parameters outside the IERS ranges.
pub fn teme_to_itrf(
    jdut1: f64,
    ttt: f64,
    eop: &EarthOrientation,
    terms: EquinoxTerms,
    state: &CartesianState,
) -> Result<CartesianState> {
    if !jdut1.is_finite() {
        return Err(Error::invalid("UT1 Julian Date", "must be finite"));
    }
    if !ttt.is_finite() {
        return Err(Error::invalid("Julian centuries of TT", "must be finite"));
    }
    eop.validate()?;

    // The sidereal angle used is the mean one, plus the kinematic terms when they are in
    // force. Before 1997 those terms were not applied, and applying them to an old date would
    // be as wrong as omitting them from a new one.
    let mut theta = sidereal::greenwich_mean(jdut1)?;
    if terms == EquinoxTerms::Kinematic && jdut1 > KINEMATIC_TERMS_FROM_JD {
        // Source: Vallado (2013), eq. 3-83. The coefficients 0.00264 and 0.000063 arcseconds
        // are the kinematic terms, and the argument is the lunar node longitude.
        let arcsec_to_rad = core::f64::consts::PI / 648_000.0;
        let omega = crate::arguments::iau_1980(ttt)?.moon_node_longitude;
        theta += 0.002_64 * arcsec_to_rad * omega.sin()
            + 0.000_063 * arcsec_to_rad * (2.0 * omega).sin();
    }

    // Rotate into the pseudo-Earth-fixed frame. `Rotation3::about_z(theta)` is an *active*
    // rotation, so converting a vector *into* the rotated frame uses its transpose.
    let to_pef = Rotation3::about_z(theta).transpose();
    let rotated = rotate(to_pef, state);

    // Remove the Earth's rotation from the velocity. A point fixed in the rotating frame moves
    // at `omega x r` when seen from inertial space, so the inertial velocity minus that is the
    // velocity measured in the rotating frame:
    //
    //     v_pef = S^T v_teme - omega x r_pef
    //
    // with `omega = [0, 0, thetasa]`, so the cross product is `[-thetasa r_y, thetasa r_x, 0]`.
    // Source: Vallado (2013), Algorithm 26 and eq. 3-84.
    let omega = EARTH_ROTATION_RAD_PER_SECOND;
    let rx = rotated.position[0].as_km();
    let ry = rotated.position[1].as_km();
    let pef = CartesianState {
        position: rotated.position,
        velocity: [
            Velocity::from_km_s(rotated.velocity[0].as_km_s() + omega * ry),
            Velocity::from_km_s(rotated.velocity[1].as_km_s() - omega * rx),
            Velocity::from_km_s(rotated.velocity[2].as_km_s()),
        ],
    };

    // Polar motion: a rotation about x through the y pole coordinate, then about y through
    // the x pole coordinate. The signs follow the definition of the pole coordinates in the
    // published reference, not a stylistic choice.
    let polar = Rotation3::about_y(eop.x_polar_rad) * Rotation3::about_x(eop.y_polar_rad);
    Ok(rotate(polar, &pef))
}

/// The Earth's rotation rate, radians per second.
///
/// Source: Vallado (2013), eq. 3-84, `7.292115e-5 rad/s`, the WGS-72 value the reference
/// `teme_ecef` uses as `earthrot`.
const EARTH_ROTATION_RAD_PER_SECOND: f64 = 7.292_115e-5;
#[cfg(test)]
#[expect(clippy::expect_used)]
mod tests {
    use super::*;

    const TTT: f64 = 0.042_623_631_9;
    const JDUT1: f64 = 2_453_101.827_406_783;

    /// Vallado, Fundamentals (2013), Example 3-15: the TEME state at the epoch of the
    /// canonical two-line element set, 2004 April 6 07:51:28.386009 UTC.
    fn example_state() -> CartesianState {
        CartesianState {
            position: [
                Distance::from_km(5_094.180_162_1),
                Distance::from_km(6_127.644_659_5),
                Distance::from_km(6_380.344_532_7),
            ],
            velocity: [
                Velocity::from_km_s(-4.746_131_487),
                Velocity::from_km_s(0.785_818_041),
                Velocity::from_km_s(5.531_931_288),
            ],
        }
    }

    fn assert_close(got: &CartesianState, r: [f64; 3], v: [f64; 3], tol: f64) {
        let got_r: Vec<f64> = got.position.iter().map(|d| d.as_km()).collect();
        let got_v: Vec<f64> = got.velocity.iter().map(|d| d.as_km_s()).collect();
        for (axis, ((g, e), (gv, ev))) in got_r
            .iter()
            .zip(r.iter())
            .zip(got_v.iter().zip(v.iter()))
            .enumerate()
        {
            assert!(
                (g - e).abs() < tol,
                "position[{axis}] was {g}, expected {e}"
            );
            assert!(
                (gv - ev).abs() < tol,
                "velocity[{axis}] was {gv}, expected {ev}"
            );
        }
    }

    #[test]
    fn teme_to_gcrf_reproduces_the_worked_example() {
        // Reference: Vallado `AstroLib.cpp :: teme_eci`, 1980 option, ddpsi = ddeps = 0, at
        // the Example 3-15 instant.
        let out = teme_to_gcrf(TTT, &example_state()).expect("finite time");
        assert_close(
            &out,
            [5_102.509_654_902, 6_123.011_474_285, 6_378.136_299_923],
            [-4.743_219_593, 0.790_536_643, 5.533_756_190],
            1e-6,
        );
    }

    #[test]
    fn teme_to_itrf_with_kinematic_terms_reproduces_the_worked_example() {
        // Reference: `AstroLib.cpp :: teme_ecef` with eqeterms = 1, required after 1997.
        let eop = EarthOrientation::negligible();
        let out = teme_to_itrf(JDUT1, TTT, &eop, EquinoxTerms::Kinematic, &example_state())
            .expect("finite inputs");
        assert_close(
            &out,
            [-1_033.474_960_481, 7_901.305_594_843, 6_380.344_532_7],
            [-3.225_632_774, -2.872_442_481, 5.531_931_288],
            1e-6,
        );
    }

    #[test]
    fn teme_to_itrf_with_geometric_terms_reproduces_its_own_reference() {
        let eop = EarthOrientation::negligible();
        let out = teme_to_itrf(JDUT1, TTT, &eop, EquinoxTerms::Geometric, &example_state())
            .expect("finite inputs");
        assert_close(
            &out,
            [-1_033.475_031_344, 7_901.305_585_574, 6_380.344_532_7],
            [-3.225_632_748, -2.872_442_510, 5.531_931_288],
            1e-6,
        );
    }

    #[test]
    fn a_frame_change_preserves_radius_and_speed() {
        // Both conversions are rotations, so |r| is invariant. This catches a matrix that is
        // orthogonalised wrongly, and it is the only test here that would notice a transposed
        // rotation when no reference table is at hand.
        let state = example_state();
        let gcrf = teme_to_gcrf(TTT, &state).expect("finite time");
        assert!((gcrf.radius() - state.radius()).abs() < 1e-9);
        assert!((gcrf.speed() - state.speed()).abs() < 1e-12);

        // The Earth-fixed *position* is still a pure rotation, but the velocity is not,
        // because the rotating frame has its own motion. Only the radius is invariant there.
        let eop = EarthOrientation::negligible();
        let itrf =
            teme_to_itrf(JDUT1, TTT, &eop, EquinoxTerms::Kinematic, &state).expect("finite inputs");
        assert!((itrf.radius() - state.radius()).abs() < 1e-9);
    }

    #[test]
    fn the_earth_fixed_velocity_carries_the_rotation_term() {
        // Guard against silently "rotating" the velocity and calling it Earth-fixed: the
        // omega x r term is about 0.5 km/s at low Earth orbit, so a conversion that omits it
        // must not pass.
        let state = example_state();
        let eop = EarthOrientation::negligible();
        let itrf =
            teme_to_itrf(JDUT1, TTT, &eop, EquinoxTerms::Geometric, &state).expect("finite inputs");
        let theta = sidereal::greenwich_mean(JDUT1).expect("finite");
        let rotated_only = rotate(Rotation3::about_z(theta).transpose(), &state);
        let difference = (itrf.velocity[0].as_km_s() - rotated_only.velocity[0].as_km_s()).abs()
            + (itrf.velocity[1].as_km_s() - rotated_only.velocity[1].as_km_s()).abs();
        assert!(
            difference > 0.1,
            "the rotation term was missing: velocities differ by only {difference} km/s"
        );
    }

    #[test]
    fn a_non_finite_time_is_refused() {
        let eop = EarthOrientation::negligible();
        assert!(teme_to_gcrf(f64::NAN, &example_state()).is_err());
        assert!(
            teme_to_itrf(
                f64::NAN,
                TTT,
                &eop,
                EquinoxTerms::Kinematic,
                &example_state()
            )
            .is_err()
        );
        assert!(
            teme_to_itrf(
                JDUT1,
                f64::NAN,
                &eop,
                EquinoxTerms::Kinematic,
                &example_state()
            )
            .is_err()
        );
    }

    #[test]
    fn impossible_earth_orientation_is_refused() {
        // A pole coordinate of a radian is a unit mistake (degrees where radians belong), not
        // a real value, and applying it would tilt the Earth by thousands of kilometres.
        let bad_pole = EarthOrientation {
            dut1_seconds: 0.0,
            x_polar_rad: 1.0,
            y_polar_rad: 0.0,
        };
        assert!(
            teme_to_itrf(
                JDUT1,
                TTT,
                &bad_pole,
                EquinoxTerms::Kinematic,
                &example_state()
            )
            .is_err()
        );

        let bad_dut1 = EarthOrientation {
            dut1_seconds: 60.0,
            x_polar_rad: 0.0,
            y_polar_rad: 0.0,
        };
        assert!(
            teme_to_itrf(
                JDUT1,
                TTT,
                &bad_dut1,
                EquinoxTerms::Kinematic,
                &example_state()
            )
            .is_err()
        );
    }
}
