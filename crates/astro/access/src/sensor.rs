//! The acceptance region of a sensor.
//!
//! A sensor sees a target when the line of sight falls inside some region around the station.
//! The simplest is a horizon mask — everything above a minimum elevation — and the general
//! case is that plus a cone about a boresight and an optional range limit, which is what a
//! dish or a camera actually is.
//!
//! The cone test is deliberately a **great-circle angle** between two directions rather than a
//! pair of independent comparisons on azimuth and elevation. Comparing `|Δazimuth|` and
//! `|Δelevation|` against the half-angle separately describes a box in the sky, not a cone,
//! and the corners of that box are accepted when they are `sqrt(2)` times too far from the
//! boresight.

use stkcraft_core::{Angle, Distance, Error, Result, Vector3};

use crate::geometry::Topocentric;

/// The acceptance region of a sensor, in the site's local frame.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Sensor {
    /// The lowest elevation at which the sensor can see: its horizon mask.
    pub minimum_elevation: Angle,
    /// Half-angle of the field-of-view cone about the boresight.
    ///
    /// A value of `180°` is the whole upper hemisphere and makes the cone test vacuous, which
    /// is why [`Sensor::above_horizon`] sets it that way.
    pub half_angle: Angle,
    /// The boresight direction, as `(azimuth, elevation)`. Clockwise from north.
    pub boresight: (Angle, Angle),
    /// Maximum slant range, or `None` for unlimited.
    pub maximum_range: Option<Distance>,
}

impl Sensor {
    /// A sensor that sees everything above a horizon mask, with no cone and no range limit.
    #[must_use]
    pub fn above_horizon(minimum_elevation: Angle) -> Self {
        Self {
            minimum_elevation,
            half_angle: Angle::from_deg(180.0),
            boresight: (Angle::ZERO, Angle::from_deg(90.0)),
            maximum_range: None,
        }
    }

    /// Check that the sensor describes a real acceptance region.
    ///
    /// # Errors
    ///
    /// - [`Error::OutOfRange`] for a minimum elevation outside `[-90°, 90°]`, a half-angle
    ///   outside `[0°, 180°]`, a boresight elevation outside `[-90°, 90°]` or a negative
    ///   maximum range.
    pub fn validate(&self) -> Result<()> {
        let min_el = self.minimum_elevation.as_deg();
        if !(-90.0..=90.0).contains(&min_el) {
            return Err(Error::out_of_range(
                "sensor minimum elevation",
                min_el,
                "-90 to 90 degrees",
            ));
        }
        let half = self.half_angle.as_deg();
        if !(0.0..=180.0).contains(&half) {
            return Err(Error::out_of_range(
                "sensor half-angle",
                half,
                "0 to 180 degrees",
            ));
        }
        let (azimuth, elevation) = self.boresight;
        if !azimuth.as_rad().is_finite() {
            return Err(Error::invalid("boresight azimuth", "must be finite"));
        }
        let el = elevation.as_deg();
        if !(-90.0..=90.0).contains(&el) {
            return Err(Error::out_of_range(
                "boresight elevation",
                el,
                "-90 to 90 degrees",
            ));
        }
        if let Some(range) = self.maximum_range
            && (!range.as_km().is_finite() || range.as_km() < 0.0)
        {
            return Err(Error::out_of_range(
                "sensor maximum range",
                format!("{} km", range.as_km()),
                "zero or more, finite",
            ));
        }
        Ok(())
    }

    /// The boresight as a unit vector in the local east/north/up frame.
    #[must_use]
    pub fn boresight_direction(&self) -> Vector3 {
        let (azimuth, elevation) = self.boresight;
        let (sin_az, cos_az) = azimuth.as_rad().sin_cos();
        let (sin_el, cos_el) = elevation.as_rad().sin_cos();
        // Azimuth is clockwise from north, so east = sin(az) cos(el) and
        // north = cos(az) cos(el).
        Vector3::new(sin_az * cos_el, cos_az * cos_el, sin_el)
    }

    /// The line of sight as a unit vector in the local east/north/up frame.
    #[must_use]
    fn line_of_sight_direction(look: &Topocentric) -> Vector3 {
        let (sin_az, cos_az) = look.azimuth.as_rad().sin_cos();
        let (sin_el, cos_el) = look.elevation.as_rad().sin_cos();
        Vector3::new(sin_az * cos_el, cos_az * cos_el, sin_el)
    }

    /// Whether a line of sight falls inside the acceptance region.
    ///
    /// The horizon mask and the range limit are **inclusive** at their boundaries, and an
    /// exactly-at-mask line of sight is visible. That is a decision, not an accident: it means
    /// a target that touches the mask produces access rather than a zero-length gap, and the
    /// alternative would make the result depend on rounding.
    #[must_use]
    pub fn accepts(&self, look: &Topocentric) -> bool {
        if look.elevation.as_rad() < self.minimum_elevation.as_rad() {
            return false;
        }
        if let Some(maximum) = self.maximum_range
            && look.range.as_km() > maximum.as_km()
        {
            return false;
        }
        self.inside_cone(look)
    }

    /// Whether a line of sight is within the field-of-view cone.
    fn inside_cone(&self, look: &Topocentric) -> bool {
        // A hemisphere is the whole cone, and the great-circle angle degenerates to a
        // comparison with zero there through rounding, so it is short-circuited rather than
        // left to floating point.
        if self.half_angle.as_deg() >= 180.0 {
            return true;
        }
        let separation = self
            .boresight_direction()
            .dot(Self::line_of_sight_direction(look))
            .clamp(-1.0, 1.0)
            .acos();
        separation <= self.half_angle.as_rad()
    }
}

#[cfg(test)]
#[expect(clippy::expect_used)]
mod tests {
    use super::*;

    fn look(azimuth_deg: f64, elevation_deg: f64, range_km: f64) -> Topocentric {
        Topocentric {
            elevation: Angle::from_deg(elevation_deg),
            azimuth: Angle::from_deg(azimuth_deg),
            range: Distance::from_km(range_km),
        }
    }

    #[test]
    fn a_horizon_sensor_accepts_above_and_rejects_below() {
        let sensor = Sensor::above_horizon(Angle::from_deg(10.0));
        sensor.validate().expect("valid sensor");
        assert!(sensor.accepts(&look(0.0, 10.5, 1_000.0)));
        assert!(!sensor.accepts(&look(0.0, 9.5, 1_000.0)));
        assert!(sensor.accepts(&look(180.0, 90.0, 1_000.0)));
        assert!(!sensor.accepts(&look(0.0, -90.0, 1_000.0)));
    }

    #[test]
    fn the_horizon_mask_is_inclusive_at_its_boundary() {
        // Exactly at the mask is visible, so a grazing target produces access rather than a
        // window that depends on rounding.
        let sensor = Sensor::above_horizon(Angle::from_deg(10.0));
        assert!(sensor.accepts(&look(37.0, 10.0, 500.0)));
    }

    #[test]
    fn a_cone_sensor_accepts_only_inside_the_cone() {
        let sensor = Sensor {
            minimum_elevation: Angle::from_deg(0.0),
            half_angle: Angle::from_deg(5.0),
            boresight: (Angle::from_deg(90.0), Angle::from_deg(45.0)),
            maximum_range: None,
        };
        sensor.validate().expect("valid sensor");
        assert!(sensor.accepts(&look(90.0, 45.0, 500.0)), "on boresight");
        assert!(
            sensor.accepts(&look(90.0, 48.0, 500.0)),
            "3 deg off in elevation"
        );
        assert!(
            !sensor.accepts(&look(90.0, 52.0, 500.0)),
            "7 deg off in elevation"
        );

        // In azimuth the separation is foreshortened by cos(elevation), so 4 deg of azimuth
        // at 45 deg elevation is only 2.83 deg on the sky and 7 deg of azimuth is 4.95 deg --
        // still inside a 5 degree cone. The out-of-cone case therefore has to be 10 deg, and
        // asserting 7 were wrong would have been a test bug that passed for the wrong reason.
        assert!(
            sensor.accepts(&look(94.0, 45.0, 500.0)),
            "4 deg of azimuth is 2.83 deg on the sky"
        );
        assert!(
            sensor.accepts(&look(97.0, 45.0, 500.0)),
            "7 deg of azimuth is 4.95 deg on the sky, still inside"
        );
        assert!(
            !sensor.accepts(&look(100.0, 45.0, 500.0)),
            "10 deg of azimuth is 7.07 deg on the sky"
        );
    }

    #[test]
    fn the_cone_is_round_not_square() {
        // The test that separates a great-circle cone from a box, with the boresight due north
        // at 45 degrees elevation. A point 4 degrees off in azimuth and 4 degrees off in
        // elevation is 4.84 degrees away on the sky: outside a 4.5 degree cone, but inside a
        // 4.5 degree box in both coordinates. A pairwise comparison accepts it and this fails.
        let sensor = Sensor {
            minimum_elevation: Angle::from_deg(0.0),
            half_angle: Angle::from_deg(4.5),
            boresight: (Angle::from_deg(0.0), Angle::from_deg(45.0)),
            maximum_range: None,
        };
        assert!(
            !sensor.accepts(&look(4.0, 49.0, 500.0)),
            "a square acceptance region would wrongly accept this corner (4.84 deg away)"
        );
        // The same offsets smaller by half are 2.44 degrees away and must be accepted.
        assert!(sensor.accepts(&look(2.0, 47.0, 500.0)));
    }

    #[test]
    fn a_range_limit_rejects_a_distant_target() {
        let sensor = Sensor {
            minimum_elevation: Angle::from_deg(0.0),
            half_angle: Angle::from_deg(180.0),
            boresight: (Angle::ZERO, Angle::from_deg(90.0)),
            maximum_range: Some(Distance::from_km(2_000.0)),
        };
        sensor.validate().expect("valid sensor");
        assert!(sensor.accepts(&look(0.0, 30.0, 1_999.0)));
        assert!(
            sensor.accepts(&look(0.0, 30.0, 2_000.0)),
            "boundary inclusive"
        );
        assert!(!sensor.accepts(&look(0.0, 30.0, 2_001.0)));
    }

    #[test]
    fn the_boresight_direction_uses_clockwise_azimuth() {
        // Azimuth 90 is east, 0 is north, 180 is south, 270 is west, all at zero elevation.
        let cases = [
            (0.0, 0.0, Vector3::new(0.0, 1.0, 0.0)),
            (90.0, 0.0, Vector3::new(1.0, 0.0, 0.0)),
            (180.0, 0.0, Vector3::new(0.0, -1.0, 0.0)),
            (270.0, 0.0, Vector3::new(-1.0, 0.0, 0.0)),
        ];
        for (azimuth, elevation, expected) in cases {
            let sensor = Sensor {
                minimum_elevation: Angle::from_deg(-90.0),
                half_angle: Angle::from_deg(1.0),
                boresight: (Angle::from_deg(azimuth), Angle::from_deg(elevation)),
                maximum_range: None,
            };
            let got = sensor.boresight_direction();
            assert!(
                (got - expected).norm() < 1e-12,
                "azimuth {azimuth} gave {got:?}"
            );
        }
    }

    #[test]
    fn an_invalid_sensor_is_refused() {
        let bad_elevation = Sensor::above_horizon(Angle::from_deg(91.0));
        assert!(bad_elevation.validate().is_err());

        let bad_half_angle = Sensor {
            minimum_elevation: Angle::ZERO,
            half_angle: Angle::from_deg(181.0),
            boresight: (Angle::ZERO, Angle::from_deg(90.0)),
            maximum_range: None,
        };
        assert!(bad_half_angle.validate().is_err());

        let negative_range = Sensor {
            minimum_elevation: Angle::ZERO,
            half_angle: Angle::from_deg(10.0),
            boresight: (Angle::ZERO, Angle::from_deg(90.0)),
            maximum_range: Some(Distance::from_km(-1.0)),
        };
        assert!(negative_range.validate().is_err());

        let bad_boresight = Sensor {
            minimum_elevation: Angle::ZERO,
            half_angle: Angle::from_deg(10.0),
            boresight: (Angle::ZERO, Angle::from_deg(120.0)),
            maximum_range: None,
        };
        assert!(bad_boresight.validate().is_err());
    }
}
