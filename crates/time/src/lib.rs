//! Time scales and epoch arithmetic for `STKCraft`.
//!
//! Spacecraft analysis cannot use a single clock: orbit integration runs on a uniform
//! scale (TAI, TT or TDB), while user-facing times and ephemeris files are in UTC, which
//! has leap seconds. Mixing the two silently is the classic way to get a timing error
//! that only shows up as a bad conjunction warning.
//!
//! This crate makes the scale part of the type ([`TimeScale`]) and keeps every conversion
//! explicit. The constants and the 1972-onward leap-second table follow the IAU SOFA /
//! ERFA reference implementation, whose values are themselves taken from the IERS
//! Conventions.
//!
//! Calendar formatting and civil-date parsing go through `chrono`, per the suite standard;
//! this crate owns only the scale offsets and the leap table.

#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]
#![forbid(unsafe_code)]

pub mod epoch;
pub mod leaps;
pub mod scales;

pub use epoch::{Epoch, JulianDate};
pub use leaps::{LeapSecondTable, UTC_INTRODUCED, tai_minus_utc};
pub use scales::{TimeScale, TimeScaleConversion};

/// Seconds per day, on the SI scale.
pub const SECONDS_PER_DAY: f64 = 86_400.0;

/// Days per Julian century.
pub const DAYS_PER_JULIAN_CENTURY: f64 = 36_525.0;

/// The Julian Date of the J2000.0 reference epoch, 2000-01-01T12:00:00 TT.
pub const JD_J2000: f64 = 2_451_545.0;

/// The Julian Date of Modified Julian Date zero.
pub const JD_MJD_ZERO: f64 = 2_400_000.5;

/// TT minus TAI, in seconds (IAU 1991, a defining constant).
///
/// Source: IERS Conventions; ERFA `ERFA_TTMTAI`.
pub const TT_MINUS_TAI: f64 = 32.184;

/// `L_G = 1 - d(TT)/d(TCG)`, from the IAU 2000 definition of Geocentric Coordinate Time.
pub const LG: f64 = 6.969_290_134e-10;

/// `L_B = 1 - d(TDB)/d(TCB)`, from the IAU 2006 definition of Barycentric Dynamical Time.
pub const LB: f64 = 1.550_519_768e-8;

/// `TDB - TCB` at TAI 1977-01-01T00:00:00, in seconds.
pub const TDB0: f64 = -6.55e-5;

/// The speed of light in vacuum, in metres per second (an SI defining constant).
pub const SPEED_OF_LIGHT: f64 = 299_792_458.0;

/// The astronomical unit in metres (IAU 2012 resolution B2).
pub const ASTRONOMICAL_UNIT: f64 = 149_597_870_700.0;

/// The light time for one astronomical unit, in seconds.
pub const LIGHT_TIME_PER_AU: f64 = ASTRONOMICAL_UNIT / SPEED_OF_LIGHT;
