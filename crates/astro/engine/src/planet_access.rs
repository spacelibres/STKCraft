//! Access from a station on another body, over a timeline.
//!
//! The pieces exist separately: [`PlanetOrbit`] places a satellite about any body, and
//! [`PlanetAccess`] answers whether a station on that body sees a point at one instant. This module
//! is the timeline: it samples the orbit, finds the transitions, and bisects each one to a
//! tolerance a mission analyst would accept.
//!
//! It is deliberately a sampler rather than an analytic window solver. The geometry of a
//! non-spherical body, a rotating station and an inclined orbit has no closed-form window, and a
//! sampler with bisection has one honest disadvantage — the step must be small enough not to step
//! over a short pass — which the caller controls and the documentation states.

use stkcraft_astro_access::PlanetAccess;
use stkcraft_core::{Error, Result};
use stkcraft_time::Epoch;

use crate::central::PlanetOrbit;

/// One interval during which the station saw the satellite.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlanetPass {
    /// Start, seconds after the search began.
    pub start_seconds: f64,
    /// End, seconds after the search began.
    pub end_seconds: f64,
}

impl PlanetPass {
    /// The pass duration, seconds.
    #[must_use]
    pub fn duration_seconds(&self) -> f64 {
        self.end_seconds - self.start_seconds
    }
}

/// How finely the transitions are bisected, seconds.
///
/// A second is far below the scale on which a station's mask geometry changes, and bisection costs
/// about twenty samples per boundary, so this is cheap and precise rather than a trade-off.
pub const BOUNDARY_TOLERANCE_SECONDS: f64 = 1.0;

/// Every pass of `orbit` over `access` in the window, in time order.
///
/// `from` is the start of the search and must be on the TDB scale, because the body's rotation
/// model takes TDB; `hours` is how long to search and `step_seconds` how coarsely to sample.
///
/// # Errors
///
/// [`Error::OutOfRange`] for a non-positive window or step, [`Error::Invalid`] for an epoch on
/// another scale, and whatever the orbit or the access problem rejects.
pub fn find_passes(
    orbit: &PlanetOrbit,
    access: &PlanetAccess,
    from: Epoch,
    hours: f64,
    step_seconds: f64,
) -> Result<Vec<PlanetPass>> {
    if orbit.central != access.body {
        return Err(Error::invalid(
            "planetary access",
            format!(
                "the orbit is about {} and the station is on {}",
                orbit.central, access.body
            ),
        ));
    }
    if !hours.is_finite() || hours <= 0.0 {
        return Err(Error::out_of_range(
            "search window",
            format!("{hours} h"),
            "positive and finite hours",
        ));
    }
    if !step_seconds.is_finite() || step_seconds <= 0.0 {
        return Err(Error::out_of_range(
            "search step",
            format!("{step_seconds} s"),
            "positive and finite seconds",
        ));
    }
    orbit.validate()?;
    access.problem().validate()?;

    let total_seconds = hours * 3_600.0;
    // At a step no smaller than the tolerance the sampler would be slower than the bisection it
    // feeds, and it could miss a pass shorter than one step; both are the caller's to choose.
    let at = |offset: f64| -> Result<bool> {
        let instant = shifted(from, offset);
        let state = orbit.state_at(instant)?;
        match access.visible_at(state.position_km, instant)? {
            Some(visible) => Ok(visible),
            None => Err(Error::invalid(
                "planetary access",
                "the target coincides with the station, so visibility is not defined",
            )),
        }
    };

    let mut passes = Vec::new();
    let mut previous_offset = 0.0_f64;
    let mut previous = at(0.0)?;
    let mut open = if previous { Some(0.0_f64) } else { None };
    let mut offset = step_seconds;
    while offset <= total_seconds {
        let current = at(offset)?;
        if current != previous {
            let boundary = bisect(&at, previous_offset, offset)?;
            if current {
                open = Some(boundary);
            } else if let Some(start) = open.take() {
                passes.push(PlanetPass {
                    start_seconds: start,
                    end_seconds: boundary,
                });
            }
            previous = current;
        }
        previous_offset = offset;
        offset += step_seconds;
    }
    // A pass still open at the end of the window is reported as running to its end: the search
    // cannot know when it would have finished, and truncating silently would hide the pass.
    if let Some(start) = open {
        passes.push(PlanetPass {
            start_seconds: start,
            end_seconds: total_seconds,
        });
    }
    Ok(passes)
}

/// `from` moved by `seconds`, on whatever scale `from` is.
fn shifted(from: Epoch, seconds: f64) -> Epoch {
    Epoch::from_julian(from.julian().plus_seconds(seconds), from.scale())
}

/// Bisect the transition between two offsets, to [`BOUNDARY_TOLERANCE_SECONDS`].
fn bisect(at: &impl Fn(f64) -> Result<bool>, mut before: f64, mut after: f64) -> Result<f64> {
    let state_before = at(before)?;
    let mut iterations = 0;
    while (after - before).abs() > BOUNDARY_TOLERANCE_SECONDS {
        iterations += 1;
        if iterations > 64 {
            return Err(Error::invalid(
                "planetary access",
                "a visibility boundary did not bisect",
            ));
        }
        let middle = f64::midpoint(before, after);
        if at(middle)? == state_before {
            before = middle;
        } else {
            after = middle;
        }
    }
    Ok(after)
}

#[cfg(test)]
#[expect(clippy::expect_used)]
mod tests {
    use super::*;
    use stkcraft_astro_access::{GroundSite, Sensor};
    use stkcraft_astro_body::Body;
    use stkcraft_core::Angle;
    use stkcraft_time::{JD_J2000, TimeScale};

    fn tdb(seconds: f64) -> Epoch {
        Epoch::from_julian(
            stkcraft_time::JulianDate::from_parts(JD_J2000, seconds / 86_400.0),
            TimeScale::Tdb,
        )
    }

    /// A station on Mars's equator with a zero-degree horizon mask, and a low circular orbit.
    fn scenario() -> (PlanetOrbit, PlanetAccess) {
        let orbit = PlanetOrbit::circular(Body::Mars, 4_000.0, 0.0, tdb(0.0)).expect("valid");
        let access = PlanetAccess {
            body: Body::Mars,
            site: GroundSite::new(Angle::from_deg(0.0), Angle::from_deg(0.0), 0.0)
                .expect("a valid site"),
            sensor: Sensor::above_horizon(Angle::from_deg(10.0)),
        };
        (orbit, access)
    }

    #[test]
    fn a_low_orbit_over_a_station_produces_repeated_passes() {
        // A 4 000 km circular orbit about Mars has a period of about 1.9 hours, so a six-hour
        // search must produce more than one pass and each must end after it starts.
        let (orbit, access) = scenario();
        let passes = find_passes(&orbit, &access, tdb(0.0), 6.0, 10.0).expect("a valid search");
        assert!(
            passes.len() >= 2,
            "six hours of a 1.9-hour orbit produced {} pass(es)",
            passes.len()
        );
        for pass in &passes {
            assert!(
                pass.duration_seconds() > 0.0,
                "a pass of {} s is empty",
                pass.duration_seconds()
            );
        }
        // Passes are disjoint and ordered.
        for pair in passes.windows(2) {
            let [a, b] = pair else { continue };
            assert!(
                a.end_seconds <= b.start_seconds + BOUNDARY_TOLERANCE_SECONDS,
                "passes overlap or are out of order"
            );
        }
    }

    #[test]
    fn the_total_time_in_view_is_a_fraction_of_the_window() {
        // A satellite at 4 000 km over a station with a 10° mask is above the mask for well under
        // half the orbit; a result of 90 % of six hours would mean the geometry is wrong.
        let (orbit, access) = scenario();
        let passes = find_passes(&orbit, &access, tdb(0.0), 6.0, 10.0).expect("a valid search");
        let total: f64 = passes.iter().map(PlanetPass::duration_seconds).sum();
        assert!(
            (0.02..0.6).contains(&(total / (6.0 * 3_600.0))),
            "the station saw the satellite for {:.1} % of the window",
            100.0 * total / (6.0 * 3_600.0)
        );
    }

    #[test]
    fn a_station_on_another_body_or_an_impossible_window_is_refused() {
        let (orbit, access) = scenario();
        let mut other = access;
        other.body = Body::Earth;
        assert!(find_passes(&orbit, &other, tdb(0.0), 1.0, 10.0).is_err());
        assert!(find_passes(&orbit, &access, tdb(0.0), 0.0, 10.0).is_err());
        assert!(find_passes(&orbit, &access, tdb(0.0), 1.0, 0.0).is_err());
    }
}
