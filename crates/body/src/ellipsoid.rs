//! Planetocentric and planetodetic coordinates on a body's own ellipsoid.
//!
//! Two latitudes describe the same point and they are not the same number:
//!
//! - **Planetocentric** latitude is the angle between the equatorial plane and the line from the
//!   body's centre to the point. It is what a position vector gives you directly.
//! - **Planetodetic** (also called planetographic for the giant planets) latitude is the angle
//!   between the equatorial plane and the normal to the ellipsoid at the point. It is what a map,
//!   a landing site or a published observation is quoted in.
//!
//! On the Earth they differ by up to 0.19°; on Mars, whose flattening is about four times larger,
//! by up to 0.6°, which is 35 km on the surface. Getting them mixed up is a silent, plausible,
//! wrong answer, so both are named in the type and crossed only through the functions here.
//!
//! # The model
//!
//! A body is an oblate spheroid with equatorial radius `a` (the mean of the two equatorial radii
//! the IAU publishes) and polar radius `c`. The conversion is the standard geodetic one — the
//! closed-form iteration from any geodesy text — and it is iterated three times, which converges
//! to well under a millimetre for the flattenings in the solar system.

/// The ellipsoid of one body: equatorial and polar radii, kilometres.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ellipsoid {
    /// Equatorial radius, km. The mean of the IAU's two equatorial radii.
    pub equatorial_km: f64,
    /// Polar radius, km.
    pub polar_km: f64,
}

/// A point expressed in a body-fixed frame, in the coordinates a map would print.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Geodetic {
    /// Planetodetic latitude, radians.
    pub latitude: f64,
    /// Longitude, radians east of the prime meridian.
    pub longitude: f64,
    /// Height above the ellipsoid along the local normal, km. Negative below the surface.
    pub height_km: f64,
}

impl Ellipsoid {
    /// Build from the IAU's `(equatorial x, equatorial y, polar)` radii in kilometres.
    ///
    /// A body whose two equatorial radii differ (there are a few, e.g. some small satellites) is
    /// treated as oblate with the mean of them, and the difference is documented rather than
    /// silently dropped: a triaxial body needs a rotation-matrix formulation that no body this
    /// crate models requires.
    #[must_use]
    pub const fn from_radii(radii_km: [f64; 3]) -> Self {
        Self {
            equatorial_km: f64::midpoint(radii_km[0], radii_km[1]),
            polar_km: radii_km[2],
        }
    }

    /// A sphere of one radius, for bodies the IAU gives no flattening.
    #[must_use]
    pub const fn spherical(radius_km: f64) -> Self {
        Self {
            equatorial_km: radius_km,
            polar_km: radius_km,
        }
    }

    /// The flattening `(a − c) / a`, dimensionless.
    #[must_use]
    pub fn flattening(&self) -> f64 {
        if self.equatorial_km > 0.0 {
            (self.equatorial_km - self.polar_km) / self.equatorial_km
        } else {
            0.0
        }
    }

    /// The first eccentricity squared, `1 − (c/a)²`.
    #[must_use]
    pub fn eccentricity_squared(&self) -> f64 {
        if self.equatorial_km > 0.0 {
            let ratio = self.polar_km / self.equatorial_km;
            1.0 - ratio * ratio
        } else {
            0.0
        }
    }

    /// The planetodetic latitude of a point given its planetocentric latitude in radians.
    ///
    /// `tan(planetodetic) = (a/c)² · tan(planetocentric)`, which is the plane-geometry identity for
    /// an ellipse and needs no iteration. The point is assumed to be on the surface; use
    /// [`Self::from_body_fixed`] for one at an arbitrary height.
    #[must_use]
    pub fn planetodetic_from_planetocentric(&self, latitude: f64) -> f64 {
        if self.polar_km <= 0.0 {
            return latitude;
        }
        let ratio = self.equatorial_km / self.polar_km;
        (ratio * ratio * latitude.tan()).atan()
    }

    /// The planetocentric latitude of a point given its planetodetic latitude in radians.
    #[must_use]
    pub fn planetocentric_from_planetodetic(&self, latitude: f64) -> f64 {
        if self.equatorial_km <= 0.0 {
            return latitude;
        }
        let ratio = self.polar_km / self.equatorial_km;
        (ratio * ratio * latitude.tan()).atan()
    }

    /// The body-fixed position of a point on the surface, in kilometres.
    ///
    /// The surface point at planetodetic latitude and longitude, which is what "a station on Mars"
    /// means before any altitude is added.
    #[must_use]
    pub fn surface_point(&self, latitude: f64, longitude: f64) -> [f64; 3] {
        let (sin_latitude, cos_latitude) = latitude.sin_cos();
        let (sin_longitude, cos_longitude) = longitude.sin_cos();
        let eccentricity_squared = self.eccentricity_squared();
        let normal =
            self.equatorial_km / (1.0 - eccentricity_squared * sin_latitude * sin_latitude).sqrt();
        [
            normal * cos_latitude * cos_longitude,
            normal * cos_latitude * sin_longitude,
            normal * (1.0 - eccentricity_squared) * sin_latitude,
        ]
    }

    /// The planetodetic coordinates of a body-fixed position, or `None` at the origin.
    ///
    /// The standard iteration: start from the spherical latitude, then solve
    /// `latitude = atan2(z + e²·N·sin(latitude), p)` with `N` the prime vertical radius of
    /// curvature. The iteration converges linearly with a factor of the eccentricity squared, so
    /// five passes take the residual below a nanoradian even for Mars's much larger flattening;
    /// the count is fixed rather than a convergence loop so the cost is bounded and the answer is
    /// reproducible.
    #[must_use]
    pub fn from_body_fixed(&self, point: [f64; 3]) -> Option<Geodetic> {
        let [x, y, z] = point;
        if !(x.is_finite() && y.is_finite() && z.is_finite()) {
            return None;
        }
        let p = x.hypot(y);
        if p == 0.0 && z == 0.0 {
            return None;
        }
        let longitude = y.atan2(x);
        let eccentricity_squared = self.eccentricity_squared();
        let mut latitude = z.atan2(p);
        // Bounded iteration rather than a convergence loop: the factor is the eccentricity
        // squared, so Jupiter's five-times-larger flattening needs more passes than the Earth's,
        // and a cap keeps the cost fixed while the early exit keeps the answer exact.
        for _ in 0..12 {
            let (sin_latitude, _) = latitude.sin_cos();
            let normal = self.equatorial_km
                / (1.0 - eccentricity_squared * sin_latitude * sin_latitude).sqrt();
            let next = (z + eccentricity_squared * normal * sin_latitude).atan2(p);
            let moved = (next - latitude).abs();
            latitude = next;
            if moved < 1e-15 {
                break;
            }
        }
        let (sin_latitude, cos_latitude) = latitude.sin_cos();
        // Height along the normal: the distance from the ellipsoid along the local vertical.
        let normal =
            self.equatorial_km / (1.0 - eccentricity_squared * sin_latitude * sin_latitude).sqrt();
        let height_km = if cos_latitude.abs() > 1e-12 {
            p / cos_latitude - normal
        } else {
            z.abs() - self.polar_km
        };
        Some(Geodetic {
            latitude,
            longitude,
            height_km,
        })
    }
}

use crate::Body;

impl Body {
    /// A point on this body's surface, in its body-fixed frame, kilometres.
    #[must_use]
    pub fn surface_point(&self, latitude: f64, longitude: f64) -> [f64; 3] {
        self.ellipsoid().surface_point(latitude, longitude)
    }
}

#[cfg(test)]
#[expect(clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn a_sphere_has_no_flattening_and_keeps_its_latitudes() {
        let sphere = Ellipsoid::spherical(1_737.4);
        assert_eq!(sphere.flattening(), 0.0);
        assert_eq!(sphere.eccentricity_squared(), 0.0);
        let latitude = 0.7;
        assert!((sphere.planetodetic_from_planetocentric(latitude) - latitude).abs() < 1e-15);
    }

    #[test]
    fn the_earths_flattening_is_wgs_84() {
        // 1/298.257223563 is the WGS-84 flattening; the IAU 2015 radii must reproduce it, which
        // is the cheapest proof that the equatorial and polar radii were not swapped.
        let earth = Body::Earth.ellipsoid();
        let expected = 1.0 / 298.257_223_563;
        assert!(
            (earth.flattening() - expected).abs() < 1e-8,
            "the Earth's flattening is {}, expected {expected}",
            earth.flattening()
        );
    }

    #[test]
    fn the_two_latitudes_differ_by_the_documented_amount() {
        // At 45° the Earth's geodetic latitude exceeds its geocentric one by 0.1924°, and Mars's
        // by about four times that. This is the number that makes the two names worth having.
        let earth = Body::Earth.ellipsoid();
        let difference = earth.planetodetic_from_planetocentric(core::f64::consts::FRAC_PI_4)
            - core::f64::consts::FRAC_PI_4;
        assert!(
            (difference.to_degrees() - 0.1924).abs() < 1e-3,
            "the Earth's latitude difference at 45° is {}°",
            difference.to_degrees()
        );
    }

    #[test]
    fn the_latitude_conversions_are_inverse() {
        for body in [Body::Earth, Body::Mars, Body::Jupiter, Body::Moon] {
            let ellipsoid = body.ellipsoid();
            for latitude in [-1.3, -0.6, 0.0, 0.25, 0.9, 1.4] {
                let planetodetic = ellipsoid.planetodetic_from_planetocentric(latitude);
                let back = ellipsoid.planetocentric_from_planetodetic(planetodetic);
                assert!(
                    (back - latitude).abs() < 1e-12,
                    "{body}: {latitude} rad came back as {back}"
                );
            }
        }
    }

    #[test]
    fn a_surface_point_comes_back_to_its_own_latitude_and_longitude() {
        // The round trip a station on another planet relies on, at the millimetre level.
        for body in [Body::Earth, Body::Mars, Body::Jupiter, Body::Moon] {
            let ellipsoid = body.ellipsoid();
            for latitude in [-1.2, -0.3, 0.0, 0.4, 1.1] {
                for longitude in [-2.8, -0.5, 0.0, 1.7, 3.0] {
                    let point = ellipsoid.surface_point(latitude, longitude);
                    let geodetic = ellipsoid
                        .from_body_fixed(point)
                        .expect("a surface point is not the origin");
                    assert!(
                        (geodetic.latitude - latitude).abs() < 1e-9,
                        "{body} at {latitude},{longitude}: latitude came back {}",
                        geodetic.latitude
                    );
                    assert!(
                        (geodetic.longitude - longitude).abs() < 1e-9,
                        "{body} at {latitude},{longitude}: longitude came back {}",
                        geodetic.longitude
                    );
                    // A tenth of a millimetre. The fixed-point iteration stops when the latitude
                    // stops moving, which on Jupiter's 6 % flattening is a residual of micrometres
                    // — 1e-14 of that body's radius.
                    assert!(
                        geodetic.height_km.abs() < 1e-7,
                        "{body}: a surface point is {} km above the surface",
                        geodetic.height_km
                    );
                }
            }
        }
    }

    #[test]
    fn a_known_height_is_recovered() {
        // A point 100 km above the equator, along the normal, must come back at 100 km.
        let earth = Body::Earth.ellipsoid();
        let mut point = earth.surface_point(0.0, 0.0);
        point[0] += 100.0;
        let geodetic = earth.from_body_fixed(point).expect("not the origin");
        assert!(
            (geodetic.height_km - 100.0).abs() < 1e-6,
            "{}",
            geodetic.height_km
        );
    }

    #[test]
    fn the_origin_has_no_coordinates_rather_than_nans() {
        let ellipsoid = Body::Earth.ellipsoid();
        assert!(ellipsoid.from_body_fixed([0.0, 0.0, 0.0]).is_none());
        assert!(ellipsoid.from_body_fixed([f64::NAN, 0.0, 0.0]).is_none());
        // And the poles, where `p` is zero, are still answerable.
        let pole = ellipsoid
            .from_body_fixed([0.0, 0.0, 6_356.751_9])
            .expect("not the origin");
        assert!((pole.latitude.to_degrees() - 90.0).abs() < 1e-9);
    }
}
