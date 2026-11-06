//! Turning a visibility predicate into access windows.
//!
//! This is the part of access analysis that is easiest to get subtly wrong. The obvious
//! implementation — sample the timeline, record where visibility is true, and report the run
//! of samples — has two failures that are invisible until someone compares two runs:
//!
//! - **The boundary is snapped to the grid.** An access that starts at 12:03:17.42 s is
//!   reported as starting at whatever sample preceded it, so the answer changes when the step
//!   changes.
//! - **A short pass between two samples disappears entirely.**
//!
//! So this module brackets sign changes and then bisects. The contract is stated precisely
//! below rather than implied, because no point-sampling method can see a window that opens and
//! closes between two samples.

use stkcraft_framework_core::{Duration, EpochOffset, Error, Result};

/// The width to which a window boundary is resolved, seconds.
///
/// A millisecond is far below any physically meaningful access boundary: a low Earth satellite
/// moves about 7 m in that time. It is small enough that the reported time is the real root to
/// the precision anyone will display, and large enough that bisection terminates in about
/// twenty evaluations from a one-hour step.
pub const BOUNDARY_TOLERANCE_SECONDS: f64 = 1e-3;

/// The default search step.
///
/// One minute. A low Earth satellite moves about 7 degrees of arc in that time, which is light
/// enough that a single state change per step is a safe assumption for any mask, and a
/// geostationary pass lasts hours.
pub const DEFAULT_STEP: Duration = Duration::from_seconds(60.0);

/// A cap on how many samples one search may take.
///
/// Ten million is far more than any real scenario needs — a one-second step over a year — and
/// exists so a mistyped step cannot hang the process.
const MAXIMUM_STEPS: f64 = 10_000_000.0;

/// One interval during which access holds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AccessWindow {
    /// Start of access.
    pub start: EpochOffset,
    /// End of access.
    pub end: EpochOffset,
}

impl AccessWindow {
    /// The window's length.
    #[must_use]
    pub fn duration(&self) -> Duration {
        Duration::from_seconds(self.end.as_seconds() - self.start.as_seconds())
    }

    /// Whether an offset falls inside the window, inclusive of both ends.
    #[must_use]
    pub fn contains(&self, at: EpochOffset) -> bool {
        at.as_seconds() >= self.start.as_seconds() && at.as_seconds() <= self.end.as_seconds()
    }
}

/// Find every access window over `[start, end]`.
///
/// `visibility` reports whether access holds at an offset, or `None` where it cannot be
/// evaluated. A `None` is treated as **not visible**: it closes an open window and does not
/// open one. Continuing an interval through a gap in knowability would invent access, and
/// starting one would invent a start time.
///
/// # Contract
///
/// The finder evaluates `visibility` at `step` intervals across `[start, end]` and, wherever
/// the answer differs between two adjacent samples, bisects until the crossing is located to
/// [`BOUNDARY_TOLERANCE_SECONDS`]. That makes the result exact for any visibility function
/// whose state changes at most once between two adjacent samples — which includes every
/// orbital geometry at a sensible step.
///
/// **What it cannot do.** A function that opens and closes entirely between two samples, with
/// the same value at both, is invisible to this method and to any other that only samples
/// points. `step` is therefore the caller's declaration of how fast it expects the geometry to
/// change; shortening it is always safe, only slower.
///
/// The boundaries of the search interval are not crossings: access true at `start` begins
/// there, and access true at `end` ends there.
///
/// # Errors
///
/// - [`Error::OutOfRange`] if `end` is not after `start`, or if `step` is not positive.
/// - [`Error::Unsupported`] if the interval would need more than ten million samples, rather
///   than silently truncating an analysis.
pub fn find_windows<F>(
    start: EpochOffset,
    end: EpochOffset,
    step: Duration,
    visibility: F,
) -> Result<Vec<AccessWindow>>
where
    F: Fn(EpochOffset) -> Option<bool>,
{
    let start_seconds = start.as_seconds();
    let end_seconds = end.as_seconds();
    let step_seconds = step.as_seconds();

    if !start_seconds.is_finite() || !end_seconds.is_finite() {
        return Err(Error::invalid("search interval", "must be finite"));
    }
    if end_seconds <= start_seconds {
        return Err(Error::out_of_range(
            "search interval",
            format!("{start_seconds}..{end_seconds} s"),
            "end strictly after start",
        ));
    }
    if !step_seconds.is_finite() || step_seconds <= 0.0 {
        return Err(Error::out_of_range(
            "search step",
            step_seconds,
            "strictly positive seconds",
        ));
    }
    let required_steps = ((end_seconds - start_seconds) / step_seconds).ceil();
    if required_steps > MAXIMUM_STEPS {
        return Err(Error::Unsupported {
            what: "an access search with more than ten million samples",
        });
    }

    let mut windows = Vec::new();
    // The offset at which the currently-open window began, if one is open.
    let mut opened_at: Option<EpochOffset> = None;

    let mut previous_offset = start;
    let mut previous_visible = visibility(start) == Some(true);
    if previous_visible {
        opened_at = Some(start);
    }

    // `sample` counts steps travelled from `start`, so the offset is a multiplication rather
    // than an accumulation and cannot drift over a long search.
    let mut sample = 1_u64;
    loop {
        let offset_seconds = start_seconds + step_seconds * sample as f64;
        let finished = offset_seconds >= end_seconds;
        let offset = if finished {
            end
        } else {
            EpochOffset::from_seconds(offset_seconds)
        };
        let visible = visibility(offset) == Some(true);

        if visible != previous_visible {
            let boundary = refine(previous_offset, offset, &visibility);
            if visible {
                opened_at = Some(boundary);
            } else if let Some(begin) = opened_at.take() {
                windows.push(AccessWindow {
                    start: begin,
                    end: boundary,
                });
            }
        }

        if finished {
            break;
        }
        previous_offset = offset;
        previous_visible = visible;
        sample = sample.saturating_add(1);
    }

    if let Some(begin) = opened_at {
        windows.push(AccessWindow { start: begin, end });
    }

    Ok(windows)
}

/// Bisect the interval between two samples with different visibility.
///
/// The invariant is that `low` and `high` always straddle a change of state. Each iteration
/// moves whichever end still shares `low`'s state up to the midpoint, so `high` converges on
/// the first offset known to have the *other* state.
fn refine<F>(low: EpochOffset, high: EpochOffset, visibility: &F) -> EpochOffset
where
    F: Fn(EpochOffset) -> Option<bool>,
{
    let mut low = low;
    let mut high = high;
    let low_state = visibility(low);

    while high.as_seconds() - low.as_seconds() > BOUNDARY_TOLERANCE_SECONDS {
        let mid_seconds = low.as_seconds().midpoint(high.as_seconds());
        // If no representable midpoint remains, the two are as close as they can be.
        if !mid_seconds.is_finite()
            || mid_seconds <= low.as_seconds()
            || mid_seconds >= high.as_seconds()
        {
            break;
        }
        let mid = EpochOffset::from_seconds(mid_seconds);
        if visibility(mid) == low_state {
            low = mid;
        } else {
            high = mid;
        }
    }

    high
}

#[cfg(test)]
#[expect(clippy::expect_used)]
mod tests {
    use super::*;

    fn secs(value: f64) -> EpochOffset {
        EpochOffset::from_seconds(value)
    }

    fn step(value: f64) -> Duration {
        Duration::from_seconds(value)
    }

    #[test]
    fn a_permanently_visible_target_gives_one_window_spanning_the_interval() {
        let windows =
            find_windows(secs(0.0), secs(3_600.0), DEFAULT_STEP, |_| Some(true)).expect("valid");
        assert_eq!(windows.len(), 1);
        assert!((windows.first().expect("one").start.as_seconds() - 0.0).abs() < 1e-9);
        assert!((windows.first().expect("one").end.as_seconds() - 3_600.0).abs() < 1e-9);
    }

    #[test]
    fn a_never_visible_target_gives_no_windows() {
        let windows =
            find_windows(secs(0.0), secs(3_600.0), DEFAULT_STEP, |_| Some(false)).expect("valid");
        assert_eq!(windows.len(), 0);
    }

    #[test]
    fn a_target_that_starts_hidden_and_ends_visible_gives_one_half_open_window() {
        // The interval boundaries are not crossings: access that is already true at the start
        // begins there, and access still true at the end ends there.
        let visible_from = 1_800.0;
        let windows = find_windows(secs(0.0), secs(3_600.0), DEFAULT_STEP, |at| {
            Some(at.as_seconds() >= visible_from)
        })
        .expect("valid");
        assert_eq!(windows.len(), 1);
        let w = windows.first().expect("one");
        assert!(
            (w.start.as_seconds() - visible_from).abs() <= BOUNDARY_TOLERANCE_SECONDS,
            "start was {}",
            w.start.as_seconds()
        );
        assert!((w.end.as_seconds() - 3_600.0).abs() < 1e-9);
    }

    #[test]
    fn the_boundary_is_a_root_not_a_sample() {
        // The test that separates bisection from sampling. The boundary is at 1234.5678 s,
        // which is nowhere near a multiple of the 60 s step. An implementation that reported
        // the last sample before the change would say 1200.0 and fail by 34 s.
        let boundary = 1_234.567_8;
        let windows = find_windows(secs(0.0), secs(3_600.0), DEFAULT_STEP, |at| {
            Some(at.as_seconds() >= boundary)
        })
        .expect("valid");
        let w = windows.first().expect("one");
        assert!(
            (w.start.as_seconds() - boundary).abs() <= BOUNDARY_TOLERANCE_SECONDS,
            "start was {} but the root is {boundary}",
            w.start.as_seconds()
        );
    }

    #[test]
    fn a_short_window_between_two_samples_is_found_when_it_straddles_one() {
        // A 20 s pass inside a 300 s step. The samples either side of the middle of the pass
        // see the transition, so bracketing finds it even though no sample is inside.
        let (from, to) = (1_490.0, 1_510.0);
        let windows = find_windows(secs(0.0), secs(3_600.0), step(300.0), |at| {
            let t = at.as_seconds();
            Some(t >= from && t <= to)
        })
        .expect("valid");
        assert_eq!(windows.len(), 1, "a short pass was missed: {windows:?}");
        let w = windows.first().expect("one");
        assert!((w.start.as_seconds() - from).abs() <= BOUNDARY_TOLERANCE_SECONDS);
        assert!((w.end.as_seconds() - to).abs() <= BOUNDARY_TOLERANCE_SECONDS);
    }

    #[test]
    fn an_unevaluable_point_closes_a_window_rather_than_continuing_it() {
        // `None` is not visible. An open window must close at the gap, not run through it.
        let windows = find_windows(secs(0.0), secs(3_600.0), step(60.0), |at| {
            let t = at.as_seconds();
            if (1_000.0..1_500.0).contains(&t) {
                None
            } else {
                Some(true)
            }
        })
        .expect("valid");
        assert_eq!(windows.len(), 2, "expected the gap to split the window");
        let first = windows.first().expect("two");
        let second = windows.last().expect("two");
        assert!(first.end.as_seconds() <= 1_000.0 + 1.0);
        assert!(second.start.as_seconds() >= 1_500.0 - 1.0);
    }

    #[test]
    fn several_passages_are_reported_in_order() {
        // A square wave with a period of 600 s and a duty cycle of a third.
        let windows = find_windows(secs(0.0), secs(3_000.0), step(10.0), |at| {
            Some(at.as_seconds() % 600.0 < 200.0)
        })
        .expect("valid");
        assert!(windows.len() >= 4, "got {} windows", windows.len());
        for pair in windows.windows(2) {
            let earlier = pair.first().expect("pair");
            let later = pair.last().expect("pair");
            assert!(
                later.start.as_seconds() > earlier.end.as_seconds(),
                "windows overlap or are out of order"
            );
        }
    }

    #[test]
    fn a_window_duration_is_the_difference() {
        let w = AccessWindow {
            start: secs(100.0),
            end: secs(400.0),
        };
        assert!((w.duration().as_seconds() - 300.0).abs() < 1e-12);
        assert!(w.contains(secs(100.0)));
        assert!(w.contains(secs(400.0)));
        assert!(!w.contains(secs(99.0)));
    }

    #[test]
    fn invalid_search_parameters_are_refused() {
        assert!(find_windows(secs(100.0), secs(100.0), DEFAULT_STEP, |_| Some(true)).is_err());
        assert!(find_windows(secs(100.0), secs(50.0), DEFAULT_STEP, |_| Some(true)).is_err());
        assert!(find_windows(secs(0.0), secs(100.0), step(0.0), |_| Some(true)).is_err());
        assert!(find_windows(secs(0.0), secs(100.0), step(-1.0), |_| Some(true)).is_err());
        assert!(find_windows(secs(f64::NAN), secs(100.0), DEFAULT_STEP, |_| Some(true)).is_err());
    }

    #[test]
    fn an_absurd_sample_count_is_refused_rather_than_run() {
        // A one-year search at a one-microsecond step is more than ten million samples; the
        // finder refuses instead of running for hours.
        let windows = find_windows(secs(0.0), secs(31_536_000.0), step(1e-6), |_| Some(true));
        assert!(windows.is_err(), "an absurd search was accepted");
    }
}
