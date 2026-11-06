//! The `TAI - UTC` leap-second table.
//!
//! UTC is steered to stay within 0.9 s of UT1 by inserting (or in principle removing)
//! whole seconds. The accumulated offset `TAI - UTC` is called `Delta(AT)`, and its value
//! is what every UTC <-> TAI conversion needs.
//!
//! # Data provenance
//!
//! The table below is the one in the IAU SOFA / ERFA reference implementation
//! (`eraDat`, revision 2023-01-17), which in turn comes from the IERS
//! `tai-utc.dat` file. Values from 1972 onward are the announced leap seconds; the
//! 1960-1971 entries are the earlier "rubber second" UTC where `TAI - UTC` also drifts
//! linearly within an interval.
//!
//! **When IERS announces a new leap second, this table must be extended.** A scenario
//! dated after the last entry is converted with the last known offset, which is an
//! extrapolation, not a fact.
//!
//! # The leap-second convention
//!
//! A day that ends with an inserted second is 86 401 SI seconds long, but a Julian Date
//! still names it as one day. Following SOFA's convention (its `eraDat` note 2), a time
//! that falls *within* the leap day uses the offset in force *before* the leap. This is
//! the convention ephemeris files use, so it is the one `STKCraft` follows.

/// One row of the `TAI - UTC` table.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LeapEntry {
    /// Calendar year, UTC.
    pub year: i32,
    /// Calendar month, UTC, 1-12.
    pub month: u32,
    /// The value of `TAI - UTC` from that date forward, in seconds.
    pub tai_minus_utc: f64,
}

/// The date UTC took its present form, when leap seconds began being inserted.
///
/// Before 1972 UTC was offset from TAI by fractions of a second that changed often; a
/// scenario predating this is almost certainly a data error, so the API treats it as a
/// warning the caller can surface rather than silently returning a number.
pub const UTC_INTRODUCED: (i32, u32) = (1960, 1);

/// The last leap second that had been announced when this table was written.
pub const LATEST_LEAP_SECOND: (i32, u32, u32) = (2016, 12, 31);

/// The year the table was last revised; dates more than five years past it are treated
/// as uncertain for the same reason SOFA does.
pub const TABLE_REVISION_YEAR: i32 = 2023;

/// Drift rows for the pre-1972 "rubber second" UTC, as `(MJD reference, s/day)`.
///
/// Kept so that historical dates convert correctly instead of returning a constant.
const DRIFT: [(f64, f64); 14] = [
    (37_300.0, 0.001_296_0),
    (37_300.0, 0.001_296_0),
    (37_300.0, 0.001_296_0),
    (37_665.0, 0.001_123_2),
    (37_665.0, 0.001_123_2),
    (38_761.0, 0.001_296_0),
    (38_761.0, 0.001_296_0),
    (38_761.0, 0.001_296_0),
    (38_761.0, 0.001_296_0),
    (38_761.0, 0.001_296_0),
    (38_761.0, 0.001_296_0),
    (38_761.0, 0.001_296_0),
    (39_126.0, 0.002_592_0),
    (39_126.0, 0.002_592_0),
];

/// Number of entries above that predate the leap-second era.
const PRE_LEAP_ENTRIES: usize = DRIFT.len();

/// The `TAI - UTC` table, oldest first.
pub const TABLE: [LeapEntry; 42] = [
    LeapEntry {
        year: 1960,
        month: 1,
        tai_minus_utc: 1.417_818_0,
    },
    LeapEntry {
        year: 1961,
        month: 1,
        tai_minus_utc: 1.422_818_0,
    },
    LeapEntry {
        year: 1961,
        month: 8,
        tai_minus_utc: 1.372_818_0,
    },
    LeapEntry {
        year: 1962,
        month: 1,
        tai_minus_utc: 1.845_858_0,
    },
    LeapEntry {
        year: 1963,
        month: 11,
        tai_minus_utc: 1.945_858_0,
    },
    LeapEntry {
        year: 1964,
        month: 1,
        tai_minus_utc: 3.240_130_0,
    },
    LeapEntry {
        year: 1964,
        month: 4,
        tai_minus_utc: 3.340_130_0,
    },
    LeapEntry {
        year: 1964,
        month: 9,
        tai_minus_utc: 3.440_130_0,
    },
    LeapEntry {
        year: 1965,
        month: 1,
        tai_minus_utc: 3.540_130_0,
    },
    LeapEntry {
        year: 1965,
        month: 3,
        tai_minus_utc: 3.640_130_0,
    },
    LeapEntry {
        year: 1965,
        month: 7,
        tai_minus_utc: 3.740_130_0,
    },
    LeapEntry {
        year: 1965,
        month: 9,
        tai_minus_utc: 3.840_130_0,
    },
    LeapEntry {
        year: 1966,
        month: 1,
        tai_minus_utc: 4.313_170_0,
    },
    LeapEntry {
        year: 1968,
        month: 2,
        tai_minus_utc: 4.213_170_0,
    },
    LeapEntry {
        year: 1972,
        month: 1,
        tai_minus_utc: 10.0,
    },
    LeapEntry {
        year: 1972,
        month: 7,
        tai_minus_utc: 11.0,
    },
    LeapEntry {
        year: 1973,
        month: 1,
        tai_minus_utc: 12.0,
    },
    LeapEntry {
        year: 1974,
        month: 1,
        tai_minus_utc: 13.0,
    },
    LeapEntry {
        year: 1975,
        month: 1,
        tai_minus_utc: 14.0,
    },
    LeapEntry {
        year: 1976,
        month: 1,
        tai_minus_utc: 15.0,
    },
    LeapEntry {
        year: 1977,
        month: 1,
        tai_minus_utc: 16.0,
    },
    LeapEntry {
        year: 1978,
        month: 1,
        tai_minus_utc: 17.0,
    },
    LeapEntry {
        year: 1979,
        month: 1,
        tai_minus_utc: 18.0,
    },
    LeapEntry {
        year: 1980,
        month: 1,
        tai_minus_utc: 19.0,
    },
    LeapEntry {
        year: 1981,
        month: 7,
        tai_minus_utc: 20.0,
    },
    LeapEntry {
        year: 1982,
        month: 7,
        tai_minus_utc: 21.0,
    },
    LeapEntry {
        year: 1983,
        month: 7,
        tai_minus_utc: 22.0,
    },
    LeapEntry {
        year: 1985,
        month: 7,
        tai_minus_utc: 23.0,
    },
    LeapEntry {
        year: 1988,
        month: 1,
        tai_minus_utc: 24.0,
    },
    LeapEntry {
        year: 1990,
        month: 1,
        tai_minus_utc: 25.0,
    },
    LeapEntry {
        year: 1991,
        month: 1,
        tai_minus_utc: 26.0,
    },
    LeapEntry {
        year: 1992,
        month: 7,
        tai_minus_utc: 27.0,
    },
    LeapEntry {
        year: 1993,
        month: 7,
        tai_minus_utc: 28.0,
    },
    LeapEntry {
        year: 1994,
        month: 7,
        tai_minus_utc: 29.0,
    },
    LeapEntry {
        year: 1996,
        month: 1,
        tai_minus_utc: 30.0,
    },
    LeapEntry {
        year: 1997,
        month: 7,
        tai_minus_utc: 31.0,
    },
    LeapEntry {
        year: 1999,
        month: 1,
        tai_minus_utc: 32.0,
    },
    LeapEntry {
        year: 2006,
        month: 1,
        tai_minus_utc: 33.0,
    },
    LeapEntry {
        year: 2009,
        month: 1,
        tai_minus_utc: 34.0,
    },
    LeapEntry {
        year: 2012,
        month: 7,
        tai_minus_utc: 35.0,
    },
    LeapEntry {
        year: 2015,
        month: 7,
        tai_minus_utc: 36.0,
    },
    LeapEntry {
        year: 2017,
        month: 1,
        tai_minus_utc: 37.0,
    },
];

/// How trustworthy a `TAI - UTC` lookup is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum LeapLookup {
    /// The date falls inside the table: the offset is a published fact.
    Exact,
    /// The date predates UTC: the offset is extrapolated and probably meaningless.
    BeforeUtc,
    /// The date is more than five years past the table's revision: a leap second may
    /// have been announced that this build does not know about.
    AfterRevisions,
}

/// The result of a leap-second lookup, with its confidence carried alongside.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LeapSeconds {
    /// `TAI - UTC` in seconds.
    pub seconds: f64,
    /// Whether that value is a published fact or an extrapolation.
    pub confidence: LeapLookup,
}

/// Look up `TAI - UTC` for a UTC calendar date.
///
/// `month_day` is the day-of-month as a fraction (`1.5` is noon on the 1st). The
/// fraction is meaningful only before 1972, where it selects the drift correction.
///
/// A date that ends with an inserted leap second returns the offset in force *before*
/// that leap, matching SOFA's convention.
#[must_use]
pub fn tai_minus_utc(year: i32, month: u32, month_day: f64) -> LeapSeconds {
    // Find the last entry whose (year, month) is not after the requested one. Walking with
    // an iterator rather than indexing keeps this free of any bounds-check failure path.
    let key = year * 12 + i32::try_from(month).unwrap_or(0);
    let mut selected: Option<(usize, &LeapEntry)> = None;
    for (i, entry) in TABLE.iter().enumerate() {
        if key >= entry.year * 12 + i32::try_from(entry.month).unwrap_or(0) {
            selected = Some((i, entry));
        } else {
            break;
        }
    }

    // A month before the table's first entry belongs to the pre-UTC era. `TABLE` is a
    // non-empty constant, so `first` always yields; the fallback keeps that explicit.
    if (year, month) < UTC_INTRODUCED {
        let seconds = TABLE.first().map_or(0.0, |e| e.tai_minus_utc);
        return LeapSeconds {
            seconds,
            confidence: LeapLookup::BeforeUtc,
        };
    }

    let Some((index, entry)) = selected else {
        return LeapSeconds {
            seconds: 0.0,
            confidence: LeapLookup::BeforeUtc,
        };
    };

    let mut seconds = entry.tai_minus_utc;

    // Before leap seconds existed, TAI - UTC also drifted with the day.
    if index < PRE_LEAP_ENTRIES
        && let Some((mjd_ref, rate)) = DRIFT.get(index).copied()
        && let Some(mjd) = mjd_from_calendar(year, month, month_day)
    {
        // Derive the MJD from the calendar date so the drift is correct.
        seconds += (mjd - mjd_ref) * rate;
    }

    let confidence = if year > TABLE_REVISION_YEAR + 5 {
        LeapLookup::AfterRevisions
    } else {
        LeapLookup::Exact
    };

    LeapSeconds {
        seconds,
        confidence,
    }
}

/// Modified Julian Date for a calendar date, valid in the proleptic Gregorian calendar.
///
/// This is the standard algorithm from the Explanatory Supplement (a shift to
/// March-based months followed by an integer division); it is exact for all dates the
/// table covers. Calendar conversion for user-facing formatting still goes through
/// `chrono`; this exists only so the leap table can index the drift rows.
#[must_use]
pub fn mjd_from_calendar(year: i32, month: u32, month_day: f64) -> Option<f64> {
    if !(1..=12).contains(&month) {
        return None;
    }
    if !(1.0..32.0).contains(&month_day) || !month_day.is_finite() {
        return None;
    }
    let m = i32::try_from(month).ok()?;
    let y = if m <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = i64::from(y - era * 400);
    let mp = i64::from(if m > 2 { m - 3 } else { m + 9 });
    let doy = (153 * mp + 2) / 5 + month_day.floor() as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days_since_unix = i64::from(era) * 146_097 + doe - 719_468;
    // Unix epoch 1970-01-01 is MJD 40587.
    let mjd = 40_587.0 + days_since_unix as f64 + (month_day - month_day.floor());
    Some(mjd)
}

/// A leap-second table a caller can extend with a newer announcement.
///
/// Scenarios dated past [`TABLE_REVISION_YEAR`] are the reason this exists: an operator
/// can supply the current `TAI - UTC` without waiting for a release.
#[derive(Debug, Clone, PartialEq)]
pub struct LeapSecondTable {
    entries: Vec<LeapEntry>,
}

impl LeapSecondTable {
    /// The compiled-in table from the IERS data.
    #[must_use]
    pub fn builtin() -> Self {
        Self {
            entries: TABLE.to_vec(),
        }
    }

    /// The entries, oldest first.
    #[must_use]
    pub fn entries(&self) -> &[LeapEntry] {
        &self.entries
    }

    /// Append a later announcement, keeping the table ordered.
    ///
    /// Returns `false` and changes nothing when the new entry would duplicate or
    /// reorder an existing one, so a bad import cannot corrupt the table.
    pub fn push(&mut self, entry: LeapEntry) -> bool {
        match self.entries.last() {
            Some(last) if (entry.year, entry.month) <= (last.year, last.month) => false,
            _ => {
                self.entries.push(entry);
                true
            }
        }
    }

    /// The offset in force for a UTC date, using this table.
    #[must_use]
    pub fn offset(&self, year: i32, month: u32) -> LeapSeconds {
        let key = year * 12 + i32::try_from(month).unwrap_or(0);
        let mut chosen = self.entries.first().copied();
        for entry in &self.entries {
            if key >= entry.year * 12 + i32::try_from(entry.month).unwrap_or(0) {
                chosen = Some(*entry);
            } else {
                break;
            }
        }
        match chosen {
            Some(entry) => LeapSeconds {
                seconds: entry.tai_minus_utc,
                confidence: LeapLookup::Exact,
            },
            None => LeapSeconds {
                seconds: 0.0,
                confidence: LeapLookup::BeforeUtc,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modern_dates_use_the_published_offset() {
        // 2017 onwards: TAI - UTC = 37 s. This is the value every current ephemeris uses.
        for year in [2017, 2020, 2024, 2026] {
            let v = tai_minus_utc(year, 6, 15.5);
            assert_eq!(v.seconds, 37.0, "year {year}");
            assert_eq!(v.confidence, LeapLookup::Exact);
        }
    }

    #[test]
    fn offsets_step_up_across_announced_leaps() {
        assert_eq!(tai_minus_utc(2016, 12, 1.0).seconds, 36.0);
        assert_eq!(tai_minus_utc(2017, 1, 1.0).seconds, 37.0);
        assert_eq!(tai_minus_utc(2012, 6, 30.0).seconds, 34.0);
        assert_eq!(tai_minus_utc(2012, 7, 1.0).seconds, 35.0);
    }

    #[test]
    fn leap_day_uses_the_offset_in_force_before_the_leap() {
        // 2016-12-31 ends with an inserted second. SOFA's convention (and ours) reports
        // the offset before the leap, so a JD naming that day converts consistently.
        assert_eq!(tai_minus_utc(2016, 12, 31.75).seconds, 36.0);
    }

    #[test]
    fn the_start_of_the_leap_second_era_is_ten_seconds() {
        assert_eq!(tai_minus_utc(1972, 1, 1.0).seconds, 10.0);
    }

    #[test]
    fn pre_1972_offsets_drift_within_the_interval() {
        // 1961-01-01 is the reference MJD for the first drift row, so drift contributes 0.
        let at_reference = tai_minus_utc(1961, 1, 1.0).seconds;
        assert!((at_reference - 1.422_818_0).abs() < 1e-9);
        // A year later the offset must be larger by roughly a year of drift.
        let later = tai_minus_utc(1961, 12, 1.0).seconds;
        assert!(later > at_reference, "drift did not accumulate");
    }

    #[test]
    fn dates_before_utc_are_flagged_not_silently_answered() {
        let v = tai_minus_utc(1950, 1, 1.0);
        assert_eq!(v.confidence, LeapLookup::BeforeUtc);
    }

    #[test]
    fn dates_far_past_the_revision_are_flagged() {
        let v = tai_minus_utc(TABLE_REVISION_YEAR + 10, 1, 1.0);
        assert_eq!(v.confidence, LeapLookup::AfterRevisions);
    }

    #[test]
    fn mjd_agrees_with_known_values() {
        // MJD 0 is 1858-11-17; the Unix epoch is MJD 40587; J2000 (2000-01-01T12:00) is 51544.5.
        let mjd0 = mjd_from_calendar(1858, 11, 17.0);
        assert!(mjd0.is_some());
        if let Some(v) = mjd0 {
            assert!(v.abs() < 1e-9, "MJD epoch gave {v}");
        }
        let unix = mjd_from_calendar(1970, 1, 1.0);
        assert_eq!(unix, Some(40_587.0));
        let j2000 = mjd_from_calendar(2000, 1, 2.0);
        assert_eq!(j2000, Some(51_545.0));
    }

    #[test]
    fn mjd_rejects_impossible_dates() {
        assert_eq!(mjd_from_calendar(2020, 13, 1.0), None);
        assert_eq!(mjd_from_calendar(2020, 1, 0.0), None);
        assert_eq!(mjd_from_calendar(2020, 1, f64::NAN), None);
    }

    #[test]
    fn a_custom_table_can_carry_a_newer_leap_second() {
        let mut table = LeapSecondTable::builtin();
        assert!(table.push(LeapEntry {
            year: 2027,
            month: 1,
            tai_minus_utc: 38.0
        }));
        assert_eq!(table.offset(2027, 6).seconds, 38.0);
        // The compiled-in table is untouched by a caller's extension.
        assert_eq!(tai_minus_utc(2027, 6, 1.0).seconds, 37.0);
    }

    #[test]
    fn a_custom_table_rejects_out_of_order_entries() {
        let mut table = LeapSecondTable::builtin();
        // The built-in table ends at 2017-01, so a 2016 entry would reorder it.
        assert!(!table.push(LeapEntry {
            year: 2016,
            month: 6,
            tai_minus_utc: 99.0
        }));
        assert_eq!(table.entries().len(), TABLE.len());
    }
}
