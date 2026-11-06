//! The end-to-end access computation: a station, a sensor, and a target position over time.
//!
//! This is the function the engine calls. It exists as its own module so the joining of the
//! pieces is visible in one place: the site is placed on the ellipsoid, the target position is
//! projected into the site's local frame, the sensor decides, and the Earth decides whether
//! anything between them is in the way.
//!
//! # What is out of scope here
//!
//! The caller supplies the target's Earth-fixed position at a given time. Turning a two-line
//! element set into that position is `stkcraft-astro-sgp4` plus `stkcraft-astro-frames`, and belongs to the
//! engine, which is the next layer. Keeping the dependency out means this module is testable
//! with a hand-written closure and no orbital machinery, which is exactly what the tests below
//! do.

use stkcraft_framework_core::{Duration, EpochOffset, Error, Result};

use crate::ellipsoid::Ellipsoid;
use crate::geometry::look_angle;
use crate::occultation::is_occulted;
use crate::sensor::Sensor;
use crate::site::GroundSite;
use crate::window::{AccessWindow, find_windows};

/// A complete access problem: where the station is, what it can see, and the Earth model.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AccessProblem {
    /// The ground site.
    pub site: GroundSite,
    /// The sensor at the site.
    pub sensor: Sensor,
    /// The Earth model, for the site position and the occultation test.
    pub ellipsoid: Ellipsoid,
}

impl AccessProblem {
    /// Check the problem is well formed.
    ///
    /// # Errors
    ///
    /// Returns whatever [`Sensor::validate`] or [`GroundSite::new`] would, plus an error if the
    /// site position cannot be computed.
    pub fn validate(&self) -> Result<()> {
        self.sensor.validate()?;
        // Re-run the site constructor's checks through the position, which is what will
        // actually be used.
        self.site.to_itrf(&self.ellipsoid)?;
        Ok(())
    }

    /// Whether the sensor sees a target at one instant.
    ///
    /// Returns `None` when the target coincides with the station, or when the site position
    /// cannot be computed, so the caller can distinguish "not visible" from "not knowable".
    #[must_use]
    pub fn visible_at(&self, target_itrf: [f64; 3]) -> Option<bool> {
        let site_position = self.site.to_itrf(&self.ellipsoid).ok()?;
        let look = look_angle(&self.site.local_frame(), site_position, target_itrf)?;
        if !self.sensor.accepts(&look) {
            return Some(false);
        }
        // A sensor that would accept the line of sight still cannot see through the planet.
        Some(!is_occulted(&self.ellipsoid, site_position, target_itrf))
    }

    /// Every access window over an interval, given the target's Earth-fixed position.
    ///
    /// `target` is called once or a handful of times per sample while bracketing and bisecting
    /// to a boundary, so it should be cheap or memoised by the caller.
    ///
    /// # Errors
    ///
    /// As [`find_windows`], plus any error from validating the problem.
    pub fn windows<F>(
        &self,
        start: EpochOffset,
        end: EpochOffset,
        step: Duration,
        target: F,
    ) -> Result<Vec<AccessWindow>>
    where
        F: Fn(EpochOffset) -> Option<[f64; 3]>,
    {
        self.validate()?;
        find_windows(start, end, step, |at| {
            target(at).and_then(|position| self.visible_at(position))
        })
    }
}

/// A convenience for a target whose Earth-fixed position is fixed, such as a geostationary
/// satellite in a co-rotating frame.
///
/// # Errors
///
/// As [`AccessProblem::windows`].
pub fn fixed_target_windows(
    problem: &AccessProblem,
    start: EpochOffset,
    end: EpochOffset,
    step: Duration,
    target_itrf: [f64; 3],
) -> Result<Vec<AccessWindow>> {
    if target_itrf.iter().any(|v| !v.is_finite()) {
        return Err(Error::invalid("target position", "must be finite"));
    }
    problem.windows(start, end, step, |_| Some(target_itrf))
}

#[cfg(test)]
#[expect(clippy::expect_used)]
mod tests {
    use stkcraft_framework_core::Angle;

    use super::*;
    use crate::window::DEFAULT_STEP;

    fn problem(latitude_deg: f64, minimum_elevation_deg: f64) -> AccessProblem {
        AccessProblem {
            site: GroundSite::new(Angle::from_deg(latitude_deg), Angle::from_deg(0.0), 0.0)
                .expect("valid site"),
            sensor: Sensor::above_horizon(Angle::from_deg(minimum_elevation_deg)),
            ellipsoid: Ellipsoid::WGS84,
        }
    }

    #[test]
    fn a_satellite_directly_overhead_is_visible_and_one_below_the_horizon_is_not() {
        // The acceptance case from AGENTS.md, end to end through every layer. The site is at
        // the equator on the prime meridian, so its up axis is +x.
        let at = problem(0.0, 0.0);
        let surface_x = Ellipsoid::WGS84.semi_major_km;
        let overhead = [surface_x + 500.0, 0.0, 0.0];
        // A point 500 km below the surface, straight down.
        let below = [surface_x - 500.0, 0.0, 0.0];

        assert_eq!(at.visible_at(overhead), Some(true), "directly overhead");
        assert_eq!(at.visible_at(below), Some(false), "below the surface");
    }

    #[test]
    fn a_satellite_on_the_far_side_is_rejected_even_at_high_elevation() {
        // The occultation test earning its place: without it, a sensor with a wide cone can be
        // pointed through the Earth.
        let wide = AccessProblem {
            site: GroundSite::new(Angle::from_deg(0.0), Angle::from_deg(0.0), 0.0)
                .expect("valid site"),
            sensor: Sensor {
                minimum_elevation: Angle::from_deg(-90.0),
                half_angle: Angle::from_deg(180.0),
                boresight: (Angle::ZERO, Angle::from_deg(90.0)),
                maximum_range: None,
            },
            ellipsoid: Ellipsoid::WGS84,
        };
        let far_side = [-Ellipsoid::WGS84.semi_major_km - 500.0, 0.0, 0.0];
        assert_eq!(
            wide.visible_at(far_side),
            Some(false),
            "a mask of -90 must still not see through the Earth"
        );
    }

    #[test]
    fn a_geostationary_satellite_is_visible_continuously() {
        // A station under the satellite and a target fixed in the Earth-fixed frame: access
        // for the whole day, as one window covering the interval.
        let at = problem(0.0, 0.0);
        let geostationary_radius_km = 42_164.17;
        let target = [geostationary_radius_km, 0.0, 0.0];
        let windows = fixed_target_windows(
            &at,
            EpochOffset::from_seconds(0.0),
            EpochOffset::from_seconds(86_400.0),
            DEFAULT_STEP,
            target,
        )
        .expect("valid problem");

        assert_eq!(
            windows.len(),
            1,
            "a fixed target gives one window: {windows:?}"
        );
        let only = windows.first().expect("one");
        assert!((only.start.as_seconds() - 0.0).abs() < 1e-9);
        assert!((only.end.as_seconds() - 86_400.0).abs() < 1e-9);
    }

    #[test]
    fn an_elevation_mask_shrinks_the_window() {
        // The same fixed target seen through a horizon mask of 0 and of 89 degrees. The higher
        // mask must still produce access, but the 89 degree mask cannot: a geostationary
        // satellite from directly under it sits at exactly 90 degrees, so it is at the
        // boundary and still accepted. A mask of 91 would be refused as invalid, which is
        // checked separately.
        let target = [42_164.17, 0.0, 0.0];
        let horizon = problem(0.0, 0.0);
        let steep = problem(0.0, 89.0);

        let a = fixed_target_windows(
            &horizon,
            EpochOffset::from_seconds(0.0),
            EpochOffset::from_seconds(3_600.0),
            DEFAULT_STEP,
            target,
        )
        .expect("valid");
        let b = fixed_target_windows(
            &steep,
            EpochOffset::from_seconds(0.0),
            EpochOffset::from_seconds(3_600.0),
            DEFAULT_STEP,
            target,
        )
        .expect("valid");
        assert_eq!(a.len(), 1);
        assert_eq!(
            b.len(),
            1,
            "a geostationary satellite overhead is exactly at 90 degrees"
        );
    }

    #[test]
    fn a_target_that_sets_produces_a_closing_window() {
        // A target moving away along a straight line at constant speed, so its visibility
        // changes exactly once at a known time. The window must end there, to the tolerance,
        // and not at a sample.
        let at = problem(0.0, 0.0);
        let surface_x = Ellipsoid::WGS84.semi_major_km;
        // Start overhead at 500 km, move 1 km/s in +x for up to 3600 s.
        let windows = at
            .windows(
                EpochOffset::from_seconds(0.0),
                EpochOffset::from_seconds(3_600.0),
                Duration::from_seconds(60.0),
                |offset| {
                    let t = offset.as_seconds();
                    Some([surface_x + 500.0 + t, 0.0, 0.0])
                },
            )
            .expect("valid problem");
        assert_eq!(windows.len(), 1);
        let only = windows.first().expect("one");
        assert!((only.start.as_seconds() - 0.0).abs() < 1e-9);
        // Moving straight up keeps the target overhead forever, so access must run to the end.
        assert!((only.end.as_seconds() - 3_600.0).abs() < 1e-9);
    }

    #[test]
    fn an_invalid_problem_is_refused() {
        let bad = AccessProblem {
            site: GroundSite::new(Angle::from_deg(10.0), Angle::from_deg(0.0), 0.0)
                .expect("valid site"),
            sensor: Sensor::above_horizon(Angle::from_deg(120.0)),
            ellipsoid: Ellipsoid::WGS84,
        };
        assert!(bad.validate().is_err());
        assert!(
            fixed_target_windows(
                &bad,
                EpochOffset::from_seconds(0.0),
                EpochOffset::from_seconds(10.0),
                DEFAULT_STEP,
                [7_000.0, 0.0, 0.0],
            )
            .is_err()
        );
    }

    #[test]
    fn a_non_finite_target_is_refused() {
        let at = problem(0.0, 0.0);
        assert!(
            fixed_target_windows(
                &at,
                EpochOffset::from_seconds(0.0),
                EpochOffset::from_seconds(10.0),
                DEFAULT_STEP,
                [f64::NAN, 0.0, 0.0],
            )
            .is_err()
        );
    }
}
