//! Elevation, azimuth and range from a ground site to a target.
//!
//! These are the angular quantities an antenna actually points at, and the ones a horizon mask
//! and a sensor cone are defined in. The conversion is a projection of the line of sight onto
//! the site's local east, north and up axes, so it depends on the frame constructed in
//! [`crate::site`] being right; the tests here are chosen to catch it being wrong.

use stkcraft_core::{Angle, Distance, Vector3};

use crate::site::LocalFrame;

/// A line of sight from a ground site to a target, in the site's local frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Topocentric {
    /// Elevation above the local horizontal plane. Positive is above the horizon.
    pub elevation: Angle,
    /// Azimuth, measured clockwise from north, in `[0°, 360°)`.
    pub azimuth: Angle,
    /// Slant range to the target.
    pub range: Distance,
}

/// The line of sight from a site to a target, both in the Earth-fixed frame, km.
///
/// ```text
/// d         = target - site
/// range     = |d|
/// elevation = asin(d · up / range)
/// azimuth   = atan2(d · east, d · north), normalised to [0, 2pi)
/// ```
///
/// Source: Vallado, *Fundamentals of Astrodynamics and Applications* (2013), section 4.4.1 and
/// Algorithm 27. Azimuth is clockwise from north — the survey convention, not the
/// counter-clockwise one astronomy sometimes uses — and the tests pin all four cardinal
/// directions so the sign cannot drift.
///
/// Returns `None` when the target coincides with the site, so there is no direction. That is
/// the case where an azimuth would otherwise be `atan2(0, 0)` and a NaN would leak into every
/// downstream comparison.
#[must_use]
pub fn look_angle(frame: &LocalFrame, site: [f64; 3], target: [f64; 3]) -> Option<Topocentric> {
    let (Some(east), Some(north), Some(up)) = (frame.east, frame.north, frame.up) else {
        return None;
    };

    let mut delta = [0.0_f64; 3];
    for (component, (t, s)) in delta.iter_mut().zip(target.iter().zip(site.iter())) {
        *component = t - s;
    }
    let direction = Vector3::new(delta[0], delta[1], delta[2]);
    let range_km = direction.norm();
    if !range_km.is_finite() || range_km <= 0.0 {
        return None;
    }

    // The elevation is the arcsin of the up component of the unit line of sight. Clamping is
    // needed even though the value is mathematically in [-1, 1]: the floating-point projection
    // can land a hair outside for a perfectly vertical line.
    let elevation = (direction.dot(up) / range_km).clamp(-1.0, 1.0).asin();
    let east_component = direction.dot(east);
    let north_component = direction.dot(north);

    // `atan2` is undefined for a zero vector; that is a vertical line of sight, where every
    // azimuth describes the same direction, so it is reported as due north rather than as a
    // NaN.
    let azimuth = if east_component == 0.0 && north_component == 0.0 {
        0.0
    } else {
        east_component.atan2(north_component)
    };

    // Fold into [0, 2pi): `atan2` returns (-pi, pi].
    let azimuth = if azimuth < 0.0 {
        azimuth + core::f64::consts::TAU
    } else {
        azimuth
    };

    Some(Topocentric {
        elevation: Angle::from_rad(elevation),
        azimuth: Angle::from_rad(azimuth),
        range: Distance::from_km(range_km),
    })
}

#[cfg(test)]
#[expect(clippy::expect_used)]
mod tests {
    use stkcraft_core::Angle;

    use super::*;
    use crate::ellipsoid::Ellipsoid;
    use crate::site::GroundSite;

    fn site(latitude_deg: f64, longitude_deg: f64) -> GroundSite {
        GroundSite::new(
            Angle::from_deg(latitude_deg),
            Angle::from_deg(longitude_deg),
            0.0,
        )
        .expect("valid site")
    }

    /// Look from a site at a raw Earth-fixed point.
    fn look_from(at: &GroundSite, target: [f64; 3]) -> Topocentric {
        let position = at.to_itrf(&Ellipsoid::WGS84).expect("valid site");
        look_angle(&at.local_frame(), position, target).expect("non-zero range")
    }

    #[test]
    fn a_target_directly_overhead_is_at_ninety_degrees_elevation() {
        // The hand-computable case AGENTS.md asks for: straight up is visible, at zenith.
        // At the equator on the prime meridian the local up axis is +x, so "straight up" is
        // more x, not more z. Getting this backwards is a test bug that looks like a code bug,
        // so the axis is stated here rather than implied.
        let at = site(0.0, 0.0);
        let target = [6_378.137 + 500.0, 0.0, 0.0];
        let view = look_from(&at, target);
        assert!(
            (view.elevation.as_deg() - 90.0).abs() < 1e-9,
            "elevation was {}",
            view.elevation.as_deg()
        );
        assert!((view.range.as_km() - 500.0).abs() < 1e-9);
    }

    #[test]
    fn a_target_directly_below_is_at_minus_ninety_degrees_elevation() {
        // ...and one below the local horizontal is not visible.
        let at = site(0.0, 0.0);
        let view = look_from(&at, [6_378.137 - 500.0, 0.0, 0.0]);
        assert!(
            (view.elevation.as_deg() + 90.0).abs() < 1e-9,
            "elevation was {}",
            view.elevation.as_deg()
        );
        assert!(view.elevation.as_rad() < 0.0);
    }

    #[test]
    fn the_four_cardinal_directions_land_on_their_azimuths() {
        // At the equator on the prime meridian the local axes are exactly the Earth-fixed ones,
        // so a thousand kilometres east, north, south or west must give azimuth 90, 0, 180 and
        // 270. This is the test that pins the clockwise-from-north convention and its sign.
        let at = site(0.0, 0.0);
        let origin = [6_378.137, 0.0, 0.0];
        for (offset, expected_azimuth, name) in [
            ([0.0, 1_000.0, 0.0], 90.0, "east"),
            ([0.0, 0.0, 1_000.0], 0.0, "north"),
            ([0.0, 0.0, -1_000.0], 180.0, "south"),
            ([0.0, -1_000.0, 0.0], 270.0, "west"),
        ] {
            let target = [origin[0], origin[1] + offset[1], origin[2] + offset[2]];
            let view = look_from(&at, target);
            assert!(
                (view.elevation.as_deg()).abs() < 1e-9,
                "{name}: elevation was {}",
                view.elevation.as_deg()
            );
            assert!(
                (view.azimuth.as_deg() - expected_azimuth).abs() < 1e-9,
                "{name}: azimuth was {}",
                view.azimuth.as_deg()
            );
            assert!((view.range.as_km() - 1_000.0).abs() < 1e-9);
        }
    }

    #[test]
    fn azimuth_stays_inside_one_turn() {
        // `atan2` returns (-pi, pi], so due west would otherwise appear as -90 degrees. The
        // range is part of the contract, not a detail.
        let at = site(0.0, 0.0);
        for bearing_deg in (0..360).step_by(7) {
            let bearing = f64::from(bearing_deg).to_radians();
            let target = [6_378.137, 1_000.0 * bearing.sin(), 1_000.0 * bearing.cos()];
            let view = look_from(&at, target);
            assert!(
                (0.0..360.0).contains(&view.azimuth.as_deg()),
                "azimuth {} out of range at bearing {bearing_deg}",
                view.azimuth.as_deg()
            );
        }
    }

    #[test]
    fn a_geostationary_satellite_matches_the_law_of_cosines() {
        // A satellite on the equatorial geostationary radius (42164.17 km) at the station's
        // longitude, seen from 45 N. The elevation and range are fixed by geometry alone, so
        // both are checked: the range by the law of cosines on the triangle
        // (geocentre, station, satellite), and the elevation against the value that leaves.
        let at = site(45.0, 0.0);
        let geostationary_radius_km = 42_164.17;
        let view = look_from(&at, [geostationary_radius_km, 0.0, 0.0]);

        let station = at.to_itrf(&Ellipsoid::WGS84).expect("valid site");
        let station_radius = Vector3::new(station[0], station[1], station[2]).norm();
        // cos(central angle) between the station radius and the satellite radius.
        let central_cos = station[0] / station_radius;
        let expected_range = (station_radius * station_radius
            + geostationary_radius_km * geostationary_radius_km
            - 2.0 * station_radius * geostationary_radius_km * central_cos)
            .sqrt();
        assert!(
            (view.range.as_km() - expected_range).abs() < 1e-6,
            "range {} vs law of cosines {}",
            view.range.as_km(),
            expected_range
        );
        assert!(
            (view.elevation.as_deg() - 38.202_602_6).abs() < 1e-6,
            "elevation was {}",
            view.elevation.as_deg()
        );
        // The satellite is due south of a northern station.
        assert!(
            (view.azimuth.as_deg() - 180.0).abs() < 1e-9,
            "azimuth was {}",
            view.azimuth.as_deg()
        );
    }

    #[test]
    fn a_satellite_on_the_stations_own_horizon_is_at_zero_elevation() {
        // Moving a target along the local horizontal keeps the elevation at zero while the
        // azimuth sweeps: this checks the projection against the right plane, independently of
        // where the horizontal happens to be.
        for latitude in [0.0, 30.0, -60.0] {
            let at = site(latitude, 12.0);
            let origin = at.to_itrf(&Ellipsoid::WGS84).expect("valid site");
            let up = at.local_frame().up.expect("defined");
            for step in 0..12 {
                let bearing = f64::from(step) * core::f64::consts::TAU / 12.0;
                let east = at.local_frame().east.expect("defined");
                let north = at.local_frame().north.expect("defined");
                let horizontal = east * bearing.cos() + north * bearing.sin();
                let target = [
                    origin[0] + horizontal.x * 500.0,
                    origin[1] + horizontal.y * 500.0,
                    origin[2] + horizontal.z * 500.0,
                ];
                let view = look_from(&at, target);
                assert!(
                    view.elevation.as_deg().abs() < 1e-9,
                    "elevation {} at latitude {latitude}, step {step}; up was {up:?}",
                    view.elevation.as_deg()
                );
            }
        }
    }

    #[test]
    fn a_coincident_target_has_no_direction() {
        // The `Option` is the whole point: an azimuth of atan2(0, 0) is a NaN, and a NaN would
        // pass every `>= mask` comparison as false and silently hide the satellite.
        let at = site(10.0, 20.0);
        let position = at.to_itrf(&Ellipsoid::WGS84).expect("valid site");
        assert!(look_angle(&at.local_frame(), position, position).is_none());
    }

    #[test]
    fn the_up_axis_being_the_normal_matters() {
        // At 45 N, measuring elevation against the geocentric radius instead of the ellipsoid
        // normal would shift it by about 0.19 degrees. This test puts a target along the
        // radial direction and asserts the elevation is NOT 90, which is what a radial frame
        // would have produced.
        let at = site(45.0, 0.0);
        let position = at.to_itrf(&Ellipsoid::WGS84).expect("valid site");
        let radial = Vector3::new(position[0], position[1], position[2]);
        let target = [radial.x * 1.1, radial.y * 1.1, radial.z * 1.1];
        let view = look_from(&at, target);
        assert!(
            (view.elevation.as_deg() - 90.0).abs() > 0.15,
            "elevation was {} -- a radial frame would give 90",
            view.elevation.as_deg()
        );
        assert!(
            (view.elevation.as_deg() - (90.0 - 0.192_423)).abs() < 0.01,
            "elevation was {}",
            view.elevation.as_deg()
        );
    }
}
