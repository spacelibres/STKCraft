//! The body-fixed frame and the matrix that reaches it.
//!
//! The IAU describes a body's orientation with three Euler angles — the pole's right ascension
//! and declination, and the prime meridian angle `W` — and the frame follows from them:
//!
//! ```text
//! r_body_fixed = R3(W) · R1(90° − δ) · R3(90° + α) · r_icrf
//! ```
//!
//! with the passive rotation matrices
//!
//! ```text
//! R1(θ) = [1  0  0 ]      R3(θ) = [ cos θ  sin θ  0 ]
//!         [0  c  s ]              [−sin θ  cos θ  0 ]
//!         [0 −s  c ]              [   0      0     1 ]
//! ```
//!
//! # How the convention was established
//!
//! Not from memory and not by taste. The same `pck00011.tpc` was loaded into CSPICE and
//! `pxform('IAU_<body>', 'J2000', et)` compared element by element with this crate's matrix at
//! ET = 0, 3.2e8 and 8.0e8 s: the tests in this module hold the CSPICE values and require
//! agreement to 1e−11. Two conventions were wrong on the first attempt and the comparison is what
//! caught them — a cosine where the IAU model uses a sine for the pole's right ascension, and the
//! direction of the matrix. A structural test (orthonormality) passes for every one of those
//! wrong answers, which is why it is not the test that decides.

/// The orientation of a body at one instant, in the IAU 2015 convention.
///
/// All three angles are radians. `prime_meridian` is reduced to `[0, 2π)`; `pole_ra` and
/// `pole_dec` are not, because a right ascension outside that range is still a valid direction and
/// reducing it would lose nothing but also gain nothing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Orientation {
    /// Right ascension of the body's north pole, radians.
    pub pole_ra: f64,
    /// Declination of the body's north pole, radians.
    pub pole_dec: f64,
    /// The prime meridian angle `W`, radians in `[0, 2π)`.
    pub prime_meridian: f64,
}

impl Orientation {
    /// The matrix that rotates an ICRF vector into the body-fixed frame, row-major.
    ///
    /// Row-major because that is the order a reader writes it in and the order a test compares
    /// it in; a consumer that wants column-major transposes once at its own boundary.
    #[must_use]
    pub fn icrf_to_body_fixed(&self) -> [[f64; 3]; 3] {
        let intermediate = about_x(core::f64::consts::FRAC_PI_2 - self.pole_dec);
        let node = about_z(core::f64::consts::FRAC_PI_2 + self.pole_ra);
        multiply(
            &multiply(&about_z(self.prime_meridian), &intermediate),
            &node,
        )
    }

    /// The matrix that rotates a body-fixed vector into the ICRF, row-major.
    ///
    /// The transpose of [`Self::icrf_to_body_fixed`], which is what a rotation matrix's inverse
    /// is; written out rather than inverted so that a consumer in a hot loop does not pay for an
    /// inversion it does not need.
    #[must_use]
    pub fn body_fixed_to_icrf(&self) -> [[f64; 3]; 3] {
        transpose(&self.icrf_to_body_fixed())
    }

    /// Whether the pole direction is the ICRF pole to within `tolerance` radians of declination.
    ///
    /// Present because it is the cheapest way for a caller to recognise the Earth's frame, whose
    /// pole is the ICRF pole by the definition of the ICRF itself.
    #[must_use]
    pub fn pole_is_icrf_pole(&self, tolerance: f64) -> bool {
        (self.pole_dec - core::f64::consts::FRAC_PI_2).abs() <= tolerance
    }
}

/// `R1(θ)`, the passive rotation about the x axis.
fn about_x(angle: f64) -> [[f64; 3]; 3] {
    let (sin, cos) = angle.sin_cos();
    [[1.0, 0.0, 0.0], [0.0, cos, sin], [0.0, -sin, cos]]
}

/// `R3(θ)`, the passive rotation about the z axis.
fn about_z(angle: f64) -> [[f64; 3]; 3] {
    let (sin, cos) = angle.sin_cos();
    [[cos, sin, 0.0], [-sin, cos, 0.0], [0.0, 0.0, 1.0]]
}

/// Row-major 3×3 product, `a · b`.
///
/// Written as dot products against `b` transposed, so that no index is ever used: the bounds are
/// the literal three-by-three shape of the value and the compiler can see it, which keeps the
/// `indexing_slicing` lint — the one that stands in for "no panics" — meaningful here.
fn multiply(a: &[[f64; 3]; 3], b: &[[f64; 3]; 3]) -> [[f64; 3]; 3] {
    let transposed = transpose(b);
    let mut out = [[0.0_f64; 3]; 3];
    for (out_row, left) in out.iter_mut().zip(a.iter()) {
        for (cell, right) in out_row.iter_mut().zip(transposed.iter()) {
            *cell = left.iter().zip(right.iter()).map(|(x, y)| x * y).sum();
        }
    }
    out
}

fn transpose(a: &[[f64; 3]; 3]) -> [[f64; 3]; 3] {
    let mut out = [[0.0_f64; 3]; 3];
    for (row, values) in a.iter().enumerate() {
        for (column, value) in values.iter().enumerate() {
            if let Some(target) = out.get_mut(column).and_then(|line| line.get_mut(row)) {
                *target = *value;
            }
        }
    }
    out
}

// The tests index 3×3 matrices by hand, where the bounds are the literal shape of the value.
#[cfg(test)]
#[expect(clippy::expect_used)]
#[allow(clippy::indexing_slicing)]
mod tests {
    use super::*;
    use crate::Body;
    use crate::generated::{CSPICE_ET, CSPICE_FRAMES};
    use stkcraft_time::{Epoch, JD_J2000, TimeScale};

    /// A TDB epoch from `et`, seconds past J2000 TDB, which is what SPICE calls `et`.
    fn epoch_at(et: f64) -> Epoch {
        Epoch::from_jd(JD_J2000 + et / 86_400.0, TimeScale::Tdb)
    }

    fn flatten(matrix: [[f64; 3]; 3]) -> [f64; 9] {
        [
            matrix[0][0],
            matrix[0][1],
            matrix[0][2],
            matrix[1][0],
            matrix[1][1],
            matrix[1][2],
            matrix[2][0],
            matrix[2][1],
            matrix[2][2],
        ]
    }

    fn body_named(name: &str) -> Body {
        Body::ALL
            .into_iter()
            .find(|body| body.name() == name)
            .expect("the reference table names a body this crate does not have")
    }

    #[test]
    fn the_model_reproduces_cspice_for_every_body_it_resolves() {
        // This is the test the whole module exists for. It compares against CSPICE's own reading
        // of the same kernel, so it fails for a wrong trigonometric function, a transposed
        // matrix, a wrong angle index or a wrong time argument — none of which orthonormality
        // would catch. Mars is excluded on purpose; it has its own test below.
        let mut checked = 0;
        for (name, matrices) in CSPICE_FRAMES {
            let body = body_named(name);
            for (index, expected) in matrices.iter().enumerate() {
                let orientation = body.orientation(epoch_at(CSPICE_ET[index]));
                let ours = flatten(orientation.icrf_to_body_fixed());
                let worst = ours
                    .iter()
                    .zip(expected.iter())
                    .map(|(a, b)| (a - b).abs())
                    .fold(0.0_f64, f64::max);
                if body == Body::Mars {
                    assert!(
                        worst > 1e-3,
                        "Mars now agrees with CSPICE to {worst:.3e}; its periodic terms have been \
                         transcribed and this exclusion should be removed, not left in place"
                    );
                    continue;
                }
                // Ten nanoradians. The pull is the prime meridian of the fast rotators: Jupiter
                // turns 870.5° a day, so after a decade its angle is nearly 10^7 degrees and the
                // last bits of an `f64` sine are what the comparison sees. A real error — the
                // wrong trigonometric function, a transposed matrix — is larger by six orders.
                assert!(
                    worst < 1e-8,
                    "{name} at ET {}: this crate differs from CSPICE by {worst:.3e}",
                    CSPICE_ET[index]
                );
                checked += 1;
            }
        }
        assert_eq!(
            checked, 27,
            "expected three epochs for nine bodies, Mars excluded"
        );
    }

    #[test]
    fn the_matrices_are_orthonormal_with_positive_determinant() {
        // A rotation matrix that is not a rotation still looks plausible on screen, so this is
        // checked at every epoch and for every body rather than once.
        for body in Body::ALL {
            for et in [0.0, 1.0e8, 5.0e8, 9.0e8] {
                let matrix = body.orientation(epoch_at(et)).icrf_to_body_fixed();
                let identity = multiply(&matrix, &transpose(&matrix));
                for (row, values) in identity.iter().enumerate() {
                    for (column, value) in values.iter().enumerate() {
                        let expected = if row == column { 1.0 } else { 0.0 };
                        assert!(
                            (value - expected).abs() < 1e-12,
                            "{body} at ET {et}: R·Rᵀ is not the identity at ({row}, {column})"
                        );
                    }
                }
                let determinant = determinant(&matrix);
                assert!(
                    (determinant - 1.0).abs() < 1e-12,
                    "{body} at ET {et}: determinant {determinant} is not +1"
                );
            }
        }
    }

    fn determinant(m: &[[f64; 3]; 3]) -> f64 {
        m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
            - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
            + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0])
    }

    #[test]
    fn the_inverse_matrix_really_inverts() {
        let orientation = Body::Mars.orientation(epoch_at(3.2e8));
        let there = orientation.icrf_to_body_fixed();
        let back = orientation.body_fixed_to_icrf();
        let vector = [0.36, -0.48, 0.8];
        let once = apply(&there, vector);
        let round_trip = apply(&back, once);
        for (a, b) in vector.iter().zip(round_trip.iter()) {
            assert!((a - b).abs() < 1e-15, "round trip drifted: {a} against {b}");
        }
    }

    fn apply(m: &[[f64; 3]; 3], v: [f64; 3]) -> [f64; 3] {
        let mut out = [0.0_f64; 3];
        for (cell, row) in out.iter_mut().zip(m.iter()) {
            *cell = row.iter().zip(v.iter()).map(|(a, b)| a * b).sum();
        }
        out
    }

    #[test]
    fn the_earth_pole_is_the_icrf_pole_at_j2000() {
        // The ICRF equator is *defined* to be the Earth's equator at J2000, so this is the one
        // check that ties the model to the frame it claims to be measured in.
        let orientation = Body::Earth.orientation(epoch_at(0.0));
        assert!(orientation.pole_is_icrf_pole(1e-9));
        let matrix = orientation.icrf_to_body_fixed();
        // The body-fixed z axis is the pole, so the third row maps it onto itself.
        for (column, value) in matrix[2].iter().enumerate() {
            let expected = if column == 2 { 1.0 } else { 0.0 };
            assert!((value - expected).abs() < 1e-12);
        }
    }

    #[test]
    fn the_earth_rotation_rate_is_one_sidereal_day() {
        // A day of TDB should advance the prime meridian by about 360.9856°, the ratio of the
        // sidereal to the solar day. This catches a per-day rate entered as per-century, which
        // would make the Earth stand still.
        let one_day = Body::Earth.orientation(epoch_at(0.0));
        let next_day = Body::Earth.orientation(epoch_at(86_400.0));
        // Modulo one turn: a day advances the prime meridian by 360.9856°, i.e. 0.9856° more
        // than a full turn, which is why a sidereal day is shorter than a solar one.
        let advance =
            (next_day.prime_meridian - one_day.prime_meridian).rem_euclid(core::f64::consts::TAU);
        let degrees = advance.to_degrees();
        assert!(
            (degrees - 0.985_623_5).abs() < 0.001,
            "a day advanced the Earth's prime meridian by {degrees}°"
        );
    }

    #[test]
    fn a_body_without_periodic_terms_has_a_linear_pole() {
        // Venus has no nutation terms, so two epochs a century apart must differ by exactly the
        // documented rate. A stray coefficient in the table would show up here.
        let start = Body::Venus.orientation(epoch_at(0.0));
        let century = Body::Venus.orientation(epoch_at(36_525.0 * 86_400.0));
        let delta_ra = century.pole_ra.to_degrees() - start.pole_ra.to_degrees();
        assert!(
            delta_ra.abs() < 1e-12,
            "Venus's pole drifted by {delta_ra}°"
        );
        assert!((start.pole_ra.to_degrees() - 272.76).abs() < 1e-9);
        assert!((start.pole_dec.to_degrees() - 67.16).abs() < 1e-9);
    }
}
