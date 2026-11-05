//! A site fixed to the rotating Earth, and the local topocentric axes at it.
//!
//! A station is given geodetically — latitude, longitude and height above the ellipsoid —
//! because that is how a real antenna position is surveyed and how it appears on a map.
//! Converting that to an Earth-fixed vector is the step that puts the station where it
//! actually is; the local axes that follow are what elevation and azimuth are measured
//! against.

use stkcraft_core::{Angle, Error, Result, Vector3};

use crate::ellipsoid::Ellipsoid;

/// A point fixed to the rotating Earth, given geodetically.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct GroundSite {
    /// Geodetic latitude, positive north. In `[-90°, 90°]`.
    pub latitude: Angle,
    /// Longitude, positive east. Any finite value; the frame is periodic in it.
    pub longitude: Angle,
    /// Height above the ellipsoid, km.
    pub height_km: f64,
}

/// The local topocentric axes at a site, expressed in the Earth-fixed frame.
///
/// Each axis is an `Option` because a direction that cannot be formed is `None` rather than a
/// NaN vector. In practice all three are `Some` for any valid site; the `Option` is what lets
/// the type be total without a panic path.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LocalFrame {
    /// Unit vector toward local east.
    pub east: Option<Vector3>,
    /// Unit vector toward local north.
    pub north: Option<Vector3>,
    /// Unit vector along the ellipsoid normal, away from the surface.
    pub up: Option<Vector3>,
}

impl GroundSite {
    /// Build a site, checking the ranges.
    ///
    /// # Errors
    ///
    /// - [`Error::OutOfRange`] for a latitude outside `[-90°, 90°]`, a non-finite longitude, or
    ///   a height deeper than the ellipsoid's semi-major axis (such a point is inside the
    ///   Earth's centre, which has no meaning).
    pub fn new(latitude: Angle, longitude: Angle, height_km: f64) -> Result<Self> {
        if !(latitude.as_rad().is_finite()) {
            return Err(Error::invalid("latitude", "must be finite"));
        }
        if latitude.as_deg() < -90.0 || latitude.as_deg() > 90.0 {
            return Err(Error::out_of_range(
                "latitude",
                latitude.as_deg(),
                "-90 to 90 degrees",
            ));
        }
        if !longitude.as_rad().is_finite() {
            return Err(Error::invalid("longitude", "must be finite"));
        }
        if !height_km.is_finite() {
            return Err(Error::invalid(
                "height above the ellipsoid",
                "must be finite",
            ));
        }
        if height_km <= -Ellipsoid::WGS84.semi_major_km {
            return Err(Error::out_of_range(
                "height above the ellipsoid",
                height_km,
                "above minus the semi-major axis",
            ));
        }
        Ok(Self {
            latitude,
            longitude,
            height_km,
        })
    }

    /// Position in an Earth-fixed frame, km.
    ///
    /// ```text
    /// N = a / sqrt(1 - e² sin²φ)
    /// x = (N + h) cos φ cos λ
    /// y = (N + h) cos φ sin λ
    /// z = (N (1 - e²) + h) sin φ
    /// ```
    ///
    /// Source: Vallado, *Fundamentals of Astrodynamics and Applications* (2013), eq. 3-8 to
    /// 3-10. Note the `(1 - e²)` on the z term: omitting it is the mistake that makes a polar
    /// site sit at the equatorial radius.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Invalid`] if the inputs would produce a non-finite position.
    pub fn to_itrf(&self, ellipsoid: &Ellipsoid) -> Result<[f64; 3]> {
        let phi = self.latitude.as_rad();
        let lambda = self.longitude.as_rad();
        let (sin_phi, cos_phi) = phi.sin_cos();
        let (sin_lambda, cos_lambda) = lambda.sin_cos();
        let n = ellipsoid.prime_vertical_radius_km(self.latitude);

        let x = (n + self.height_km) * cos_phi * cos_lambda;
        let y = (n + self.height_km) * cos_phi * sin_lambda;
        let z = (n * (1.0 - ellipsoid.eccentricity_squared()) + self.height_km) * sin_phi;

        if !x.is_finite() || !y.is_finite() || !z.is_finite() {
            return Err(Error::invalid(
                "site position",
                "produced a non-finite value",
            ));
        }
        Ok([x, y, z])
    }

    /// The local east, north and up unit vectors at this site, in the Earth-fixed frame.
    ///
    /// ```text
    /// east  = [-sin λ,          cos λ,          0    ]
    /// north = [-sin φ cos λ,   -sin φ sin λ,    cos φ]
    /// up    = [ cos φ cos λ,    cos φ sin λ,    sin φ]
    /// ```
    ///
    /// Source: Vallado (2013), eq. 3-11 and 3-12. The `up` axis is the **ellipsoid normal**,
    /// not the direction to the geocentre. The two differ by up to `0.19°`, and an elevation
    /// measured against the radial direction instead would be wrong by that much everywhere.
    #[must_use]
    pub fn local_frame(&self) -> LocalFrame {
        let (sin_phi, cos_phi) = self.latitude.as_rad().sin_cos();
        let (sin_lambda, cos_lambda) = self.longitude.as_rad().sin_cos();

        LocalFrame {
            east: Vector3::new(-sin_lambda, cos_lambda, 0.0).normalized(),
            north: Vector3::new(-sin_phi * cos_lambda, -sin_phi * sin_lambda, cos_phi).normalized(),
            up: Vector3::new(cos_phi * cos_lambda, cos_phi * sin_lambda, sin_phi).normalized(),
        }
    }
}

#[cfg(test)]
#[expect(clippy::expect_used)]
mod tests {
    use super::*;

    fn site(latitude_deg: f64, longitude_deg: f64, height_km: f64) -> GroundSite {
        GroundSite::new(
            Angle::from_deg(latitude_deg),
            Angle::from_deg(longitude_deg),
            height_km,
        )
        .expect("valid site")
    }

    #[test]
    fn a_site_at_the_equator_and_prime_meridian_is_on_the_x_axis() {
        let r = site(0.0, 0.0, 0.0)
            .to_itrf(&Ellipsoid::WGS84)
            .expect("valid site");
        assert!((r[0] - 6_378.137).abs() < 1e-9, "x was {}", r[0]);
        assert!(r[1].abs() < 1e-9, "y was {}", r[1]);
        assert!(r[2].abs() < 1e-9, "z was {}", r[2]);
    }

    #[test]
    fn a_site_at_the_north_pole_is_on_the_z_axis_at_the_polar_radius() {
        // With h = 0 a pole site sits at exactly b. A site at the equatorial radius here would
        // mean the `(1 - e²)` factor on the z term had been dropped.
        let r = site(90.0, 0.0, 0.0)
            .to_itrf(&Ellipsoid::WGS84)
            .expect("valid site");
        assert!(r[0].abs() < 1e-9, "x was {}", r[0]);
        assert!(r[1].abs() < 1e-9, "y was {}", r[1]);
        assert!(
            (r[2] - 6_356.752_314_245_179).abs() < 1e-6,
            "z was {}",
            r[2]
        );
    }

    #[test]
    fn a_southern_pole_site_is_the_negative_of_the_northern_one() {
        let north = site(90.0, 0.0, 0.0)
            .to_itrf(&Ellipsoid::WGS84)
            .expect("valid");
        let south = site(-90.0, 0.0, 0.0)
            .to_itrf(&Ellipsoid::WGS84)
            .expect("valid");
        for (a, b) in north.iter().zip(south.iter()) {
            assert!((a + b).abs() < 1e-9);
        }
    }

    #[test]
    fn a_known_station_matches_an_independent_inverse() {
        // 40 N, 75 W, on the ellipsoid. The value below comes from the forward formula, and it
        // was confirmed by inverting it with Bowring's closed form, which shares no code with
        // this function and recovered 40.000000 / -75.000000 / 0.000000 exactly. That round
        // trip is the evidence; the number itself is not from memory.
        let r = site(40.0, -75.0, 0.0)
            .to_itrf(&Ellipsoid::WGS84)
            .expect("valid site");
        assert!((r[0] - 1_266.325_909).abs() < 1e-6, "x was {}", r[0]);
        assert!((r[1] - -4_725.992_631).abs() < 1e-6, "y was {}", r[1]);
        assert!((r[2] - 4_077.985_572).abs() < 1e-6, "z was {}", r[2]);
    }

    #[test]
    fn height_lifts_the_site_along_the_ellipsoid_normal() {
        // Height is along the normal, not radially. At 45 degrees those differ, so the change
        // in position is exactly one kilometre up the normal.
        let ground = site(45.0, 10.0, 0.0);
        let raised = site(45.0, 10.0, 1.0);
        let a = ground.to_itrf(&Ellipsoid::WGS84).expect("valid");
        let b = raised.to_itrf(&Ellipsoid::WGS84).expect("valid");
        let delta = Vector3::new(b[0] - a[0], b[1] - a[1], b[2] - a[2]);
        assert!((delta.norm() - 1.0).abs() < 1e-9, "moved {}", delta.norm());
        let up = ground.local_frame().up.expect("defined");
        assert!((delta.normalized().expect("non-zero") - up).norm() < 1e-12);
    }

    #[test]
    fn the_local_frame_is_orthonormal_and_right_handed() {
        for lat in [-89.0, -45.0, -0.0, 0.0, 30.0, 89.0] {
            for lon in [-179.0, 0.0, 37.0, 179.0] {
                let frame = site(lat, lon, 0.0).local_frame();
                let east = frame.east.expect("defined");
                let north = frame.north.expect("defined");
                let up = frame.up.expect("defined");
                for (name, v) in [("east", east), ("north", north), ("up", up)] {
                    assert!(
                        (v.norm() - 1.0).abs() < 1e-12,
                        "{name} not unit at {lat},{lon}"
                    );
                }
                assert!(east.dot(north).abs() < 1e-12);
                assert!(east.dot(up).abs() < 1e-12);
                assert!(north.dot(up).abs() < 1e-12);
                // Right-handed: east x north = up.
                assert!(
                    (east.cross(north) - up).norm() < 1e-12,
                    "not right-handed at {lat},{lon}"
                );
            }
        }
    }

    #[test]
    fn the_up_axis_is_the_ellipsoid_normal_not_the_radius() {
        // At 45 degrees geodetic and geocentric latitude differ by about 0.192 degrees. A
        // frame built from the radial direction would tilt every elevation by that much, so
        // the difference is asserted rather than assumed.
        let latitude = 45.0;
        let at = site(latitude, 0.0, 0.0);
        let up = at.local_frame().up.expect("defined");
        let r = at.to_itrf(&Ellipsoid::WGS84).expect("valid site");
        let radial = Vector3::new(r[0], r[1], r[2])
            .normalized()
            .expect("non-zero");
        let separation = up.dot(radial).clamp(-1.0, 1.0).acos().to_degrees();
        assert!(
            (separation - 0.192).abs() < 0.01,
            "geodetic vs geocentric was {separation} degrees"
        );
    }

    #[test]
    fn out_of_range_sites_are_refused() {
        assert!(
            GroundSite::new(Angle::from_deg(90.5), Angle::from_deg(0.0), 0.0).is_err(),
            "latitude above the pole"
        );
        assert!(
            GroundSite::new(Angle::from_deg(-91.0), Angle::from_deg(0.0), 0.0).is_err(),
            "latitude below the pole"
        );
        assert!(
            GroundSite::new(Angle::from_deg(0.0), Angle::from_deg(0.0), f64::NAN).is_err(),
            "non-finite height"
        );
        assert!(
            GroundSite::new(Angle::from_deg(0.0), Angle::from_deg(0.0), -7_000.0).is_err(),
            "height inside the geocentre"
        );
        assert!(
            GroundSite::new(Angle::from_rad(f64::NAN), Angle::from_deg(0.0), 0.0).is_err(),
            "non-finite latitude"
        );
    }
}
