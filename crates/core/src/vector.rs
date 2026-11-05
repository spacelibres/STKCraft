//! A small, dependency-free 3-vector in `f64`.
//!
//! Mission analysis needs double precision everywhere — a state vector in metres at
//! geostationary distance already spends most of an `f32`'s mantissa — and the
//! operations needed are few. Pulling in a linear-algebra crate for this would add a
//! large dependency for a handful of dot and cross products, so the vector lives here.
//!
//! The type is deliberately minimal: no generic scalar parameter, no SIMD. Rotation
//! lives in `stkcraft-frames`, which owns the matrices.

use core::fmt;
use core::ops::{Add, AddAssign, Div, Mul, Neg, Sub, SubAssign};

use serde::{Deserialize, Serialize};

/// A three-component vector of `f64`, conventionally `[x, y, z]`.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Vector3 {
    /// The x component.
    pub x: f64,
    /// The y component.
    pub y: f64,
    /// The z component.
    pub z: f64,
}

/// Alias kept for call sites that want to be explicit about precision.
pub type Vector3d = Vector3;

impl Vector3 {
    /// The zero vector.
    pub const ZERO: Self = Self::new(0.0, 0.0, 0.0);

    /// The vector along +x, a unit vector.
    pub const X: Self = Self::new(1.0, 0.0, 0.0);

    /// The vector along +y, a unit vector.
    pub const Y: Self = Self::new(0.0, 1.0, 0.0);

    /// The vector along +z, a unit vector.
    pub const Z: Self = Self::new(0.0, 0.0, 1.0);

    /// Build a vector from its components.
    #[must_use]
    pub const fn new(x: f64, y: f64, z: f64) -> Self {
        Self { x, y, z }
    }

    /// Build a vector from an array.
    #[must_use]
    pub const fn from_array(a: [f64; 3]) -> Self {
        Self::new(a[0], a[1], a[2])
    }

    /// The components as an array.
    #[must_use]
    pub const fn to_array(self) -> [f64; 3] {
        [self.x, self.y, self.z]
    }

    /// The dot product.
    #[must_use]
    pub fn dot(self, rhs: Self) -> f64 {
        self.x * rhs.x + self.y * rhs.y + self.z * rhs.z
    }

    /// The cross product, right-handed.
    #[must_use]
    pub fn cross(self, rhs: Self) -> Self {
        Self::new(
            self.y * rhs.z - self.z * rhs.y,
            self.z * rhs.x - self.x * rhs.z,
            self.x * rhs.y - self.y * rhs.x,
        )
    }

    /// The squared Euclidean norm.
    ///
    /// Prefer this to [`Self::norm`] when comparing magnitudes: it avoids a square root.
    #[must_use]
    pub fn norm_squared(self) -> f64 {
        self.dot(self)
    }

    /// The Euclidean norm.
    #[must_use]
    pub fn norm(self) -> f64 {
        self.norm_squared().sqrt()
    }

    /// The vector scaled to unit length.
    ///
    /// Returns [`None`] for a zero or non-finite vector rather than producing NaN, so
    /// callers must decide what a degenerate direction means.
    #[must_use]
    pub fn normalized(self) -> Option<Self> {
        let n = self.norm();
        if n > 0.0 && n.is_finite() {
            Some(self / n)
        } else {
            None
        }
    }

    /// The component-wise product of two vectors.
    #[must_use]
    pub fn hadamard(self, rhs: Self) -> Self {
        Self::new(self.x * rhs.x, self.y * rhs.y, self.z * rhs.z)
    }

    /// A copy with every component negated.
    #[must_use]
    pub fn negated(self) -> Self {
        Self::new(-self.x, -self.y, -self.z)
    }

    /// True when every component is finite.
    #[must_use]
    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite() && self.z.is_finite()
    }

    /// The Euclidean distance to another point.
    #[must_use]
    pub fn distance(self, rhs: Self) -> f64 {
        (self - rhs).norm()
    }
}

impl Add for Vector3 {
    type Output = Self;
    fn add(self, rhs: Self) -> Self {
        Self::new(self.x + rhs.x, self.y + rhs.y, self.z + rhs.z)
    }
}

impl Sub for Vector3 {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self {
        Self::new(self.x - rhs.x, self.y - rhs.y, self.z - rhs.z)
    }
}

impl AddAssign for Vector3 {
    fn add_assign(&mut self, rhs: Self) {
        *self = *self + rhs;
    }
}

impl SubAssign for Vector3 {
    fn sub_assign(&mut self, rhs: Self) {
        *self = *self - rhs;
    }
}

impl Mul<f64> for Vector3 {
    type Output = Self;
    fn mul(self, rhs: f64) -> Self {
        Self::new(self.x * rhs, self.y * rhs, self.z * rhs)
    }
}

impl Div<f64> for Vector3 {
    type Output = Self;
    fn div(self, rhs: f64) -> Self {
        Self::new(self.x / rhs, self.y / rhs, self.z / rhs)
    }
}

impl Neg for Vector3 {
    type Output = Self;
    fn neg(self) -> Self {
        self.negated()
    }
}

impl fmt::Display for Vector3 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{:.6}, {:.6}, {:.6}]", self.x, self.y, self.z)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: Vector3, b: Vector3) -> bool {
        (a - b).norm() < 1e-12
    }

    #[test]
    fn cross_product_is_right_handed() {
        assert!(close(Vector3::X.cross(Vector3::Y), Vector3::Z));
        assert!(close(Vector3::Y.cross(Vector3::X), -Vector3::Z));
    }

    #[test]
    fn parallel_vectors_have_zero_cross() {
        let a = Vector3::new(2.0, 4.0, 6.0);
        assert!(a.cross(a * 3.0).norm() < 1e-12);
    }

    #[test]
    fn normalize_gives_unit_length() {
        let v = Vector3::new(3.0, 4.0, 0.0).normalized();
        assert!(v.is_some());
        if let Some(u) = v {
            assert!((u.norm() - 1.0).abs() < 1e-15);
        }
    }

    #[test]
    fn normalizing_zero_is_none_not_nan() {
        // This is the property that keeps the "never crash, never NaN" contract.
        assert_eq!(Vector3::ZERO.normalized(), None);
    }

    #[test]
    fn normalizing_a_non_finite_vector_is_none() {
        assert_eq!(Vector3::new(f64::NAN, 0.0, 0.0).normalized(), None);
        assert_eq!(Vector3::new(f64::INFINITY, 0.0, 0.0).normalized(), None);
    }

    #[test]
    fn norm_squared_agrees_with_norm() {
        let v = Vector3::new(1.0, 2.0, 3.0);
        assert!((v.norm_squared() - v.norm().powi(2)).abs() < 1e-12);
    }

    #[test]
    fn distance_is_symmetric() {
        let a = Vector3::new(1.0, 2.0, 3.0);
        let b = Vector3::new(-4.0, 5.0, 0.5);
        assert!((a.distance(b) - b.distance(a)).abs() < 1e-12);
    }
}
