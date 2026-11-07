//! IERS `finals2000A.all` Earth-orientation parameters.
//!
//! The file publishes daily `UT1 - UTC`, polar motion `x` and `y`, and the predictions that follow
//! the observed part. The values are interpolated linearly between the published epochs; EOP
//! changes slowly enough that daily linear interpolation is the normal use of this file.
//!
//! Source: IERS Bulletin A, `finals2000A.all` format. The columns used here are the fixed fields
//! for year, month, day, MJD, the two I/P flags, `x`, `y` and `UT1 - UTC`.

use std::path::Path;

use stkcraft_framework_core::{Error, Result};

use crate::teme::EarthOrientation;

/// The largest file this loader will read.
///
/// The real `finals2000A.all` is a few megabytes. A caller-supplied path is input, so the read is
/// bounded rather than trusting the file's length.
pub const MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;

/// Arcseconds in one radian, for the polar-motion columns.
const ARCSEC_TO_RAD: f64 = core::f64::consts::PI / 648_000.0;

/// One daily row of the IERS file.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Entry {
    /// Modified Julian Date, UTC.
    mjd: f64,
    /// Polar motion x, arcseconds.
    x_arcsec: f64,
    /// Polar motion y, arcseconds.
    y_arcsec: f64,
    /// UT1 - UTC, seconds.
    ut1_seconds: f64,
}

/// The parsed Earth-orientation series.
#[derive(Debug, Clone, PartialEq)]
pub struct Finals2000A {
    entries: Vec<Entry>,
}

impl Finals2000A {
    /// Read and parse a `finals2000A.all` file.
    ///
    /// # Errors
    ///
    /// Returns an error for a file larger than [`MAX_FILE_BYTES`], an unreadable file, or a file
    /// with no valid EOP row.
    pub fn from_path(path: &Path) -> Result<Self> {
        let metadata = std::fs::metadata(path).map_err(|error| Error::NotFound {
            what: format!("{}: {error}", path.display()),
        })?;
        if metadata.len() > MAX_FILE_BYTES {
            return Err(Error::out_of_range(
                "finals2000A file size",
                metadata.len() as f64,
                "at most 64 MiB",
            ));
        }
        let text = std::fs::read_to_string(path).map_err(|error| {
            Error::invalid(
                "finals2000A",
                format!(
                    "{}: {error}; the IERS file must be UTF-8 text",
                    path.display()
                ),
            )
        })?;
        Self::parse(&text)
    }

    /// Parse the file's text.
    ///
    /// # Errors
    ///
    /// Returns an error when no row has the expected fields. A malformed row after the first valid
    /// row is skipped: the file carries historical and prediction blocks whose trailing columns
    /// vary, and the three values this loader uses are always in the same leading fields.
    pub fn parse(text: &str) -> Result<Self> {
        let mut entries = Vec::new();
        for line in text.lines() {
            let fields: Vec<&str> = line.split_whitespace().collect();
            let (Some(_year), Some(month), Some(day), Some(mjd)) = (
                field::<u16>(&fields, 0),
                field::<u32>(&fields, 1),
                field::<u32>(&fields, 2),
                field::<f64>(&fields, 3),
            ) else {
                continue;
            };
            if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
                continue;
            }
            // The two flags are `I` (observed) or `P` (prediction). A row with another flag is not
            // an EOP row for this loader.
            let (Some(flag_one), Some(flag_two)) = (fields.get(4), fields.get(9)) else {
                continue;
            };
            if !matches!(*flag_one, "I" | "P") || !matches!(*flag_two, "I" | "P") {
                continue;
            }
            let (Some(x_arcsec), Some(y_arcsec), Some(ut1_seconds)) = (
                field::<f64>(&fields, 5),
                field::<f64>(&fields, 7),
                field::<f64>(&fields, 10),
            ) else {
                continue;
            };
            entries.push(Entry {
                mjd,
                x_arcsec,
                y_arcsec,
                ut1_seconds,
            });
        }
        if entries.is_empty() {
            return Err(Error::invalid(
                "finals2000A",
                "no daily EOP row with the expected fields",
            ));
        }
        entries.sort_by(|left, right| left.mjd.total_cmp(&right.mjd));
        Ok(Self { entries })
    }

    /// The EOP at a UTC Modified Julian Date, linearly interpolated between daily rows.
    ///
    /// # Errors
    ///
    /// Returns [`Error::OutOfRange`] outside the file's date range.
    pub fn at(&self, mjd: f64) -> Result<EarthOrientation> {
        if !mjd.is_finite() {
            return Err(Error::invalid("MJD", "must be finite"));
        }
        let Some(first) = self.entries.first() else {
            return Err(Error::invalid("finals2000A", "no entries"));
        };
        let Some(last) = self.entries.last() else {
            return Err(Error::invalid("finals2000A", "no entries"));
        };
        if mjd < first.mjd || mjd > last.mjd {
            return Err(Error::out_of_range(
                "MJD",
                mjd,
                "within the IERS file's date range",
            ));
        }
        let upper = self.entries.partition_point(|entry| entry.mjd < mjd);
        if upper == 0 {
            return Ok(from_entry(*first));
        }
        if upper >= self.entries.len() {
            return Ok(from_entry(*last));
        }
        let before = self
            .entries
            .get(upper.saturating_sub(1))
            .copied()
            .unwrap_or(*first);
        let after = self.entries.get(upper).copied().unwrap_or(*last);
        let span = after.mjd - before.mjd;
        if span <= 0.0 {
            return Ok(from_entry(before));
        }
        let fraction = (mjd - before.mjd) / span;
        Ok(EarthOrientation {
            dut1_seconds: before.ut1_seconds + (after.ut1_seconds - before.ut1_seconds) * fraction,
            x_polar_rad: (before.x_arcsec + (after.x_arcsec - before.x_arcsec) * fraction)
                * ARCSEC_TO_RAD,
            y_polar_rad: (before.y_arcsec + (after.y_arcsec - before.y_arcsec) * fraction)
                * ARCSEC_TO_RAD,
        })
    }

    /// First and last MJD in the file, as a range for a caller's error message.
    #[must_use]
    pub fn range(&self) -> Option<(f64, f64)> {
        Some((self.entries.first()?.mjd, self.entries.last()?.mjd))
    }
}

fn field<T: std::str::FromStr>(fields: &[&str], index: usize) -> Option<T> {
    fields.get(index)?.parse().ok()
}

fn from_entry(entry: Entry) -> EarthOrientation {
    EarthOrientation {
        dut1_seconds: entry.ut1_seconds,
        x_polar_rad: entry.x_arcsec * ARCSEC_TO_RAD,
        y_polar_rad: entry.y_arcsec * ARCSEC_TO_RAD,
    }
}

#[cfg(test)]
#[expect(clippy::expect_used)]
mod tests {
    use super::*;

    const TWO_ROWS: &str = "\
73 1 2 41684.00 I  0.120733 0.009786  0.136966 0.015902  I 0.8084178 0.0002710  0.0000 0.1916  P
73 1 3 41685.00 I  0.118980 0.011039  0.135656 0.013616  I 0.8056163 0.0002710  3.5563 0.1916  P
";

    #[test]
    fn the_daily_rows_interpolate_linearly() {
        let file = Finals2000A::parse(TWO_ROWS).expect("two valid rows");
        let start = file.at(41_684.0).expect("first row");
        assert!((start.dut1_seconds - 0.808_417_8).abs() < 1e-12);
        let middle = file.at(41_684.0_f64.midpoint(41_685.0)).expect("midpoint");
        let expected = 0.808_417_8_f64.midpoint(0.805_616_3);
        assert!((middle.dut1_seconds - expected).abs() < 1e-12);
        assert!(middle.x_polar_rad.is_finite() && middle.y_polar_rad.is_finite());
    }

    #[test]
    fn a_date_outside_the_file_is_refused() {
        let file = Finals2000A::parse(TWO_ROWS).expect("two valid rows");
        assert!(file.at(41_000.0).is_err());
        assert!(file.at(42_000.0).is_err());
        assert!(file.at(f64::NAN).is_err());
    }

    #[test]
    fn a_file_without_an_eop_row_is_refused() {
        assert!(Finals2000A::parse("not an EOP file\n").is_err());
    }
}
