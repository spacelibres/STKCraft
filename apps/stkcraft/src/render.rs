//! Turning engine results into text a person can read.
//!
//! Kept separate from [`crate::cli`] so the formatting can be tested without running a
//! subprocess, and so the same tables can be reused by a future report exporter.

use std::fmt::Write as _;

use stkcraft_astro_engine::SatelliteAccess;
use stkcraft_time::{Epoch, julian_to_utc_naive};

/// What a pass table describes, so the formatter takes one value instead of eight.
#[derive(Debug, Clone, Copy)]
pub struct Report<'a> {
    /// The station's display name.
    pub station_name: &'a str,
    /// The satellite's display name.
    pub satellite_name: &'a str,
    /// Station geodetic latitude, degrees.
    pub latitude_deg: f64,
    /// Station longitude, degrees.
    pub longitude_deg: f64,
    /// The horizon mask the table was computed with, degrees.
    pub minimum_elevation_deg: f64,
    /// The length of the analysed window, hours.
    pub hours: f64,
    /// The instant the window starts.
    pub epoch: Epoch,
}

/// A pass table for one satellite over one station.
///
/// The times are absolute UTC instants, because a list of offsets from an epoch is something a
/// person has to do arithmetic on before it means anything.
#[must_use]
pub fn pass_table(report: Report<'_>, results: &[SatelliteAccess]) -> String {
    let Report {
        station_name,
        satellite_name,
        latitude_deg,
        longitude_deg,
        minimum_elevation_deg,
        hours,
        epoch,
    } = report;
    let mut text = String::new();
    let _ = writeln!(text, "STKCraft {}", env!("CARGO_PKG_VERSION"));
    let _ = writeln!(
        text,
        "Satellite {satellite_name} over station {station_name}"
    );
    let _ = writeln!(
        text,
        "  station    {latitude_deg:.4} deg lat, {longitude_deg:.4} deg lon, mask {minimum_elevation_deg:.1} deg"
    );
    let _ = writeln!(
        text,
        "  window     {hours:.2} h from {}",
        format_epoch(epoch)
    );
    let _ = writeln!(text);

    // The engine returns one entry per satellite-station pair; this CLI asks for exactly one.
    let Some(access) = results.first() else {
        let _ = writeln!(text, "No access computed.");
        return text;
    };

    if access.windows.is_empty() {
        let _ = writeln!(
            text,
            "No passes. The satellite never rises above the mask in this window."
        );
        return text;
    }

    let _ = writeln!(
        text,
        "{:>4}  {:<19}  {:<19}  {:>10}",
        "#", "start (UTC)", "end (UTC)", "duration"
    );
    let _ = writeln!(text, "{}", "-".repeat(60));

    for (index, window) in access.windows.iter().enumerate() {
        let start = epoch_at(epoch, window.start.as_seconds());
        let end = epoch_at(epoch, window.end.as_seconds());
        let seconds = window.duration().as_seconds();
        let _ = writeln!(
            text,
            "{:>4}  {:<19}  {:<19}  {:>10}",
            index + 1,
            format_epoch(start),
            format_epoch(end),
            format_duration(seconds)
        );
    }

    let _ = writeln!(text, "{}", "-".repeat(60));
    let total = access.total.as_seconds();
    let _ = writeln!(
        text,
        "{} pass(es), {} total ({:.1}% of the window)",
        access.windows.len(),
        format_duration(total),
        100.0 * total / (hours * 3_600.0)
    );
    text
}

/// An absolute instant, recovered from the scenario epoch plus an offset.
fn epoch_at(epoch: Epoch, offset_seconds: f64) -> Epoch {
    Epoch::from_julian(epoch.julian().plus_seconds(offset_seconds), epoch.scale())
}

/// Format an instant as a UTC calendar time.
///
/// Falls back to a numeric form rather than failing, so an unusual epoch still prints
/// something a reader can interpret.
#[must_use]
pub fn format_epoch(epoch: Epoch) -> String {
    // The scenario runs on the element set's scale, which `stkcraft-astro-sgp4` records as TAI.
    // `to_utc` reads the leap table to get back to a calendar time; a numeric fallback keeps
    // an out-of-range epoch printing something interpretable rather than nothing.
    let utc = epoch.to_utc().ok();
    match utc.and_then(|e| julian_to_utc_naive(e.julian())) {
        Some(datetime) => datetime.format("%Y-%m-%d %H:%M:%S").to_string(),
        None => format!("JD {}", epoch.julian().sum()),
    }
}

/// Format an instant for the viewport's clock, to the second.
///
/// The same recovery as [`format_epoch`], with a `UTC` suffix so a screenshot carries its own
/// time scale. Kept separate from the table formatter because a clock that changes width as the
/// seconds tick would make the overlay jitter.
#[must_use]
pub fn format_instant(epoch: Epoch) -> String {
    format!("{} UTC", format_epoch(epoch))
}

/// Format a duration in seconds as hours, minutes and seconds.
#[must_use]
pub fn format_duration(seconds: f64) -> String {
    if !seconds.is_finite() {
        return "n/a".to_owned();
    }
    let total = seconds.round().max(0.0) as u64;
    let hours = total / 3_600;
    let minutes = (total % 3_600) / 60;
    let remainder = total % 60;
    if hours > 0 {
        format!("{hours}h {minutes:02}m {remainder:02}s")
    } else {
        format!("{minutes}m {remainder:02}s")
    }
}

#[cfg(test)]
#[expect(clippy::expect_used)]
mod tests {
    use stkcraft_core::{Duration, EpochOffset};

    use super::*;

    fn window(start: f64, end: f64) -> stkcraft_astro_access::AccessWindow {
        stkcraft_astro_access::AccessWindow {
            start: EpochOffset::from_seconds(start),
            end: EpochOffset::from_seconds(end),
        }
    }

    fn access(windows: Vec<stkcraft_astro_access::AccessWindow>) -> SatelliteAccess {
        let total: f64 = windows.iter().map(|w| w.duration().as_seconds()).sum();
        SatelliteAccess {
            satellite: stkcraft_astro_engine::SatelliteId(1),
            station: stkcraft_astro_engine::StationId(1),
            windows,
            total: Duration::from_seconds(total),
        }
    }

    #[test]
    fn durations_read_as_hours_minutes_and_seconds() {
        assert_eq!(format_duration(0.0), "0m 00s");
        assert_eq!(format_duration(59.4), "0m 59s");
        assert_eq!(format_duration(90.0), "1m 30s");
        assert_eq!(format_duration(3_723.0), "1h 02m 03s");
        assert_eq!(format_duration(f64::NAN), "n/a");
    }

    #[test]
    fn a_pass_table_lists_every_window_with_absolute_times() {
        let epoch = stkcraft_time::utc_from_ymd_hms(2024, 1, 1, 12, 0, 0).expect("valid");
        let text = pass_table(
            Report {
                station_name: "London",
                satellite_name: "25544",
                latitude_deg: 51.5,
                longitude_deg: -0.13,
                minimum_elevation_deg: 10.0,
                hours: 24.0,
                epoch,
            },
            &[access(vec![window(600.0, 1_200.0)])],
        );
        assert!(text.contains("London"));
        assert!(text.contains("25544"));
        // The start is the epoch plus ten minutes, in UTC.
        assert!(
            text.contains("2024-01-01 12:10:00"),
            "start time missing from:\n{text}"
        );
        assert!(text.contains("10m 00s"));
        assert!(text.contains("1 pass(es)"));
    }

    #[test]
    fn no_passes_says_so_rather_than_printing_an_empty_table() {
        let epoch = stkcraft_time::utc_from_ymd_hms(2024, 1, 1, 12, 0, 0).expect("valid");
        let text = pass_table(
            Report {
                station_name: "Nowhere",
                satellite_name: "25544",
                latitude_deg: 0.0,
                longitude_deg: 0.0,
                minimum_elevation_deg: 80.0,
                hours: 24.0,
                epoch,
            },
            &[access(Vec::new())],
        );
        assert!(text.contains("No passes"), "got:\n{text}");
        assert!(!text.contains("start (UTC)"), "an empty table was printed");
    }

    #[test]
    fn the_percentage_of_the_window_is_reported() {
        let epoch = stkcraft_time::utc_from_ymd_hms(2024, 1, 1, 0, 0, 0).expect("valid");
        // 6 hours of access in a 24 hour window is 25%.
        let text = pass_table(
            Report {
                station_name: "S",
                satellite_name: "SAT",
                latitude_deg: 0.0,
                longitude_deg: 0.0,
                minimum_elevation_deg: 0.0,
                hours: 24.0,
                epoch,
            },
            &[access(vec![window(0.0, 21_600.0)])],
        );
        assert!(text.contains("25.0%"), "got:\n{text}");
    }
}
