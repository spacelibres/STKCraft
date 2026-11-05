//! Access from a station on a body other than the Earth.
//!
//! The Earth path in this crate works in the Earth-fixed frame, because SGP4 and the Earth
//! orientation chain both deliver that. A station on Mars has the same problem shape and two
//! different ingredients: the body's own ellipsoid (its IAU radii, not WGS-84) and the rotation
//! from the body-fixed frame to the ICRF axes, which the body crate's IAU 2015 orientation model
//! supplies at any TDB instant.
//!
//! So this module is a thin adapter rather than a second implementation: it rotates the target into
//! the body's fixed frame and hands the result to [`AccessProblem`], which already knows how to
//! place a site, test the mask and test the occultation. A bug here can only be a rotation or an
//! ellipsoid, and both are the body crate's own, tested against CSPICE.
//!
//! # What is not modelled
//!
//! The body's rotation is taken from the IAU model at the target's instant, with no correction for
//! the light time between the two (a millisecond is 25 km at Mars, but the orientation changes by
//! 0.0013° in that time, which is less than the model's own error). Nothing else is approximated.

use stkcraft_body::Body;
use stkcraft_core::{Error, Result};
use stkcraft_time::Epoch;

use crate::ellipsoid::Ellipsoid;
use crate::problem::AccessProblem;
use crate::sensor::Sensor;
use crate::site::GroundSite;

/// An access problem seen from a station on a given body.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlanetAccess {
    /// The body the station stands on.
    pub body: Body,
    /// The station's planetodetic position on that body.
    pub site: GroundSite,
    /// What the station can see.
    pub sensor: Sensor,
}

impl PlanetAccess {
    /// The ellipsoid of the body, from its IAU 2015 radii.
    ///
    /// Flattening is `(a − c) / a` with the equatorial radius the mean of the two the IAU
    /// publishes, which is the same convention the body crate's own ellipsoid uses.
    #[must_use]
    pub fn ellipsoid(&self) -> Ellipsoid {
        let radii = self.body.radii_km();
        let equatorial = f64::midpoint(radii[0], radii[1]);
        let flattening = if equatorial > 0.0 {
            (equatorial - radii[2]) / equatorial
        } else {
            0.0
        };
        Ellipsoid {
            semi_major_km: equatorial,
            flattening,
        }
    }

    /// The Earth-shaped access problem for this body, for a target already in body-fixed axes.
    ///
    /// Exposed because a caller with its own body-fixed positions — a map, a saved track — should
    /// not have to go through a rotation it does not need.
    #[must_use]
    pub fn problem(&self) -> AccessProblem {
        AccessProblem {
            site: self.site,
            sensor: self.sensor,
            ellipsoid: self.ellipsoid(),
        }
    }

    /// Whether the station sees a target at one instant.
    ///
    /// `target_inertial` is the target's position in ICRF *axes* with the body's centre as the
    /// origin, which is what a two-body propagation about that body produces. `at` must be on the
    /// TDB scale, because the IAU rotation model's argument is TDB.
    ///
    /// # Errors
    ///
    /// [`Error::Invalid`] for an epoch that is not TDB, since the orientation model would otherwise
    /// be evaluated at the wrong argument by up to a millisecond.
    pub fn visible_at(&self, target_inertial: [f64; 3], at: Epoch) -> Result<Option<bool>> {
        if at.scale() != stkcraft_time::TimeScale::Tdb {
            return Err(Error::invalid(
                "planetary access epoch",
                format!(
                    "the IAU orientation model's argument is TDB, not {}; convert the epoch first",
                    at.scale().id()
                ),
            ));
        }
        let matrix = self.body.orientation(at).icrf_to_body_fixed();
        let mut target_fixed = [0.0_f64; 3];
        for (cell, row) in target_fixed.iter_mut().zip(matrix.iter()) {
            *cell = row[0] * target_inertial[0]
                + row[1] * target_inertial[1]
                + row[2] * target_inertial[2];
        }
        Ok(self.problem().visible_at(target_fixed))
    }
}

#[cfg(test)]
#[expect(clippy::expect_used)]
mod tests {
    use super::*;
    use stkcraft_core::Angle;
    use stkcraft_time::{JD_J2000, TimeScale};

    fn tdb(seconds: f64) -> Epoch {
        Epoch::from_julian(
            stkcraft_time::JulianDate::from_parts(JD_J2000, seconds / 86_400.0),
            TimeScale::Tdb,
        )
    }

    fn martian_station(latitude_deg: f64) -> PlanetAccess {
        PlanetAccess {
            body: Body::Mars,
            site: GroundSite::new(Angle::from_deg(latitude_deg), Angle::from_deg(0.0), 0.0)
                .expect("a valid site"),
            sensor: Sensor::above_horizon(Angle::from_deg(0.0)),
        }
    }

    #[test]
    fn mars_gets_mars_flattening_not_the_earths() {
        // Mars is about four times flatter than the Earth; WGS-84 here would put a station 20 km
        // off, which is the whole reason the ellipsoid is taken from the body.
        let access = martian_station(0.0);
        let ellipsoid = access.ellipsoid();
        assert!((ellipsoid.semi_major_km - 3_396.19).abs() < 1e-6);
        let flattening = ellipsoid.flattening;
        assert!(
            (flattening - 0.005_89).abs() < 1e-4,
            "Mars's flattening came out as {flattening}"
        );
        assert!(
            (ellipsoid.semi_minor_km() - 3_376.20).abs() < 1e-3,
            "the polar radius is not the IAU one"
        );
    }

    #[test]
    fn a_target_straight_overhead_is_visible_and_one_below_the_horizon_is_not() {
        // The hand-computable case `AGENTS.md` asks for, on Mars: a body-fixed position on the
        // zenith is visible, and its antipode is not.
        let access = martian_station(0.0);
        let radius = 4_000.0;
        // Rotate a body-fixed zenith vector into the ICRF axes with the body crate's matrix, which
        // is exactly what `visible_at` undoes.
        let orientation = Body::Mars.orientation(tdb(0.0));
        let to_icrf = orientation.body_fixed_to_icrf();
        let through = |fixed: [f64; 3]| {
            let mut out = [0.0_f64; 3];
            for (cell, row) in out.iter_mut().zip(to_icrf.iter()) {
                *cell = row[0] * fixed[0] + row[1] * fixed[1] + row[2] * fixed[2];
            }
            out
        };
        let above = access
            .visible_at(through([radius, 0.0, 0.0]), tdb(0.0))
            .expect("a TDB epoch")
            .expect("the target is not coincident with the station");
        assert!(above, "a target on the zenith must be visible");

        let below = access
            .visible_at(through([-radius, 0.0, 0.0]), tdb(0.0))
            .expect("a TDB epoch")
            .expect("the target is not coincident with the station");
        assert!(!below, "a target through the planet must not be visible");
    }

    #[test]
    fn an_epoch_on_the_wrong_scale_is_refused() {
        let access = martian_station(0.0);
        let tt = Epoch::from_jd(JD_J2000, TimeScale::Tt);
        assert!(access.visible_at([4_000.0, 0.0, 0.0], tt).is_err());
    }
}
