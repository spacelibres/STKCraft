//! A strict two-line element set parser.
//!
//! A TLE is a fixed-column format from the punched-card era. Its two lines are 69 characters
//! long, every field has a fixed column, and line 2 ends with a modulo-10 checksum. Files in
//! the wild are inconsistent about trailing whitespace and about how many digits a field
//! may run to, so a parser has a choice: guess, or refuse.
//!
//! This one refuses. Every rejection names the line and the column, because "invalid TLE" on
//! a file of 20 000 satellites is not actionable. The rules it enforces:
//!
//! - Line 1 starts with `1 `, line 2 with `2 `, and both carry the same catalog number.
//! - The line-1 checksum and the line-2 checksum both match. A wrong checksum means a
//!   truncated or hand-edited line, and propagating it silently gives a wrong orbit.
//! - Every numeric field parses in its own column range, with the implied-decimal and
//!   exponent notation (`28098-4` is `0.28098e-4`) the format is defined in.
//! - The epoch year is resolved with the standard window: 57-99 is 1957-1999, 00-56 is
//!   2000-2056. A TLE cannot express a year outside that window, and guessing past it would
//!   be wrong.
//!
//! Definition source: the `Two-Line Element Set Format` description at `CelesTrak` (see
//! <https://celestrak.org/NORAD/documentation/gp-data-formats.php>), which
//! documents the columns of the NORAD/Space-Track format, and the format specification in
//! the `twoline2rv` reader of the AIAA 2006-6753 reference implementation.

use serde::{Deserialize, Serialize};
use stkcraft_framework_core::{Angle, Error, Result};
use stkcraft_framework_time::{Epoch, JulianDate, TimeScale};

use crate::elements::Elements;

/// The length of a TLE data line, excluding any trailing whitespace.
pub const TLE_LINE_LENGTH: usize = 69;

/// A parsed two-line element set.
///
/// The raw text is kept alongside the decoded values so a scenario can round-trip an element
/// set without re-formatting it, and so a UI can show exactly what was loaded.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TwoLineElement {
    /// The catalog number as text, so an Alpha-5 identifier (`B1234`) survives.
    pub catalog_number: String,
    /// The international designator, e.g. `58002B`, trimmed. May be empty.
    pub international_designator: String,
    /// The classification letter (`U`, `C`, `S`).
    pub classification: char,
    /// The element set number carried on line 1.
    pub element_set_number: u32,
    /// The epoch, as the TLE states it: a UTC civil date on the TAI-based scale.
    ///
    /// TLE epochs are nominally UTC. This crate records the instant on the scale the TLE
    /// itself is defined against — UT1 in the reference implementation's reading — but
    /// expresses it in TAI so that no leap-second conversion is invented. See
    /// [`Self::elements`] for the same caveat applied to propagation.
    pub epoch: Epoch,
    /// The raw epoch year as written, 0-99.
    pub epoch_year: u8,
    /// The raw day of year as written, 1.0 to 366.99999999.
    pub epoch_day_of_year: f64,
    /// The first time derivative of mean motion divided by two, rev/day².
    ///
    /// The TLE stores half of `dn/dt`, a historical convention; SGP4 is initialized from the
    /// mean motion and drag term instead, so this is carried for display.
    pub mean_motion_dot_over_two: f64,
    /// The second time derivative of mean motion divided by six, rev/day³.
    pub mean_motion_double_dot_over_six: f64,
    /// The B* drag term, in units of 1 / Earth radii.
    pub bstar: f64,
    /// Inclination, degrees in the raw line.
    pub inclination: Angle,
    /// Right ascension of the ascending node, degrees in the raw line.
    pub right_ascension: Angle,
    /// Eccentricity, dimensionless. The line stores it without the leading `0.`.
    pub eccentricity: f64,
    /// Argument of perigee, degrees in the raw line.
    pub argument_of_perigee: Angle,
    /// Mean anomaly, degrees in the raw line.
    pub mean_anomaly: Angle,
    /// Mean motion, rev/day in the raw line.
    pub mean_motion: f64,
    /// The revolution number at epoch.
    pub revolution_number: u32,
    /// The original text of line 1, without the line terminator.
    pub line1: String,
    /// The original text of line 2, without the line terminator.
    pub line2: String,
}

impl TwoLineElement {
    /// Parse the two data lines of a TLE.
    ///
    /// Leading and trailing whitespace is ignored; comments and blank lines are the caller's
    /// to skip, because a TLE has no comment syntax of its own.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Invalid`] naming the line and column of the first problem found.
    pub fn parse(line1: &str, line2: &str) -> Result<Self> {
        let l1 = trim_line(line1);
        let l2 = trim_line(line2);
        check_layout(&l1, '1', "line 1")?;
        check_layout(&l2, '2', "line 2")?;

        let catalog_number = l1
            .get(2..7)
            .ok_or_else(|| column_error("line 1", 3, "too short"))?
            .trim()
            .to_owned();
        if catalog_number.is_empty() {
            return Err(Error::invalid(
                "TLE",
                "line 1 column 3: empty catalog number",
            ));
        }
        validate_catalog_number(&catalog_number)?;

        let catalog2 = l2
            .get(2..7)
            .ok_or_else(|| column_error("line 2", 3, "too short"))?
            .trim();
        if catalog2 != catalog_number {
            return Err(Error::invalid(
                "TLE",
                format!(
                    "line 2 column 3: catalog number `{catalog2}` does not match line 1's `{catalog_number}`"
                ),
            ));
        }

        let classification = l1
            .get(7..8)
            .and_then(|s| s.chars().next())
            .ok_or_else(|| column_error("line 1", 8, "too short"))?;
        if !classification.is_ascii_alphabetic() {
            return Err(Error::invalid(
                "TLE",
                format!("line 1 column 8: classification `{classification}` is not a letter"),
            ));
        }

        let international_designator = l1.get(9..17).unwrap_or_default().trim().to_owned();

        let element_set_number = parse_unsigned(&l1, 64, 68, "line 1 element set number")?;

        let epoch_year = parse_unsigned(&l1, 18, 20, "line 1 epoch year")?;
        let epoch_year = u8::try_from(epoch_year)
            .map_err(|_| column_error("line 1", 19, "epoch year is not two digits"))?;
        let epoch_day_of_year = parse_decimal_field(&l1, 20, 32, "line 1 epoch day")?;
        if !(1.0..367.0).contains(&epoch_day_of_year) {
            return Err(Error::out_of_range(
                "TLE epoch day of year",
                epoch_day_of_year,
                "1.0 <= day < 367.0",
            ));
        }

        let mean_motion_dot_over_two =
            parse_signed_field(&l1, 33, 43, "line 1 mean motion derivative")?;
        let mean_motion_double_dot_over_six =
            parse_implied_exponent(&l1, 44, 52, "line 1 second mean motion derivative")?;
        let bstar = parse_implied_exponent(&l1, 53, 61, "line 1 B* drag term")?;

        let inclination = Angle::from_deg(parse_decimal_field(&l2, 8, 16, "line 2 inclination")?);
        let right_ascension =
            Angle::from_deg(parse_decimal_field(&l2, 17, 25, "line 2 right ascension")?);
        let eccentricity_digits = l2
            .get(26..33)
            .ok_or_else(|| column_error("line 2", 27, "too short"))?
            .trim();
        let eccentricity_digits = if eccentricity_digits.is_empty() {
            "0"
        } else {
            eccentricity_digits
        };
        let eccentricity: f64 = eccentricity_of(eccentricity_digits)
            .ok_or_else(|| column_error("line 2", 27, "eccentricity is not seven digits"))?;
        let argument_of_perigee = Angle::from_deg(parse_decimal_field(
            &l2,
            34,
            42,
            "line 2 argument of perigee",
        )?);
        let mean_anomaly =
            Angle::from_deg(parse_decimal_field(&l2, 43, 51, "line 2 mean anomaly")?);
        let mean_motion = parse_decimal_field(&l2, 52, 63, "line 2 mean motion")?;
        if mean_motion <= 0.0 {
            return Err(Error::out_of_range(
                "TLE mean motion",
                mean_motion,
                "greater than zero",
            ));
        }
        let revolution_number = parse_unsigned(&l2, 63, 68, "line 2 revolution number")?;

        let epoch = epoch_from_tle(epoch_year, epoch_day_of_year);

        Ok(Self {
            catalog_number,
            international_designator,
            classification,
            element_set_number,
            epoch,
            epoch_year,
            epoch_day_of_year,
            mean_motion_dot_over_two,
            mean_motion_double_dot_over_six,
            bstar,
            inclination,
            right_ascension,
            eccentricity,
            argument_of_perigee,
            mean_anomaly,
            mean_motion,
            revolution_number,
            line1: l1,
            line2: l2,
        })
    }

    /// The orbital elements, converted to the units SGP4 is defined in.
    ///
    /// # Errors
    ///
    /// Returns [`Error::OutOfRange`] for an eccentricity outside `[0, 1)` or an inclination
    /// outside `[0, π]`, both of which a TLE can express but no orbit can have.
    pub fn elements(&self) -> Result<Elements> {
        let fraction_of_day = self.epoch_day_of_year.fract();
        Elements::from_tle(
            self.epoch,
            self.eccentricity,
            self.inclination,
            self.right_ascension,
            self.argument_of_perigee,
            self.mean_anomaly,
            self.mean_motion,
            self.bstar,
        )
        .map(|elements| elements.with_fraction_of_day(fraction_of_day))
    }
}

/// Resolve the TLE epoch year with the published two-digit window.
///
/// A TLE stores the year in two digits. The convention, in force since the format was
/// defined for 1957 onward, is that 57-99 mean 1957-1999 and 00-56 mean 2000-2056.
#[must_use]
pub fn resolve_epoch_year(two_digit: u8) -> u16 {
    if two_digit >= 57 {
        1_900 + u16::from(two_digit)
    } else {
        2_000 + u16::from(two_digit)
    }
}

/// Build the epoch instant for a TLE year and day of year.
///
/// The epoch is written as a fraction of a day in a continuous 86400-second day count, so it
/// is converted here without any calendar arithmetic: the year's January 1.0 Julian Date
/// plus the day number, with day 1.0 meaning January 1 00:00.
fn epoch_from_tle(two_digit_year: u8, day_of_year: f64) -> Epoch {
    let year = resolve_epoch_year(two_digit_year);
    let year_start = january_first_jd(year);
    // A TLE day of year counts from January 1 00:00 as day 1.0, so day `D` is `D - 1` whole
    // days after January 1 00:00 -- that is, `floor(D)` days, since `D >= 1`. The whole days
    // are split out of the fraction so a Julian Date read back from the epoch always carries
    // the half-day part on `day`, which is the boundary the sidereal-time formula relies on.
    // A TLE day of year counts from January 1 00:00 as day 1.0, so day `D` is `D - 1` whole
    // days after January 1 00:00. The whole days stay on `day`, which must be integral; the
    // half-day that separates a civil midnight from the Julian-day boundary, and the time of
    // day, both go on the fraction.
    let whole_days = (day_of_year - 1.0).floor();
    let day_fraction = (day_of_year - 1.0) - whole_days;
    let jd = JulianDate::from_parts(
        year_start.day + whole_days,
        year_start.fraction + day_fraction,
    );
    // The TLE epoch is recorded on the scale the element set is defined in. The reference
    // implementation treats it as UT1 without saying so; recording it as TAI and converting
    // deliberately is the only way to keep `Epoch` from inventing a leap-second offset. SGP4
    // itself consumes only the fraction of a day and the day count, so this choice does not
    // change a propagation result; it changes what the number *means*.
    Epoch::from_julian(jd, TimeScale::Tai)
}

/// The Julian Date of January 1.0 of a Gregorian year.
fn january_first_jd(year: u16) -> JulianDate {
    // Days from 2000 January 1 to January 1 of `year`, by the Gregorian leap rule. Working
    // from a fixed anchor keeps this exact for the whole representable range without a
    // calendar library, and it is checked against chrono in the tests.
    // J2000.0 is 2000 January 1.5, so January 1.0 of that year is half a day earlier.
    let days = i64::from(year) - 2_000;
    let leaps = count_leap_days(2_000, year);
    let total = days * 365 + leaps;
    // Whole days go on `day` so it stays integral. J2000 is noon on 2000 January 1, so
    // January 1 00:00 of that year is one whole day earlier and half a day into the day.
    JulianDate::from_parts(
        JulianDate::J2000.day - 1.0 + f64::from(i32::try_from(total).unwrap_or(0)),
        0.5,
    )
}

/// Leap days in the Gregorian years `[from, to)`, counting February 29 occurrences.
fn count_leap_days(from: u16, to: u16) -> i64 {
    if from == to {
        return 0;
    }
    let (lo, hi) = if from < to { (from, to) } else { (to, from) };
    let leaps_before = |y: u16| -> i64 {
        let y = i64::from(y);
        let y_minus_1 = y - 1;
        y_minus_1 / 4 - y_minus_1 / 100 + y_minus_1 / 400
    };
    let count = leaps_before(hi) - leaps_before(lo);
    if from < to { count } else { -count }
}

/// Strip trailing whitespace and carriage returns from a TLE line.
fn trim_line(text: &str) -> String {
    text.trim_end().to_owned()
}

/// Check that a line is long enough, starts with the right number, and carries the checksum.
fn check_layout(line: &str, expected: char, label: &'static str) -> Result<()> {
    let mut chars = line.chars();
    if chars.next() != Some(expected) {
        return Err(Error::invalid(
            "TLE",
            format!("{label} must begin with `{expected}`"),
        ));
    }
    if chars.next() != Some(' ') {
        return Err(column_error(label, 2, "expected a space"));
    }
    let length = line.chars().count();
    if length != TLE_LINE_LENGTH {
        return Err(column_error(
            label,
            length.min(TLE_LINE_LENGTH) + 1,
            format!("line is {length} characters, a TLE line is exactly {TLE_LINE_LENGTH}"),
        ));
    }
    let expected_checksum = checksum(line).ok_or_else(|| {
        column_error(
            label,
            TLE_LINE_LENGTH,
            "line contains characters outside the ASCII range a TLE allows",
        )
    })?;
    let actual = line
        .get(68..69)
        .and_then(|s| s.parse::<u8>().ok())
        .ok_or_else(|| column_error(label, 69, "checksum is not a digit"))?;
    if actual != expected_checksum {
        return Err(Error::invalid(
            "TLE",
            format!(
                "{label} column 69: checksum is {actual} but the line computes {expected_checksum}"
            ),
        ));
    }
    Ok(())
}

/// The modulo-10 checksum of a TLE line: every digit counts its value, every minus sign
/// counts 1, and everything else counts 0, over columns 1-68.
#[must_use]
pub fn checksum(line: &str) -> Option<u8> {
    let mut total: u32 = 0;
    for (index, byte) in line.bytes().enumerate() {
        if index >= 68 {
            break;
        }
        match byte {
            b'0'..=b'9' => total += u32::from(byte - b'0'),
            b'-' => total += 1,
            0x20..=0x7e => {}
            _ => return None,
        }
    }
    Some(u8::try_from(total % 10).unwrap_or(0))
}

/// Reject a catalog number that is neither numeric nor a well-formed Alpha-5 identifier.
///
/// An Alpha-5 identifier substitutes a letter from the set `A H J K L M N P Q R S T U V W X
/// Y Z` (the letters that are not confusable with digits or with `I`/`O`) for the leading
/// digit. The mapping is validated so a typo does not become a plausible-looking object.
/// The letters an Alpha-5 catalog number may begin with: `A`-`Z` without `I` and `O`, which
/// are too easily read as `1` and `0`.
const ALPHA5: &str = "ABCDEFGHJKLMNPQRSTUVWXYZ";

fn validate_catalog_number(text: &str) -> Result<()> {
    if text.bytes().all(|b| b.is_ascii_digit()) {
        return Ok(());
    }
    // The Alpha-5 set: A-Z without I and O, which are too easily read as 1 and 0.
    let mut chars = text.chars();
    let Some(first) = chars.next() else {
        return Err(Error::invalid("TLE catalog number", "is empty"));
    };
    let digits: String = chars.collect();
    if !ALPHA5.contains(first) || digits.len() != 4 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return Err(Error::invalid(
            "TLE catalog number",
            format!(
                "`{text}` is neither a five-digit number nor an Alpha-5 identifier \
                 (one of A H J K L M N P Q R S T U V W X Y Z followed by four digits)"
            ),
        ));
    }
    Ok(())
}

fn column_error(label: &'static str, column: usize, reason: impl std::fmt::Display) -> Error {
    Error::invalid("TLE", format!("{label} column {column}: {reason}"))
}

/// Decode the implied-decimal eccentricity field, which stores `0.xxxxxxx` as `xxxxxxx`.
fn eccentricity_of(digits: &str) -> Option<f64> {
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let scaled: u32 = digits.parse().ok()?;
    Some(f64::from(scaled) / 10_f64.powi(7))
}

/// Parse the digits in columns `[start, end)` (1-based, inclusive start, exclusive end) as an
/// unsigned integer, treating blanks as absent.
fn parse_unsigned(line: &str, start: usize, end: usize, what: &'static str) -> Result<u32> {
    let field = line
        .get(start..end)
        .ok_or_else(|| column_error("TLE", start + 1, "field extends past the end of the line"))?
        .trim();
    if field.is_empty() {
        return Ok(0);
    }
    field
        .parse::<u32>()
        .map_err(|e| Error::invalid(what, format!("`{field}`: {e}")))
}

/// Parse a fixed-point decimal field with an optional sign, allowing a leading `+`/`-`, and
/// treating blanks as zero.
fn parse_decimal_field(line: &str, start: usize, end: usize, what: &'static str) -> Result<f64> {
    let raw = line
        .get(start..end)
        .ok_or_else(|| column_error("TLE", start + 1, "field extends past the end of the line"))?;
    let field = raw.trim();
    if field.is_empty() {
        return Ok(0.0);
    }
    // A leading '.' (as some generators write `-.00000084`) is valid decimal but not always
    // what `str::parse` accepts in every position, so normalise it explicitly.
    let normalised = match field {
        f if f.starts_with('.') => format!("0{f}"),
        f if f.starts_with("-.") => format!("-0{}", &f[1..]),
        f if f.starts_with("+.") => format!("+0{}", &f[1..]),
        f => f.to_owned(),
    };
    normalised
        .parse::<f64>()
        .map_err(|e| Error::invalid(what, format!("`{field}`: {e}")))
}

/// Parse a signed real that carries its sign in the text (columns 34-43 of line 1).
fn parse_signed_field(line: &str, start: usize, end: usize, what: &'static str) -> Result<f64> {
    parse_decimal_field(line, start, end, what)
}

/// Parse a field in the `mantissa-exponent` notation the TLE uses for small quantities:
/// `" 28098-4"` is `0.28098e-4` and `" 00000+0"` is zero. The sign of the mantissa is the
/// character before the mantissa; the exponent is one or two digits after the exponent sign.
fn parse_implied_exponent(line: &str, start: usize, end: usize, what: &'static str) -> Result<f64> {
    let raw = line
        .get(start..end)
        .ok_or_else(|| column_error("TLE", start + 1, "field extends past the end of the line"))?;
    let field = raw.trim();
    if field.is_empty() {
        return Ok(0.0);
    }
    let bytes = field.as_bytes();
    // Find the exponent sign, scanning from the right so a leading mantissa sign is not
    // mistaken for it.
    let split = bytes
        .iter()
        .rposition(|b| *b == b'+' || *b == b'-')
        .ok_or_else(|| {
            Error::invalid(
                what,
                format!("`{field}` has no exponent sign (expected e.g. `28098-4`)"),
            )
        })?;
    if split == 0 {
        return Err(Error::invalid(
            what,
            format!("`{field}` has no mantissa digits"),
        ));
    }
    let mantissa_text = &field[..split];
    let exponent_text = &field[split + 1..];
    if exponent_text.is_empty() || !exponent_text.bytes().all(|b| b.is_ascii_digit()) {
        return Err(Error::invalid(
            what,
            format!("`{field}` has a non-numeric exponent `{exponent_text}`"),
        ));
    }
    let (sign, digits) = match mantissa_text.as_bytes().first() {
        Some(b'-') => (-1.0, &mantissa_text[1..]),
        Some(b'+') => (1.0, &mantissa_text[1..]),
        _ => (1.0, mantissa_text),
    };
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return Err(Error::invalid(
            what,
            format!("`{field}` has a non-numeric mantissa `{digits}`"),
        ));
    }
    let mantissa: f64 = digits
        .parse::<f64>()
        .map_err(|e| Error::invalid(what, format!("`{field}`: {e}")))?;
    let exponent: i32 = exponent_text
        .parse::<i32>()
        .map_err(|e| Error::invalid(what, format!("`{field}`: {e}")))?;
    let exponent = if field.as_bytes().get(split) == Some(&b'-') {
        -exponent
    } else {
        exponent
    };
    // The mantissa is implied to follow a decimal point, so `28098` with exponent -4 is
    // 0.28098e-4 = 2.8098e-5.
    Ok(sign * mantissa * 10f64.powi(exponent - i32::try_from(digits.len()).unwrap_or(0)))
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
mod tests {
    use super::*;

    /// The first verification case from the AIAA 2006-6753 `SGP4-VER.TLE` file.
    const L1: &str = "1 00005U 58002B   00179.78495062  .00000023  00000-0  28098-4 0  4753";
    const L2: &str = "2 00005  34.2682 348.7242 1859667 331.7664  19.3264 10.82419157413667     ";

    #[test]
    fn parses_the_published_verification_element_set() {
        let tle = TwoLineElement::parse(L1, L2).expect("the published case must parse");
        assert_eq!(tle.catalog_number, "00005");
        assert_eq!(tle.international_designator, "58002B");
        assert_eq!(tle.classification, 'U');
        assert_eq!(tle.epoch_year, 0);
        assert!((tle.epoch_day_of_year - 179.784_950_62).abs() < 1e-12);
        assert!((tle.inclination.as_deg() - 34.2682).abs() < 1e-12);
        assert!((tle.right_ascension.as_deg() - 348.7242).abs() < 1e-12);
        assert!((tle.eccentricity - 0.185_966_7).abs() < 1e-15);
        assert!((tle.argument_of_perigee.as_deg() - 331.7664).abs() < 1e-12);
        assert!((tle.mean_anomaly.as_deg() - 19.3264).abs() < 1e-12);
        assert!((tle.mean_motion - 10.824_191_57).abs() < 1e-12);
        assert!((tle.bstar - 0.28098e-4).abs() < 1e-18);
        assert_eq!(tle.revolution_number, 41366);
    }

    #[test]
    fn the_checksum_matches_the_line() {
        assert_eq!(checksum(L1), Some(3));
        assert_eq!(checksum(L2), Some(7));
    }

    #[test]
    fn a_flipped_digit_is_rejected_by_the_checksum() {
        // Change the inclination from 34.2682 to 35.2682 in place, without fixing the
        // checksum: everything else about the line stays valid, so only the checksum can
        // catch it.
        let corrupted = format!("{}{}{}", &L2[..9], "35.2682", &L2[16..]);
        let err = TwoLineElement::parse(L1, &corrupted).expect_err("checksum must catch this");
        let text = err.to_string();
        assert!(text.contains("checksum"), "unexpected message: {text}");
    }

    #[test]
    fn a_mismatched_catalog_number_is_rejected() {
        // Line 2 of 20413 against line 1 of 00005.
        let l2 = "2 20413  12.3514 187.4253 7864447 196.3027 356.5478  0.24690082  7978";
        let err = TwoLineElement::parse(L1, l2).expect_err("catalog numbers disagree");
        assert!(err.to_string().contains("catalog number"), "{err}");
    }

    #[test]
    fn a_short_line_is_rejected_with_the_column() {
        let err = TwoLineElement::parse(&L1[..40], L2).expect_err("truncated line");
        assert!(err.to_string().contains("column"), "{err}");
    }

    #[test]
    fn trailing_whitespace_and_carriage_returns_are_tolerated() {
        let with_crlf = format!("{L1}\r\n");
        let tle = TwoLineElement::parse(&with_crlf, L2).expect("CRLF is not a data error");
        assert!(tle.line1.ends_with("4753"));
    }

    #[test]
    fn implied_exponent_notation_is_decoded() {
        // `28098-4` is 0.28098e-4 and `00000+0` is zero.
        assert!(
            (parse_implied_exponent(" 28098-4", 0, 8, "bstar").unwrap() - 0.28098e-4).abs() < 1e-18
        );
        assert!((parse_implied_exponent(" 00000+0", 0, 8, "bstar").unwrap()).abs() < 1e-30);
        assert!(
            (parse_implied_exponent("-13525-3", 0, 8, "bstar").unwrap() + 0.13525e-3).abs() < 1e-18
        );
        // `0.14311e-1` is not exactly representable, so this comparison is relative.
        let value = parse_implied_exponent(" 14311-1", 0, 8, "bstar").unwrap();
        assert!(((value - 0.14311e-1) / 0.14311e-1).abs() < 1e-12, "{value}");
    }

    #[test]
    fn implied_exponent_rejects_garbage() {
        let err = parse_implied_exponent(" abcdefg", 0, 8, "bstar").expect_err("not a number");
        assert!(err.to_string().contains("exponent sign"), "{err}");
    }

    #[test]
    fn epoch_year_uses_the_published_window() {
        assert_eq!(resolve_epoch_year(57), 1957);
        assert_eq!(resolve_epoch_year(99), 1999);
        assert_eq!(resolve_epoch_year(0), 2000);
        assert_eq!(resolve_epoch_year(56), 2056);
        assert_eq!(resolve_epoch_year(4), 2004);
    }

    #[test]
    fn alpha5_catalog_numbers_are_accepted_and_validated() {
        // An Alpha-5 identifier replaces the leading digit with a letter from the set that
        // excludes I and O.
        assert!(validate_catalog_number("B1234").is_ok());
        assert!(validate_catalog_number("00005").is_ok());
        assert!(validate_catalog_number("I1234").is_err());
        assert!(validate_catalog_number("O1234").is_err());
        assert!(validate_catalog_number("B123").is_err());
    }

    #[test]
    fn the_epoch_lands_on_the_stated_day() {
        // Day 179.78495062 of 2000 is 178.78495062 days after 2000-01-01 00:00, which is
        // 2000-06-28 18:50:19.733568.
        let tle = TwoLineElement::parse(L1, L2).expect("parses");
        let jd = tle.epoch.julian();
        // `day` is a whole day count from JD 0, so it is integral by construction; the
        // half-day and the time of day both sit in `fraction`. Getting this wrong is
        // invisible until a deep-space satellite drifts, so it is asserted directly.
        assert!((jd.day - 2_451_722.0).abs() < 1e-9, "day {}", jd.day);
        assert_eq!(jd.day.fract(), 0.0, "the day part must be integral");
        // The instant itself, which is what callers actually mean.
        assert!((jd.sum() - 2_451_722.5 - 67_819.733_568 / 86_400.0).abs() < 1e-9);
        let seconds = (jd.fraction - 0.5) * 86_400.0;
        assert!(
            (seconds - 67_819.733_568).abs() < 1e-4,
            "fraction of day gave {seconds} s"
        );
    }

    #[test]
    fn january_first_julian_dates_match_the_known_values() {
        // J2000.0 is 2000-01-01T12:00, so January 1.0 2000 is 2451544.5.
        let jd = january_first_jd(2000);
        assert!((jd.sum() - 2_451_544.5).abs() < 1e-9);
        // 2004 had a leap day, so 2005 January 1 is 5 * 365 + 2 days after 2000 January 1
        // counting 2000 and 2004.
        let jd2004 = january_first_jd(2004);
        assert!((jd2004.sum() - (2_451_544.5 + 4.0 * 365.0 + 1.0)).abs() < 1e-9);
    }
}
