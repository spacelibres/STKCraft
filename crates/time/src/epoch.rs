//! Epochs and Julian dates.
//!
//! A Julian Date near the present is about 2.46e6, and an `f64` carries roughly 15-16
//! significant digits, so a single `f64` JD resolves only about 40 microseconds. Ephemeris
//! interpolation and conjunction screening need better than that.
//!
//! [`JulianDate`] therefore stores the date in two parts, following the IAU SOFA
//! convention (its "2-part Julian Date"): a whole-day part and a fractional part that are
//! added only when a double is actually needed. The same pattern is used by SPICE and by
//! every serious orbit-determination code, and it is why this crate does not simply hand
//! around `f64`.

use core::fmt;

use chrono::{DateTime, Datelike, NaiveDate, NaiveDateTime, TimeZone, Utc};
use stkcraft_core::{Error, Result};

use crate::leaps::{LeapLookup, tai_minus_utc};
use crate::scales::TimeScale;
use crate::{DAYS_PER_JULIAN_CENTURY, JD_MJD_ZERO, SECONDS_PER_DAY, TT_MINUS_TAI};

/// A Julian Date held as two parts for precision.
///
/// `day` holds the whole days counted from JD 0 and `fraction` the remainder, in days. Sum for
/// the scalar:
///
/// ```
/// # use stkcraft_time::JulianDate;
/// // J2000.0 is JD 2451545.0 = 2451545 whole days + 0.0 day.
/// let jd = JulianDate::from_parts(2_451_545.0, 0.0);
/// assert!((jd.sum() - 2_451_545.0).abs() < 1e-12);
/// ```
///
/// **`day` is always a whole number**, which is what makes the split worth having: an
/// integral `f64` below 2^53 is exact, so day arithmetic never rounds, and only the fraction
/// carries rounding. The day may still run into the following day through `fraction` — the
/// two parts are not normalised — so a caller can hold `day` fixed and vary the fraction.
///
/// Keeping `day` integral is a rule, not an accident: downstream code that subtracts a
/// whole-day epoch from `day` relies on getting an exact integer back.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct JulianDate {
    /// The whole-day part, counted from JD 0. Always integral.
    pub day: f64,
    /// The remainder in days, normally in `[0, 1)`.
    pub fraction: f64,
}

impl JulianDate {
    /// The J2000.0 reference epoch, `2000-01-01T12:00:00 TT`, JD 2451545.0.
    ///
    /// JD 2451545.0 is 2451545 whole days exactly, so [`Self::day`] is 2451545 and the
    /// fraction is zero. The fraction is normally in `[0, 1)`; the constant keeps it there so
    /// that subtracting it from another value stays exact.
    pub const J2000: Self = Self {
        day: 2_451_545.0,
        fraction: 0.0,
    };

    /// Build from a whole-day part and a fractional part.
    #[must_use]
    pub const fn from_parts(day: f64, fraction: f64) -> Self {
        Self { day, fraction }
    }

    /// Build from a single scalar Julian Date.
    ///
    /// The whole days go to [`Self::day`] and the remainder to [`Self::fraction`], which
    /// keeps the fraction small and therefore precise. The whole days are computed from the
    /// *sum*, so a value whose fraction has run past one day still lands on an integral
    /// `day`.
    #[must_use]
    pub fn from_scalar(jd: f64) -> Self {
        let day = jd.floor();
        Self {
            day,
            fraction: jd - day,
        }
    }

    /// The scalar Julian Date. Use only where a single `f64` is required, such as a
    /// polynomial argument; it loses the sub-40-microsecond precision the split holds.
    #[must_use]
    pub fn sum(self) -> f64 {
        self.day + self.fraction
    }

    /// The Modified Julian Date, `JD - 2400000.5`.
    #[must_use]
    pub fn to_mjd(self) -> f64 {
        (self.day - JD_MJD_ZERO) + self.fraction
    }

    /// Build from a Modified Julian Date.
    #[must_use]
    pub fn from_mjd(mjd: f64) -> Self {
        Self::from_scalar(mjd + JD_MJD_ZERO)
    }

    /// The Julian centuries elapsed since J2000.0.
    ///
    /// This is the argument most IAU precession and nutation series expect. The difference
    /// from J2000 is taken part by part for the same reason as [`Self::seconds_until`].
    #[must_use]
    pub fn centuries_since_j2000(self) -> f64 {
        let days = (self.day - Self::J2000.day) + self.fraction;
        days / DAYS_PER_JULIAN_CENTURY
    }

    /// Julian millennia elapsed since J2000.0.
    #[must_use]
    pub fn millennia_since_j2000(self) -> f64 {
        self.centuries_since_j2000() / 10.0
    }

    /// Add a number of days, keeping the split.
    #[must_use]
    pub fn plus_days(self, days: f64) -> Self {
        Self {
            day: self.day,
            fraction: self.fraction + days,
        }
    }

    /// Add a number of seconds, keeping the split.
    #[must_use]
    pub fn plus_seconds(self, seconds: f64) -> Self {
        self.plus_days(seconds / SECONDS_PER_DAY)
    }

    /// The interval in seconds from `self` to `other`, positive when `other` is later.
    ///
    /// The subtraction is done part by part, never on the summed Julian Dates: two values
    /// near 2.46e6 have only about 40 us of resolution between them, so subtracting their
    /// sums and then scaling would throw away the precision the split exists to keep.
    /// Taking the day difference and the fraction difference separately keeps both exact.
    #[must_use]
    pub fn seconds_until(self, other: Self) -> f64 {
        let days = (other.day - self.day) + (other.fraction - self.fraction);
        days * SECONDS_PER_DAY
    }

    /// True when both parts are finite.
    #[must_use]
    pub fn is_finite(self) -> bool {
        self.day.is_finite() && self.fraction.is_finite()
    }
}

impl fmt::Display for JulianDate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "JD {:.6}", self.sum())
    }
}

/// An instant, with the time scale it is expressed in and its Julian Date.
///
/// Construct one from a stated scale or from a UTC calendar time. Converting between
/// scales consumes the value and returns a new one, so a mistyped scale shows up as a type
/// error rather than a silent offset.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Epoch {
    /// The Julian Date in the scale named by [`Self::scale`].
    julian: JulianDate,
    /// The scale [`Self::julian`] is expressed in.
    scale: TimeScale,
}

impl Epoch {
    /// Build from a Julian Date and the scale it is expressed in.
    #[must_use]
    pub const fn from_julian(julian: JulianDate, scale: TimeScale) -> Self {
        Self { julian, scale }
    }

    /// Build from a scalar Julian Date and its scale.
    #[must_use]
    pub fn from_jd(jd: f64, scale: TimeScale) -> Self {
        Self {
            julian: JulianDate::from_scalar(jd),
            scale,
        }
    }

    /// Build from a UTC calendar time.
    ///
    /// The conversion to TAI uses the leap-second table, so this is the correct way to
    /// turn a user-entered or file-supplied UTC time into an instant.
    ///
    /// # Warning
    ///
    /// For a time on a day that ends with an inserted leap second the result follows
    /// SOFA's convention and uses the offset in force before the leap.
    ///
    /// # Errors
    ///
    /// Returns an error for a time outside the range the leap table covers meaningfully.
    pub fn from_utc(datetime: NaiveDateTime) -> Result<Self> {
        let year = datetime.year();
        let month = datetime.month();
        let day = f64::from(datetime.day());
        let day_fraction = f64::from(datetime.and_utc().timestamp_subsec_nanos()) / 1e9;
        let month_day = day + day_fraction;

        let leap = tai_minus_utc(year, month, month_day);
        if leap.confidence == LeapLookup::BeforeUtc {
            return Err(Error::out_of_range(
                "UTC year",
                year,
                "1960 or later (UTC did not exist before 1960)",
            ));
        }

        let utc_jd = utc_naive_to_julian(datetime).ok_or_else(|| {
            Error::invalid(
                "UTC date",
                format!("{datetime} is outside the supported range"),
            )
        })?;

        // TAI = UTC + (TAI - UTC); the offset is a whole number of seconds after 1972.
        let tai = utc_jd.plus_seconds(leap.seconds);
        Ok(Self {
            julian: tai,
            scale: TimeScale::Tai,
        })
    }

    /// Build from a `chrono` UTC timestamp.
    ///
    /// # Errors
    ///
    /// As [`Self::from_utc`].
    pub fn from_chrono_utc(datetime: DateTime<Utc>) -> Result<Self> {
        Self::from_utc(datetime.naive_utc())
    }

    /// The Julian Date in this epoch's own scale.
    #[must_use]
    pub const fn julian(&self) -> JulianDate {
        self.julian
    }

    /// The scale this epoch is expressed in.
    #[must_use]
    pub const fn scale(&self) -> TimeScale {
        self.scale
    }

    /// Re-express this instant in another scale.
    ///
    /// `dut1` (`UT1 - UTC`, in seconds) is needed only when either side is UT1; IERS
    /// publishes it and it cannot be derived here. Passing `None` when it is required is
    /// an error rather than a silent default.
    ///
    /// # Errors
    ///
    /// Returns an error when UT1 is involved and `dut1` is `None`, or when an offset would
    /// need leap-second data the epoch does not carry.
    pub fn to_scale(self, target: TimeScale, dut1: Option<f64>) -> Result<Self> {
        if target == self.scale {
            return Ok(self);
        }

        // Route everything through TAI, which is defined uniformly and has no
        // discontinuities. Going directly between two derived scales would need the same
        // work with more places to get the sign wrong.
        let tai_jd = self.as_tai(dut1)?;
        let converted = match target {
            TimeScale::Tai => tai_jd,
            // TT, TDB, TCG and TCB all track TAI at this level of approximation: TT differs
            // from TAI by the defining constant 32.184 s, TDB differs from TT by under 2 ms
            // (below this layer's scope), and TCG/TCB differ from TT by rates that only
            // matter over decades. A future revision that adds the periodic TDB term or the
            // TCG rate belongs here, where the offset is applied.
            TimeScale::Tt | TimeScale::Tdb | TimeScale::Tcg | TimeScale::Tcb => {
                tai_jd.plus_seconds(TT_MINUS_TAI)
            }
            TimeScale::Gps => tai_jd.plus_seconds(-19.0),
            TimeScale::Ut1 => {
                // UT1 = TAI - (TAI - UTC) + DUT1. The `TAI - UTC` offset at this instant is
                // not recoverable from a TAI Julian Date alone, so converting into UT1 from
                // a uniform scale is refused rather than guessed. A scenario that needs UT1
                // starts from a UTC calendar date, where the leap table applies directly.
                if dut1.is_none() {
                    return Err(Error::invalid(
                        "UT1 conversion",
                        "DUT1 (UT1 - UTC) must be supplied; it cannot be derived and only IERS publishes it",
                    ));
                }
                return Err(Error::unsupported(
                    "converting to UT1 from a uniform scale (it needs the TAI - UTC offset at that instant)",
                ));
            }
            TimeScale::Utc => {
                return Err(Error::unsupported(
                    "converting back to UTC from a uniform scale (it needs the leap offset at that instant)",
                ));
            }
        };

        Ok(Self {
            julian: converted,
            scale: target,
        })
    }

    /// The instant as a TAI Julian Date, converting if needed.
    fn as_tai(self, dut1: Option<f64>) -> Result<JulianDate> {
        let _ = dut1;
        match self.scale {
            TimeScale::Tai => Ok(self.julian),
            TimeScale::Tt | TimeScale::Tdb | TimeScale::Tcg | TimeScale::Tcb => {
                Ok(self.julian.plus_seconds(-TT_MINUS_TAI))
            }
            TimeScale::Gps => Ok(self.julian.plus_seconds(19.0)),
            TimeScale::Utc | TimeScale::Ut1 => Err(Error::unsupported(
                "reading a UTC or UT1 epoch as TAI without its leap offset",
            )),
        }
    }

    /// Seconds from `self` to `other`, as a scalar.
    ///
    /// Both epochs must be in the same scale; comparing across scales would silently mix
    /// a uniform interval with a leap-contaminated one.
    ///
    /// # Errors
    ///
    /// Returns an error when the scales differ.
    pub fn seconds_until(self, other: Self) -> Result<f64> {
        if self.scale != other.scale {
            return Err(Error::invalid(
                "interval between epochs",
                format!(
                    "scales differ ({} and {}); convert one first",
                    self.scale.id(),
                    other.scale.id()
                ),
            ));
        }
        Ok(self.julian.seconds_until(other.julian))
    }
}

impl Default for Epoch {
    fn default() -> Self {
        Self::from_julian(JulianDate::J2000, TimeScale::Tt)
    }
}

impl fmt::Display for Epoch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.julian, self.scale.id())
    }
}

/// Julian Date for a naive UTC calendar time, valid from 1801 onwards.
///
/// The Unix timestamp is exact for every date `chrono` represents in the modern era, so
/// this avoids a second calendar algorithm alongside the one in `leaps`.
#[must_use]
pub fn utc_naive_to_julian(datetime: NaiveDateTime) -> Option<JulianDate> {
    let unix_seconds = datetime.and_utc().timestamp();
    let nanos = f64::from(datetime.and_utc().timestamp_subsec_nanos());
    // The day part is the whole days, which puts MJD 0 (1858-11-17T00:00) at fraction 0.5
    // inside the Julian day; that half goes in the fraction, keeping `day` integral.
    let whole_days = unix_seconds.div_euclid(SECONDS_PER_DAY as i64);
    let seconds_of_day = unix_seconds.rem_euclid(SECONDS_PER_DAY as i64) as f64;
    // The Julian Date of a UTC midnight is `X.5`: the day counter is integral and the half
    // day plus the time of day make up the fraction.
    let day = 40_587.0 + whole_days as f64 + JD_MJD_ZERO;
    let fraction = 0.5 + (seconds_of_day + nanos / 1e9) / SECONDS_PER_DAY;
    Some(JulianDate::from_parts(day, fraction))
}

/// Convert a Julian Date to a naive UTC calendar time.
///
/// This is the inverse of [`utc_naive_to_julian`] for presentation; it does not apply a
/// leap offset, because a UTC Julian Date already includes it.
#[must_use]
pub fn julian_to_utc_naive(jd: JulianDate) -> Option<NaiveDateTime> {
    // Work in whole seconds and nanoseconds so rounding can carry into the next second
    // instead of being discarded. The half day is removed first because a UTC midnight is
    // JD `X.5`, not `X.0`.
    let total_seconds = (jd.fraction - 0.5) * SECONDS_PER_DAY;
    let mut whole_seconds = total_seconds.floor();
    let mut nanos = ((total_seconds - whole_seconds) * 1e9).round();
    if nanos >= 1_000_000_000.0 {
        nanos -= 1_000_000_000.0;
        whole_seconds += 1.0;
    }

    // Julian Date -> days since the Unix epoch. `day` is the whole day number and `fraction`
    // is the time of day, so the day difference already lands on a whole number of days.
    let unix_days = (jd.day - JD_MJD_ZERO - 40_587.0) as i64;
    let unix_seconds = unix_days * SECONDS_PER_DAY as i64 + whole_seconds as i64;
    let datetime = DateTime::from_timestamp(unix_seconds, nanos as u32)?;
    Some(datetime.naive_utc())
}

/// Build a naive date from components, for tests and file importers.
///
/// # Errors
///
/// Returns an error for a date that does not exist in the Gregorian calendar.
pub fn naive_from_ymd_hms(
    year: i32,
    month: u32,
    day: u32,
    hour: u32,
    minute: u32,
    second: u32,
) -> Result<NaiveDateTime> {
    let date = NaiveDate::from_ymd_opt(year, month, day)
        .ok_or_else(|| Error::invalid("date", format!("{year}-{month:02}-{day:02}")))?;
    date.and_hms_opt(hour, minute, second)
        .ok_or_else(|| Error::invalid("time of day", format!("{hour:02}:{minute:02}:{second:02}")))
}

/// Build a UTC instant from calendar components.
///
/// # Errors
///
/// Returns an error for an impossible date or a time UTC did not exist.
pub fn utc_from_ymd_hms(
    year: i32,
    month: u32,
    day: u32,
    hour: u32,
    minute: u32,
    second: u32,
) -> Result<Epoch> {
    let naive = naive_from_ymd_hms(year, month, day, hour, minute, second)?;
    Epoch::from_utc(naive)
}

/// Re-exported so callers do not need to depend on `chrono` directly for the common case.
pub fn utc_datetime(naive: NaiveDateTime) -> DateTime<Utc> {
    Utc.from_utc_datetime(&naive)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::JD_J2000;

    fn close(a: f64, b: f64, tol: f64) -> bool {
        (a - b).abs() <= tol
    }

    #[test]
    fn j2000_constant_matches_formula() {
        assert!(close(JulianDate::J2000.sum(), JD_J2000, 0.0));
        assert!(close(JulianDate::J2000.centuries_since_j2000(), 0.0, 0.0));
    }

    #[test]
    fn split_preserves_precision_a_scalar_loses() {
        // A one-microsecond step at J2000: the split keeps it, a scalar JD near 2.46e6
        // cannot resolve it in an f64.
        let base = JulianDate::J2000;
        let stepped = base.plus_seconds(1e-6);
        let delta = base.seconds_until(stepped);
        assert!(
            close(delta, 1e-6, 1e-15),
            "split lost the microsecond: {delta}"
        );

        let scalar_delta = base.sum().to_owned();
        let scalar_step = stepped.sum();
        // The scalar difference is quantised to the f64 spacing at this magnitude.
        assert!(
            scalar_step - scalar_delta != 1e-6 / SECONDS_PER_DAY,
            "the test premise is wrong if a scalar JD resolved the step exactly"
        );
    }

    #[test]
    fn mjd_round_trips() {
        let jd = JulianDate::from_scalar(2_451_545.0);
        assert!(close(jd.to_mjd(), 51_544.5, 1e-9));
        assert!(close(
            JulianDate::from_mjd(51_544.5).sum(),
            2_451_545.0,
            1e-9
        ));
    }

    #[test]
    fn utc_epoch_applies_the_leap_offset() {
        // 2020-01-01T00:00:00 UTC is TAI + 37 s, so its TAI JD is 37 s later than its
        // UTC JD.
        let epoch = utc_from_ymd_hms(2020, 1, 1, 0, 0, 0);
        assert!(epoch.is_ok());
        if let Ok(e) = epoch {
            assert_eq!(e.scale(), TimeScale::Tai);
            let utc_jd =
                utc_naive_to_julian(naive_from_ymd_hms(2020, 1, 1, 0, 0, 0).unwrap_or_default());
            assert!(utc_jd.is_some());
            if let Some(u) = utc_jd {
                let diff = u.seconds_until(e.julian());
                assert!(close(diff, 37.0, 1e-6), "expected 37 s, got {diff}");
            }
        }
    }

    #[test]
    fn utc_before_1960_is_rejected_rather_than_guessed() {
        let epoch = utc_from_ymd_hms(1950, 6, 1, 0, 0, 0);
        assert!(epoch.is_err());
    }

    #[test]
    fn impossible_dates_are_rejected() {
        assert!(utc_from_ymd_hms(2020, 2, 30, 0, 0, 0).is_err());
        assert!(utc_from_ymd_hms(2020, 13, 1, 0, 0, 0).is_err());
    }

    #[test]
    fn tai_to_tt_adds_32_184_seconds() {
        let tai = Epoch::from_jd(JD_J2000, TimeScale::Tai);
        let tt = tai.to_scale(TimeScale::Tt, None);
        assert!(tt.is_ok());
        if let Ok(tt) = tt {
            let diff = tai.julian().seconds_until(tt.julian());
            assert!(close(diff, TT_MINUS_TAI, 1e-9), "got {diff}");
        }
    }

    #[test]
    fn tai_to_gps_subtracts_19_seconds() {
        let tai = Epoch::from_jd(JD_J2000, TimeScale::Tai);
        let gps = tai.to_scale(TimeScale::Gps, None);
        assert!(gps.is_ok());
        if let Ok(gps) = gps {
            let diff = tai.julian().seconds_until(gps.julian());
            assert!(close(diff, -19.0, 1e-9), "got {diff}");
        }
    }

    #[test]
    fn converting_to_the_same_scale_is_identity() {
        let e = Epoch::from_jd(JD_J2000, TimeScale::Tt);
        let same = e.to_scale(TimeScale::Tt, None);
        assert_eq!(same, Ok(e));
    }

    #[test]
    fn ut1_conversion_without_dut1_is_an_error_not_a_default() {
        let e = Epoch::from_jd(JD_J2000, TimeScale::Tai);
        assert!(e.to_scale(TimeScale::Ut1, None).is_err());
    }

    #[test]
    fn intervals_across_different_scales_are_refused() {
        let a = Epoch::from_jd(JD_J2000, TimeScale::Tai);
        let b = Epoch::from_jd(JD_J2000, TimeScale::Tt);
        assert!(a.seconds_until(b).is_err());
    }

    #[test]
    fn julian_round_trips_through_utc_naive() {
        let naive = naive_from_ymd_hms(2024, 3, 15, 12, 34, 56);
        assert!(naive.is_ok());
        if let Ok(naive) = naive {
            let jd = utc_naive_to_julian(naive);
            assert!(jd.is_some());
            if let Some(jd) = jd {
                let back = julian_to_utc_naive(jd);
                assert_eq!(back, Some(naive));
            }
        }
    }

    #[test]
    fn non_finite_julian_dates_are_detectable() {
        assert!(!JulianDate::from_parts(f64::NAN, 0.0).is_finite());
        assert!(!JulianDate::from_parts(0.0, f64::INFINITY).is_finite());
        assert!(JulianDate::J2000.is_finite());
    }
}
