//! IAU-77 precession, which takes the mean equator and equinox of date to the mean equator
//! and equinox of J2000 (MOD to GCRF).
//!
//! Source: Vallado, *Fundamentals of Astrodynamics and Applications* (2013), section 3.7 and
//! eq. 3-56 through 3-58. This is the IAU-77 theory, the one the reference implementation
//! selects with its 1980 option, and the one that belongs with the IAU-80 nutation series in
//! [`crate::nutation`].

use stkcraft_framework_core::{Error, Result};

use crate::rotation::Rotation3;

/// Arcseconds to radians.
const ARCSEC_TO_RAD: f64 = core::f64::consts::PI / 648_000.0;

/// The three IAU-77 precession angles, radians.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PrecessionAngles {
    /// `zeta`, the accumulated precession in right ascension at the moving equinox.
    pub zeta: f64,
    /// `theta`, the accumulated precession in declination.
    pub theta: f64,
    /// `z`, the accumulated precession in right ascension at the fixed equinox.
    pub z: f64,
}

/// The IAU-77 precession angles for `ttt` Julian centuries of TT.
///
/// All three are odd polynomials in `t` that vanish at J2000, which is why the precession
/// matrix is the identity there.
///
/// # Errors
///
/// Returns [`Error::Invalid`] for a non-finite time.
pub fn iau_1977_angles(ttt: f64) -> Result<PrecessionAngles> {
    if !ttt.is_finite() {
        return Err(Error::Invalid {
            what: "Julian centuries of TT",
            reason: "must be finite".to_owned(),
        });
    }
    Ok(PrecessionAngles {
        zeta: ((0.017_998 * ttt + 0.301_88) * ttt + 2306.2181) * ttt * ARCSEC_TO_RAD,
        theta: ((-0.041_833 * ttt - 0.426_65) * ttt + 2004.3109) * ttt * ARCSEC_TO_RAD,
        z: ((0.018_203 * ttt + 1.094_68) * ttt + 2306.2181) * ttt * ARCSEC_TO_RAD,
    })
}

/// The rotation taking mean-of-date to mean-of-J2000 (MOD to GCRF).
///
/// The matrix is written out rather than composed from three axis rotations. That is how the
/// published reference forms it: the closed form saves two matrix multiplications and,
/// more importantly, the rounding they carry, which is visible at the fifteenth digit — the
/// precision the acceptance test compares at.
///
/// # Errors
///
/// Returns [`Error::Invalid`] for a non-finite time, or if the angles produce a matrix that
/// is not a proper rotation.
pub fn iau_1976_matrix(ttt: f64) -> Result<Rotation3> {
    let PrecessionAngles { zeta, theta, z } = iau_1977_angles(ttt)?;
    let (sin_zeta, cos_zeta) = zeta.sin_cos();
    let (sin_theta, cos_theta) = theta.sin_cos();
    let (sin_z, cos_z) = z.sin_cos();

    Rotation3::from_rows([
        [
            cos_zeta * cos_theta * cos_z - sin_zeta * sin_z,
            cos_zeta * cos_theta * sin_z + sin_zeta * cos_z,
            cos_zeta * sin_theta,
        ],
        [
            -sin_zeta * cos_theta * cos_z - cos_zeta * sin_z,
            -sin_zeta * cos_theta * sin_z + cos_zeta * cos_z,
            -sin_zeta * sin_theta,
        ],
        [-sin_theta * cos_z, -sin_theta * sin_z, cos_theta],
    ])
}

#[cfg(test)]
#[expect(clippy::expect_used)]
mod tests {
    use stkcraft_framework_core::Vector3;

    use super::*;

    const TTT: f64 = 0.042_623_631_9;

    #[test]
    fn the_angles_match_the_published_polynomials() {
        // Source: Vallado, Fundamentals (2013), eq. 3-56 through 3-58, the IAU-77 precession:
        //   zeta  = 2306.2181" t + 0.30188" t^2 + 0.017998" t^3
        //   theta = 2004.3109" t - 0.42665" t^2 - 0.041833" t^3
        //   z     = 2306.2181" t + 1.09468" t^2 + 0.018203" t^3
        // Evaluated here from the seconds form, so a wrong arcsecond constant in the
        // implementation cannot also be wrong in the expectation.
        let a = iau_1977_angles(TTT).expect("finite time");
        let t = TTT;
        let zeta = ((0.017_998 * t + 0.301_88) * t + 2306.2181) * t * ARCSEC_TO_RAD;
        let theta = ((-0.041_833 * t - 0.426_65) * t + 2004.3109) * t * ARCSEC_TO_RAD;
        let z = ((0.018_203 * t + 1.094_68) * t + 2306.2181) * t * ARCSEC_TO_RAD;
        assert!((a.zeta - zeta).abs() < 1e-18, "zeta was {}", a.zeta);
        assert!((a.theta - theta).abs() < 1e-18, "theta was {}", a.theta);
        assert!((a.z - z).abs() < 1e-18, "z was {}", a.z);
    }

    #[test]
    fn the_matrix_matches_the_reference_to_fifteen_digits() {
        // Reference: Vallado `AstroLib.cpp :: precess` with the 1980 option, evaluated at
        // ttt = 0.0426236319 and printed at full precision.
        let m = iau_1976_matrix(TTT).expect("finite time");
        let expected = [
            [
                0.999_999_459_981_008_6,
                0.000_953_149_924_920_8,
                0.000_414_177_391_960_8,
            ],
            [
                -0.000_953_149_924_921_4,
                0.999_999_545_752_487_7,
                -0.000_000_197_385_182_2,
            ],
            [
                -0.000_414_177_391_959_4,
                -0.000_000_197_388_074_4,
                0.999_999_914_228_520_9,
            ],
        ];
        for (i, (got, want)) in m.rows().iter().zip(expected).enumerate() {
            for (j, (g, w)) in got.iter().zip(want).enumerate() {
                assert!((g - w).abs() < 1e-15, "[{i}][{j}] was {g}, expected {w}");
            }
        }
    }

    #[test]
    fn the_matrix_is_a_proper_rotation_everywhere() {
        for ttt in [-0.5, -0.01, 0.0, TTT, 0.5, 1.0] {
            let m = iau_1976_matrix(ttt).expect("finite time");
            m.validate().expect("precession must be a proper rotation");
        }
    }

    #[test]
    fn precession_is_the_identity_at_j2000() {
        // At t = 0 every angle is zero, so the matrix must be exactly the identity. Asserting
        // equality rather than closeness would catch a matrix built with the angles in the
        // wrong roles.
        let m = iau_1976_matrix(0.0).expect("finite time");
        assert_eq!(m, Rotation3::IDENTITY);
    }

    #[test]
    fn precession_accumulates_forty_six_arcseconds_a_century() {
        // The IAU-77 angles sum to the accumulated rotation of the mean equator and equinox,
        // which is about 46"/century. It is *not* the general precession in longitude
        // (about 5029"/century): that figure also carries the motion of the ecliptic itself,
        // so a test written against 5029 would be measuring the wrong quantity and would
        // fail on a correct implementation. The two differ by a factor of about 109.
        //
        // This is the test that catches a degrees/arcseconds/radians mix-up: every other
        // test here shares the same unit constant, so they would all still pass.
        let angles = iau_1977_angles(0.01).expect("finite time");
        let total_arcsec = (angles.zeta + angles.z).to_degrees() * 3600.0;
        assert!(
            (total_arcsec - 46.12).abs() < 0.5,
            "accumulated {total_arcsec} arcsec per century"
        );

        // And the matrix must agree with the angles: rotating the J2000 x axis moves it about
        // that same total in the equatorial plane.
        let m = iau_1976_matrix(0.01).expect("finite time");
        let moved = m.rotate(Vector3::X);
        let matrix_arcsec = moved.y.atan2(moved.x).to_degrees().abs() * 3600.0;
        assert!(
            (matrix_arcsec - total_arcsec).abs() < 1e-6,
            "matrix moved {matrix_arcsec}\u{2033}, angles say {total_arcsec}\u{2033}"
        );
    }

    #[test]
    fn a_non_finite_time_is_refused() {
        assert!(iau_1977_angles(f64::NAN).is_err());
        assert!(iau_1977_angles(f64::INFINITY).is_err());
        assert!(iau_1976_matrix(f64::NAN).is_err());
        assert!(iau_1976_matrix(f64::NEG_INFINITY).is_err());
    }
}
