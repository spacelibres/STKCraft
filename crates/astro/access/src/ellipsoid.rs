//! The WGS-84 reference ellipsoid.
//!
//! A spherical Earth is not good enough here. Geodetic and geocentric latitude differ by up to
//! `0.19°`, which puts a station up to 21 km from where it is, and that is larger than many
//! sensor footprints. The Earth is modelled as the WGS-84 ellipsoid of revolution.
//!
//! Source: National Imagery and Mapping Agency, *Department of Defense World Geodetic System
//! 1984: Its Definition and Relationships with Local Geodetic Systems*, NIMA TR8350.2,
//! 3rd edition (2000). The two defining constants are the semi-major axis and the inverse
//! flattening; everything else below is derived from them rather than transcribed, because a
//! derived value typed from memory is a transcription error waiting to happen.

use stkcraft_core::Angle;

/// The WGS-84 semi-major axis (equatorial radius), km.
///
/// Exactly `6_378_137.0` m by definition.
pub const WGS84_SEMI_MAJOR_KM: f64 = 6_378.137;

/// The WGS-84 inverse flattening, `1/f`.
///
/// Exactly `298.257_223_563` by definition.
pub const WGS84_INVERSE_FLATTENING: f64 = 298.257_223_563;

/// The WGS-84 flattening, `f = 1 / (1/f)`.
pub const WGS84_FLATTENING: f64 = 1.0 / WGS84_INVERSE_FLATTENING;

/// An ellipsoid of revolution, defined by its semi-major axis and flattening.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ellipsoid {
    /// Equatorial radius, km.
    pub semi_major_km: f64,
    /// Flattening, dimensionless.
    pub flattening: f64,
}

impl Ellipsoid {
    /// The WGS-84 ellipsoid.
    pub const WGS84: Self = Self {
        semi_major_km: WGS84_SEMI_MAJOR_KM,
        flattening: WGS84_FLATTENING,
    };

    /// The polar radius, `b = a (1 - f)`, km.
    #[must_use]
    pub fn semi_minor_km(&self) -> f64 {
        self.semi_major_km * (1.0 - self.flattening)
    }

    /// The first eccentricity squared, `e² = 2f - f²`.
    ///
    /// Written this way rather than as `(a² - b²)/a²` because it needs no subtraction of two
    /// nearly equal large numbers.
    #[must_use]
    pub fn eccentricity_squared(&self) -> f64 {
        2.0 * self.flattening - self.flattening * self.flattening
    }

    /// The radius of curvature in the prime vertical at a geodetic latitude, km.
    ///
    /// `N = a / sqrt(1 - e² sin²φ)`. At the equator this is the semi-major axis; at the pole it
    /// is `a / sqrt(1 - e²)`, which is **not** the semi-minor axis — that is the meridional
    /// radius of curvature. Confusing the two is the classic ellipsoid mistake.
    #[must_use]
    pub fn prime_vertical_radius_km(&self, latitude: Angle) -> f64 {
        let sin_lat = latitude.as_rad().sin();
        self.semi_major_km / (1.0 - self.eccentricity_squared() * sin_lat * sin_lat).sqrt()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wgs84_derived_values_match_the_published_definition() {
        // Source: NIMA TR8350.2, 3rd ed. (2000). The two defining constants are exact; every
        // derived value follows from them and is checked to the precision those carry.
        let e = Ellipsoid::WGS84;
        assert!((e.semi_major_km - 6_378.137).abs() < 1e-12);

        // b = a(1 - f) = 6_356_752.314_245_179 m. The value is written to the twelve digits
        // the arithmetic actually produces, not to the ten a table might print, so the test
        // fails if the formula changes rather than only if it drifts.
        assert!(
            (e.semi_minor_km() - 6_356.752_314_245_179).abs() < 1e-12,
            "semi-minor was {}",
            e.semi_minor_km()
        );

        // e² = 2f - f² = 0.006_694_379_990_14
        assert!(
            (e.eccentricity_squared() - 0.006_694_379_990_14).abs() < 1e-14,
            "e² was {}",
            e.eccentricity_squared()
        );
    }

    #[test]
    fn the_prime_vertical_radius_matches_published_values() {
        // `N(φ)` at the equator is the semi-major axis, and at the pole is a/sqrt(1 - e²),
        // which is about 6399.59 km. The test pins both, because a pole value equal to the
        // semi-minor axis would mean the meridional radius had been substituted.
        let e = Ellipsoid::WGS84;
        let equatorial = e.prime_vertical_radius_km(Angle::from_deg(0.0));
        assert!((equatorial - 6_378.137).abs() < 1e-9);

        let polar = e.prime_vertical_radius_km(Angle::from_deg(90.0));
        let expected = 6_378.137 / (1.0 - e.eccentricity_squared()).sqrt();
        assert!((polar - expected).abs() < 1e-12);
        assert!(
            (polar - 6_399.593_625_6).abs() < 1e-6,
            "polar N was {polar}"
        );

        // The distinction that matters: N at the pole is not b.
        assert!((polar - e.semi_minor_km()).abs() > 40.0);
    }

    #[test]
    fn the_prime_vertical_radius_is_symmetric_about_the_equator() {
        let e = Ellipsoid::WGS84;
        for lat in [0.0, 10.0, 35.0, 60.0, 80.0, 90.0] {
            let north = e.prime_vertical_radius_km(Angle::from_deg(lat));
            let south = e.prime_vertical_radius_km(Angle::from_deg(-lat));
            assert!((north - south).abs() < 1e-12, "asymmetric at {lat}");
        }
    }
}
