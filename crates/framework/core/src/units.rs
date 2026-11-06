//! Physical quantities as newtypes.
//!
//! A bare `f64` cannot say whether it holds radians or degrees, and a mission-analysis
//! codebase that mixes the two produces wrong answers that look plausible. These types
//! make the unit part of the type, and make every conversion a named method.
//!
//! Internally everything uses the SI base unit for its dimension: metres, seconds,
//! kilograms, radians. The `from_*`/`as_*` pairs are the only unit crossings.

use core::fmt;
use core::ops::{Add, AddAssign, Div, Mul, Neg, Sub, SubAssign};

use serde::{Deserialize, Serialize};

macro_rules! quantity {
    ($(#[$meta:meta])* $name:ident, $unit:literal) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(f64);

        impl $name {
            /// The zero value of this quantity.
            pub const ZERO: Self = Self(0.0);

            /// Wrap a raw value already expressed in the SI base unit.
            #[must_use]
            pub const fn from_si(value: f64) -> Self {
                Self(value)
            }

            /// The value in the SI base unit.
            #[must_use]
            pub const fn as_si(self) -> f64 {
                self.0
            }

            /// True when the value is neither infinite nor NaN.
            #[must_use]
            pub fn is_finite(self) -> bool {
                self.0.is_finite()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{} {}", self.0, $unit)
            }
        }

        impl Add for $name {
            type Output = Self;
            fn add(self, rhs: Self) -> Self {
                Self(self.0 + rhs.0)
            }
        }

        impl Sub for $name {
            type Output = Self;
            fn sub(self, rhs: Self) -> Self {
                Self(self.0 - rhs.0)
            }
        }

        impl AddAssign for $name {
            fn add_assign(&mut self, rhs: Self) {
                self.0 += rhs.0;
            }
        }

        impl SubAssign for $name {
            fn sub_assign(&mut self, rhs: Self) {
                self.0 -= rhs.0;
            }
        }

        impl Neg for $name {
            type Output = Self;
            fn neg(self) -> Self {
                Self(-self.0)
            }
        }
    };
}

quantity!(
    /// A distance, held in metres.
    Distance,
    "m"
);
quantity!(
    /// A velocity, held in metres per second.
    Velocity,
    "m/s"
);
quantity!(
    /// A mass, held in kilograms.
    Mass,
    "kg"
);

impl Distance {
    /// One kilometre.
    pub const KM: f64 = 1_000.0;

    /// Build from kilometres.
    #[must_use]
    pub const fn from_km(value: f64) -> Self {
        Self(value * Self::KM)
    }

    /// The distance in kilometres.
    #[must_use]
    pub const fn as_km(self) -> f64 {
        self.0 / Self::KM
    }
}

impl Velocity {
    /// Build from kilometres per second.
    #[must_use]
    pub const fn from_km_s(value: f64) -> Self {
        Self(value * Distance::KM)
    }

    /// The velocity in kilometres per second.
    #[must_use]
    pub const fn as_km_s(self) -> f64 {
        self.0 / Distance::KM
    }
}

impl Mul<f64> for Distance {
    type Output = Self;
    fn mul(self, rhs: f64) -> Self {
        Self(self.0 * rhs)
    }
}

impl Div<f64> for Distance {
    type Output = Self;
    fn div(self, rhs: f64) -> Self {
        Self(self.0 / rhs)
    }
}

impl Mul<f64> for Velocity {
    type Output = Self;
    fn mul(self, rhs: f64) -> Self {
        Self(self.0 * rhs)
    }
}

/// An angle, held in radians.
///
/// Construct with [`Angle::from_rad`], [`Angle::from_deg`] or [`Angle::from_arcsec`];
/// read back with the matching `as_*` method. There is deliberately no `From<f64>`.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Angle(f64);

impl Angle {
    /// The zero angle.
    pub const ZERO: Self = Self(0.0);

    /// A full turn, 2π radians.
    pub const TAU_RAD: f64 = core::f64::consts::TAU;

    /// Build from radians.
    #[must_use]
    pub const fn from_rad(value: f64) -> Self {
        Self(value)
    }

    /// Build from degrees.
    #[must_use]
    pub fn from_deg(value: f64) -> Self {
        Self(value.to_radians())
    }

    /// Build from arcseconds, the unit most star catalogues and sensor geometries use.
    #[must_use]
    pub fn from_arcsec(value: f64) -> Self {
        Self(value * core::f64::consts::PI / (180.0 * 3_600.0))
    }

    /// Build from degrees, minutes and seconds.
    #[must_use]
    pub fn from_dms(deg: i32, min: u8, sec: f64) -> Self {
        let sign = if deg < 0 { -1.0 } else { 1.0 };
        let magnitude = f64::from(deg).abs() + f64::from(min) / 60.0 + sec / 3_600.0;
        Self((sign * magnitude).to_radians())
    }

    /// The angle in radians.
    #[must_use]
    pub const fn as_rad(self) -> f64 {
        self.0
    }

    /// The angle in degrees.
    #[must_use]
    pub fn as_deg(self) -> f64 {
        self.0.to_degrees()
    }

    /// The angle in arcseconds.
    #[must_use]
    pub fn as_arcsec(self) -> f64 {
        self.0 * 180.0 * 3_600.0 / core::f64::consts::PI
    }

    /// Split into signed degrees, minutes and seconds.
    #[must_use]
    pub fn to_dms(self) -> (i32, u8, f64) {
        let total = self.as_deg();
        let sign = if total < 0.0 { -1 } else { 1 };
        let mut rest = total.abs();
        let mut deg = rest.floor();
        rest = (rest - deg) * 60.0;
        let mut min = rest.floor();
        let mut sec = (rest - min) * 60.0;

        // Carry in order, seconds before minutes: 59'59.99999999" must become 60'00"
        // and then 1°00'00", never 0°60'00".
        if sec >= 59.999_999_5 {
            sec = 0.0;
            min += 1.0;
        }
        if min >= 60.0 {
            min = 0.0;
            deg += 1.0;
        }

        let min = min.clamp(0.0, 59.0) as u8;
        (sign * deg as i32, min, sec)
    }

    /// True when the value is neither infinite nor NaN.
    #[must_use]
    pub fn is_finite(self) -> bool {
        self.0.is_finite()
    }

    /// Wrap into `[0, 2π)`.
    #[must_use]
    pub fn wrapped(self) -> Self {
        let tau = Self::TAU_RAD;
        let mut v = self.0 % tau;
        if v < 0.0 {
            v += tau;
        }
        // `%` can return exactly `tau` for tiny negatives; `rem_euclid` avoids that.
        Self(v.rem_euclid(tau))
    }
}

impl Add for Angle {
    type Output = Self;
    fn add(self, rhs: Self) -> Self {
        Self(self.0 + rhs.0)
    }
}

impl Sub for Angle {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self {
        Self(self.0 - rhs.0)
    }
}

impl Neg for Angle {
    type Output = Self;
    fn neg(self) -> Self {
        Self(-self.0)
    }
}

impl fmt::Display for Angle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (d, m, s) = self.to_dms();
        write!(f, "{d}°{m:02}'{s:05.2}\"")
    }
}

/// A duration of time, held in seconds.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Duration(f64);

impl Duration {
    /// The zero duration.
    pub const ZERO: Self = Self(0.0);

    /// Seconds in one day, on the SI scale.
    pub const SECONDS_PER_DAY: f64 = 86_400.0;

    /// Build from seconds.
    #[must_use]
    pub const fn from_seconds(value: f64) -> Self {
        Self(value)
    }

    /// Build from days.
    #[must_use]
    pub const fn from_days(value: f64) -> Self {
        Self(value * Self::SECONDS_PER_DAY)
    }

    /// The duration in seconds.
    #[must_use]
    pub const fn as_seconds(self) -> f64 {
        self.0
    }

    /// The duration in days.
    #[must_use]
    pub const fn as_days(self) -> f64 {
        self.0 / Self::SECONDS_PER_DAY
    }
}

impl Add for Duration {
    type Output = Self;
    fn add(self, rhs: Self) -> Self {
        Self(self.0 + rhs.0)
    }
}

impl Sub for Duration {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self {
        Self(self.0 - rhs.0)
    }
}

impl Neg for Duration {
    type Output = Self;
    fn neg(self) -> Self {
        Self(-self.0)
    }
}

impl fmt::Display for Duration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:.3} s", self.0)
    }
}

/// An offset from a scenario's epoch, held in seconds.
///
/// Kept distinct from [`Duration`] so that a function taking an epoch offset cannot be
/// handed an interval by mistake.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EpochOffset(f64);

impl EpochOffset {
    /// The epoch itself.
    pub const ZERO: Self = Self(0.0);

    /// Build from seconds after the epoch.
    #[must_use]
    pub const fn from_seconds(value: f64) -> Self {
        Self(value)
    }

    /// The offset in seconds.
    #[must_use]
    pub const fn as_seconds(self) -> f64 {
        self.0
    }
}

impl Add<Duration> for EpochOffset {
    type Output = Self;
    fn add(self, rhs: Duration) -> Self {
        Self(self.0 + rhs.as_seconds())
    }
}

impl Sub for EpochOffset {
    type Output = Duration;
    fn sub(self, rhs: Self) -> Duration {
        Duration::from_seconds(self.0 - rhs.0)
    }
}

impl fmt::Display for EpochOffset {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "T{:+} s", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distance_round_trips_through_kilometres() {
        let d = Distance::from_km(7_000.0);
        assert!((d.as_si() - 7_000_000.0).abs() < 1e-9);
        assert!((d.as_km() - 7_000.0).abs() < 1e-9);
    }

    #[test]
    fn angle_round_trips_through_degrees() {
        let a = Angle::from_deg(45.0);
        assert!((a.as_deg() - 45.0).abs() < 1e-12);
        assert!((a.as_rad() - core::f64::consts::FRAC_PI_4).abs() < 1e-12);
    }

    #[test]
    fn one_degree_is_3600_arcseconds() {
        let a = Angle::from_deg(1.0);
        assert!((a.as_arcsec() - 3_600.0).abs() < 1e-6);
    }

    #[test]
    fn dms_round_trips_for_a_negative_angle() {
        let a = Angle::from_dms(-12, 34, 56.0);
        let (d, m, s) = a.to_dms();
        assert_eq!((d, m), (-12, 34));
        assert!((s - 56.0).abs() < 1e-3, "seconds were {s}");
    }

    #[test]
    fn dms_carry_does_not_produce_sixty_minutes() {
        // 0° 59' 59.99999999" rounds up to 1° 00' 00".
        let a = Angle::from_dms(0, 59, 59.999_999_99);
        let (d, m, s) = a.to_dms();
        assert_eq!((d, m), (1, 0), "got {d}d {m}m {s}s");
        assert!(s < 1e-3, "seconds were {s}");
    }

    #[test]
    fn wrapped_maps_into_half_open_turn() {
        assert!((Angle::from_deg(370.0).wrapped().as_deg() - 10.0).abs() < 1e-9);
        assert!((Angle::from_deg(-10.0).wrapped().as_deg() - 350.0).abs() < 1e-9);
    }

    #[test]
    fn epoch_offset_subtraction_yields_a_duration() {
        let a = EpochOffset::from_seconds(100.0);
        let b = EpochOffset::from_seconds(40.0);
        assert!((a - b).as_seconds() - 60.0 < 1e-12);
    }

    #[test]
    fn duration_converts_days_and_seconds() {
        let d = Duration::from_days(1.0);
        assert!((d.as_seconds() - 86_400.0).abs() < 1e-9);
        assert!((d.as_days() - 1.0).abs() < 1e-12);
    }
}
