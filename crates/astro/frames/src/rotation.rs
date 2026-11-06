//! Rotations between reference frames.
//!
//! A frame rotation is a pure, proper rotation: three orthonormal axes, determinant +1,
//! no translation and no scale. Representing it as a dedicated type rather than a bare
//! matrix means the caller cannot accidentally build one by scaling a row, and the
//! `inverse` operation is always the transpose rather than a numerically worse matrix
//! inverse.
//!
//! Convention: a [`Rotation3`] is an **active** rotation — it turns a vector, and a
//! positive angle about an axis is counter-clockwise looking from the positive end of that
//! axis back toward the origin. A reference-frame conversion is the passive counterpart of
//! the same turn, so converting *into* a frame uses the rotation's [`Rotation3::transpose`].
//!
//! Keeping one meaning for the type means a caller never has to remember which of two
//! conventions a particular matrix was built with.

use core::fmt;
use core::ops::Mul;

use stkcraft_core::{Error, Result, Vector3};

/// A rotation between two reference frames, stored as a row-major 3x3 matrix.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rotation3 {
    rows: [[f64; 3]; 3],
}

impl Rotation3 {
    /// The identity rotation, which leaves every vector unchanged.
    pub const IDENTITY: Self = Self {
        rows: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
    };

    /// Build from a row-major 3x3 array, checking that it is a proper rotation.
    ///
    /// Every row must be unit length, mutually orthogonal, and the determinant must be
    /// +1. A matrix that fails those tests is a programming error in the caller, so it is
    /// rejected here rather than producing subtly wrong positions later.
    ///
    /// # Errors
    ///
    /// Returns an error naming the test that failed.
    pub fn from_rows(rows: [[f64; 3]; 3]) -> Result<Self> {
        let candidate = Self { rows };
        candidate.validate()?;
        Ok(candidate)
    }

    /// The matrix as row-major rows.
    #[must_use]
    pub const fn rows(&self) -> [[f64; 3]; 3] {
        self.rows
    }

    /// Row `index`, or [`None`] when out of range.
    #[must_use]
    pub fn row(&self, index: usize) -> Option<[f64; 3]> {
        self.rows.get(index).copied()
    }

    /// The transpose, which is the inverse of a proper rotation.
    #[must_use]
    pub fn transpose(self) -> Self {
        let r = self.rows;
        Self {
            rows: [
                [r[0][0], r[1][0], r[2][0]],
                [r[0][1], r[1][1], r[2][1]],
                [r[0][2], r[1][2], r[2][2]],
            ],
        }
    }

    /// The inverse rotation, mapping "to" back to "from".
    #[must_use]
    pub fn inverse(self) -> Self {
        self.transpose()
    }

    /// The determinant, computed by cofactor expansion.
    #[must_use]
    pub fn determinant(self) -> f64 {
        let r = self.rows;
        r[0][0] * (r[1][1] * r[2][2] - r[1][2] * r[2][1])
            - r[0][1] * (r[1][0] * r[2][2] - r[1][2] * r[2][0])
            + r[0][2] * (r[1][0] * r[2][1] - r[1][1] * r[2][0])
    }

    /// Check the orthonormality and handedness conditions.
    ///
    /// The tolerance is 1e-12, which is far tighter than any real frame construction
    /// needs and loose enough to accept a matrix that was assembled from trigonometric
    /// series.
    ///
    /// # Errors
    ///
    /// Returns an error naming the failed condition.
    pub fn validate(&self) -> Result<()> {
        const TOL: f64 = 1e-12;

        for (index, row) in self.rows.iter().enumerate() {
            if !row.iter().all(|v| v.is_finite()) {
                return Err(Error::invalid(
                    "rotation matrix",
                    format!("row {index} contains a non-finite value"),
                ));
            }
            let norm_sq = row[0] * row[0] + row[1] * row[1] + row[2] * row[2];
            if (norm_sq - 1.0).abs() > TOL {
                return Err(Error::invalid(
                    "rotation matrix",
                    format!("row {index} is not unit length (norm^2 = {norm_sq})"),
                ));
            }
        }

        for (a, b) in [(0_usize, 1_usize), (0, 2), (1, 2)] {
            let Some(ra) = self.rows.get(a) else {
                continue;
            };
            let Some(rb) = self.rows.get(b) else {
                continue;
            };
            let dot = ra[0] * rb[0] + ra[1] * rb[1] + ra[2] * rb[2];
            if dot.abs() > TOL {
                return Err(Error::invalid(
                    "rotation matrix",
                    format!("rows {a} and {b} are not orthogonal (dot = {dot})"),
                ));
            }
        }

        let det = self.determinant();
        if (det - 1.0).abs() > TOL {
            return Err(Error::invalid(
                "rotation matrix",
                format!("determinant is {det}, not +1 (a reflection, not a rotation)"),
            ));
        }

        Ok(())
    }

    /// A rotation about the x axis through `angle` radians, right-handed.
    ///
    /// This is an *active* rotation: it turns a vector, and a positive angle is
    /// counter-clockwise when looking from +x toward the origin. A frame conversion is the
    /// passive counterpart of the same operation, and is obtained with
    /// [`Self::transpose`] — see the type documentation.
    #[must_use]
    pub fn about_x(angle: f64) -> Self {
        let (s, c) = angle.sin_cos();
        Self {
            rows: [[1.0, 0.0, 0.0], [0.0, c, -s], [0.0, s, c]],
        }
    }

    /// A rotation about the y axis through `angle` radians, right-handed.
    #[must_use]
    pub fn about_y(angle: f64) -> Self {
        let (s, c) = angle.sin_cos();
        Self {
            rows: [[c, 0.0, s], [0.0, 1.0, 0.0], [-s, 0.0, c]],
        }
    }

    /// A rotation about the z axis through `angle` radians, right-handed.
    #[must_use]
    pub fn about_z(angle: f64) -> Self {
        let (s, c) = angle.sin_cos();
        Self {
            rows: [[c, -s, 0.0], [s, c, 0.0], [0.0, 0.0, 1.0]],
        }
    }

    /// Build a rotation from the three angles of the classical 3-1-3 Euler sequence.
    ///
    /// This sequence is what precession (zeta, z, theta) and Earth-fixed frame
    /// constructions use, so it has its own constructor instead of asking callers to
    /// multiply three `about_*` rotations in the right order every time.
    #[must_use]
    pub fn from_euler_313(first: f64, second: f64, third: f64) -> Self {
        Self::about_z(first) * Self::about_x(second) * Self::about_z(third)
    }

    /// Build a rotation from the three angles of the 3-2-1 yaw-pitch-roll sequence,
    /// the aerospace convention for attitude.
    #[must_use]
    pub fn from_euler_321(yaw: f64, pitch: f64, roll: f64) -> Self {
        Self::about_z(yaw) * Self::about_y(pitch) * Self::about_x(roll)
    }

    /// Apply the rotation to a vector.
    #[must_use]
    pub fn rotate(self, v: Vector3) -> Vector3 {
        let r = self.rows;
        Vector3::new(
            r[0][0] * v.x + r[0][1] * v.y + r[0][2] * v.z,
            r[1][0] * v.x + r[1][1] * v.y + r[1][2] * v.z,
            r[2][0] * v.x + r[2][1] * v.y + r[2][2] * v.z,
        )
    }
}

impl Mul for Rotation3 {
    type Output = Self;

    /// Compose two rotations: `a * b` applies `b` first, then `a`.
    ///
    /// This is the standard matrix-product order for active rotations, so a chain reads
    /// right to left: `Rotation3::about_z(turn) * Rotation3::about_x(tilt)` tilts first,
    /// then turns.
    fn mul(self, rhs: Self) -> Self {
        let mut rows = [[0.0_f64; 3]; 3];
        for (i, out_row) in rows.iter_mut().enumerate() {
            let Some(lhs_row) = self.rows.get(i) else {
                continue;
            };
            for (j, cell) in out_row.iter_mut().enumerate() {
                let mut sum = 0.0;
                for (k, lhs_value) in lhs_row.iter().enumerate() {
                    let Some(rhs_row) = rhs.rows.get(k) else {
                        continue;
                    };
                    let Some(rhs_value) = rhs_row.get(j) else {
                        continue;
                    };
                    sum += lhs_value * rhs_value;
                }
                *cell = sum;
            }
        }
        Self { rows }
    }
}

impl fmt::Display for Rotation3 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for row in self.rows {
            writeln!(f, "[{:>12.8} {:>12.8} {:>12.8}]", row[0], row[1], row[2])?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: Vector3, b: Vector3) -> bool {
        (a - b).norm() < 1e-12
    }

    #[test]
    fn identity_leaves_vectors_alone() {
        let v = Vector3::new(1.0, -2.0, 3.0);
        assert!(close(Rotation3::IDENTITY.rotate(v), v));
    }

    #[test]
    fn identity_is_valid() {
        assert!(Rotation3::IDENTITY.validate().is_ok());
        assert!((Rotation3::IDENTITY.determinant() - 1.0).abs() < 1e-15);
    }

    #[test]
    fn right_handed_rotation_about_z_turns_x_into_y() {
        let r = Rotation3::about_z(core::f64::consts::FRAC_PI_2);
        assert!(close(r.rotate(Vector3::X), Vector3::Y));
    }

    #[test]
    fn right_handed_rotation_about_x_turns_y_into_z() {
        let r = Rotation3::about_x(core::f64::consts::FRAC_PI_2);
        assert!(close(r.rotate(Vector3::Y), Vector3::Z));
    }

    #[test]
    fn right_handed_rotation_about_y_turns_z_into_x() {
        let r = Rotation3::about_y(core::f64::consts::FRAC_PI_2);
        assert!(close(r.rotate(Vector3::Z), Vector3::X));
    }

    #[test]
    fn axis_rotations_are_valid_rotations() {
        for angle in [0.0, 0.3, core::f64::consts::PI, -1.234] {
            assert!(Rotation3::about_x(angle).validate().is_ok());
            assert!(Rotation3::about_y(angle).validate().is_ok());
            assert!(Rotation3::about_z(angle).validate().is_ok());
        }
    }

    #[test]
    fn transpose_undoes_the_rotation() {
        let r = Rotation3::from_euler_313(0.3, -0.7, 1.1);
        let v = Vector3::new(2.0, -1.0, 0.5);
        assert!(close(r.inverse().rotate(r.rotate(v)), v));
    }

    #[test]
    fn composition_applies_the_right_operand_first() {
        // Rotating x by +90 about z gives y; then by -90 about z gives x back.
        let first = Rotation3::about_z(core::f64::consts::FRAC_PI_2);
        let second = Rotation3::about_z(-core::f64::consts::FRAC_PI_2);
        let composed = second * first;
        assert!(close(composed.rotate(Vector3::X), Vector3::X));
    }

    #[test]
    fn euler_313_is_a_valid_rotation() {
        let r = Rotation3::from_euler_313(0.1, 0.4, -2.2);
        assert!(r.validate().is_ok());
    }

    #[test]
    fn euler_321_reproduces_the_aerospace_order() {
        let (yaw, pitch, roll) = (0.2, -0.3, 0.4);
        let built = Rotation3::from_euler_321(yaw, pitch, roll);
        let manual = Rotation3::about_z(yaw) * Rotation3::about_y(pitch) * Rotation3::about_x(roll);
        assert_eq!(built, manual);
    }

    #[test]
    fn a_scaled_matrix_is_rejected() {
        let mut rows = Rotation3::IDENTITY.rows();
        rows[0] = [2.0, 0.0, 0.0];
        assert!(Rotation3::from_rows(rows).is_err());
    }

    #[test]
    fn a_reflection_is_rejected() {
        // Negating one row keeps the rows orthonormal but flips the determinant to -1.
        let mut rows = Rotation3::IDENTITY.rows();
        rows[0] = [-1.0, 0.0, 0.0];
        let result = Rotation3::from_rows(rows);
        assert!(result.is_err());
        if let Err(e) = result {
            assert!(e.to_string().contains("determinant"), "got: {e}");
        }
    }

    #[test]
    fn a_non_orthogonal_matrix_is_rejected() {
        let rows = [[0.6, 0.8, 0.0], [0.8, 0.6, 0.0], [0.0, 0.0, 1.0]];
        assert!(Rotation3::from_rows(rows).is_err());
    }

    #[test]
    fn a_non_finite_matrix_is_rejected() {
        let mut rows = Rotation3::IDENTITY.rows();
        rows[1] = [f64::NAN, 0.0, 1.0];
        assert!(Rotation3::from_rows(rows).is_err());
    }

    #[test]
    fn row_access_is_bounds_checked() {
        assert_eq!(Rotation3::IDENTITY.row(0), Some([1.0, 0.0, 0.0]));
        assert_eq!(Rotation3::IDENTITY.row(3), None);
    }
}
