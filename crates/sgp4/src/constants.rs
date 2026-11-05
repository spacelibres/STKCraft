//! The constants SGP4 is defined in terms of.
//!
//! SGP4 is not a general force model; it is a fitted theory whose coefficients belong to a
//! specific gravity field, and the choice of field changes the answer in the last digits.
//! Two fields are in use:
//!
//! - **WGS-72**, the one the element sets were generated with and the one this crate uses.
//! - **WGS-84**, supplied by the reference implementation for comparison only.
//!
//! Mixing them is a classic error that produces a plausible orbit offset by a few hundred
//! metres, so the field travels with the model rather than being a global.

/// The gravity field an SGP4 run is defined in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub enum GravityField {
    /// WGS-72, the field the NORAD element sets are consistent with.
    ///
    /// This is the correct choice for any real element set and it is the default.
    #[default]
    Wgs72,
    /// WGS-84, available because the reference implementation exposes it.
    ///
    /// Use it only to reproduce a comparison run. An operational element set propagated
    /// with WGS-84 is subtly wrong.
    Wgs84,
}

/// The constants of one gravity field, in the units SGP4 works in.
///
/// SGP4 is written in canonical units: distance in Earth radii, time in minutes, angle in
/// radians. Keeping the raw reciprocal-length constants (`xke`, `j2`, …) rather than
/// converting to metres keeps this crate's arithmetic identical in structure to the
/// reference implementation, which is what makes a bit-for-bit tolerance comparable.
#[derive(Debug, Clone, Copy)]
pub struct GravityConstants {
    /// Minutes per canonical time unit, `sqrt(Re^3 / mu)`.
    pub tumin: f64,
    /// The gravitational parameter, km³/s².
    pub mu: f64,
    /// The equatorial Earth radius, km.
    pub radius_earth_km: f64,
    /// `60 / sqrt(Re³ / mu)`, the reciprocal of the canonical length unit per minute.
    pub xke: f64,
    /// The second zonal harmonic, dimensionless.
    pub j2: f64,
    /// The third zonal harmonic, dimensionless.
    pub j3: f64,
    /// The fourth zonal harmonic, dimensionless.
    pub j4: f64,
    /// `j3 / j2`, which appears on its own in several coefficients.
    pub j3_over_j2: f64,
    /// Kilometres per second per canonical velocity unit, `Re * xke / 60`.
    pub vkmpersec: f64,
}

/// WGS-72 constants.
///
/// Source: AIAA 2006-6753 `getgravconst`, WGS-72 branch. The values are the standard WGS-72
/// set — `Re = 6378.135 km`, `mu = 398600.8 km³/s²`, `J2 = 0.001082616`,
/// `J3 = -0.00000253881`, `J4 = -0.00000165597` — with the derived quantities computed the
/// way the reference implementation computes them so the rounding matches.
pub const WGS72: GravityConstants = GravityConstants {
    tumin: 806.810_381_817_558_6,
    mu: 398_600.8,
    radius_earth_km: 6_378.135,
    xke: 0.074_366_916_133_173_42,
    j2: 0.001_082_616,
    j3: -0.000_002_538_81,
    j4: -0.000_001_655_97,
    j3_over_j2: -0.002_345_069_720_011_528,
    vkmpersec: 7.905_370_510_517_634,
};

/// WGS-84 constants.
///
/// Source: AIAA 2006-6753 `getgravconst`, WGS-84 branch. Present for reproducing comparison
/// runs only; see [`GravityField`].
pub const WGS84: GravityConstants = GravityConstants {
    tumin: 13.446_851_049_667_486,
    mu: 398_600.5,
    radius_earth_km: 6_378.137,
    xke: 0.074_366_877_315_620_71,
    j2: 0.001_082_629_989_3,
    j3: -0.000_002_532_153_249_833_5,
    j4: -0.000_001_610_988_923_359_852_7,
    j3_over_j2: -0.002_339_565_025_023_434_6,
    vkmpersec: 7.905_370_995_154_893,
};

impl GravityField {
    /// The constants of this field.
    #[must_use]
    pub const fn constants(self) -> GravityConstants {
        match self {
            Self::Wgs72 => WGS72,
            Self::Wgs84 => WGS84,
        }
    }
}

// -------------------------------------------------------------------------------------------
// Model thresholds. These are part of the theory, not tuning knobs: changing one changes the
// answer for a whole class of orbits, so each carries the condition it appears under.
// -------------------------------------------------------------------------------------------

/// Atmospheric model constants that depend on the perigee height, in km.
///
/// Below 156 km the drag model lowers its `s` and `q0` parameters; below 98 km it holds `s`
/// at 20 km. Source: AIAA 2006-6753 `sgp4init`, the `perige < 156` branch.
pub const PERIGEE_HEIGHT_LIMIT_KM: f64 = 156.0;

/// Below this perigee height the atmospheric parameters are held at their floor.
///
/// Source: AIAA 2006-6753 `sgp4init`, the `perige <= 98` branch.
pub const PERIGEE_HEIGHT_FLOOR_KM: f64 = 98.0;

/// The floor value of the atmospheric parameter `s`, in km.
///
/// Source: AIAA 2006-6753 `sgp4init`.
pub const ATMOSPHERE_S_FLOOR_KM: f64 = 20.0;

/// The `q0` reference height used in the atmospheric parameter `qzms2t`, in km.
///
/// Source: AIAA 2006-6753 `sgp4init`.
pub const ATMOSPHERE_Q0_KM: f64 = 120.0;

/// Below this perigee height the simplified drag series is used.
///
/// In canonical units: `220 km / Re + 1`. Source: AIAA 2006-6753 `sgp4init`, the `isimp`
/// branch.
pub const SIMPLIFIED_DRAG_RADIUS: f64 = 1.0 + 220.0 / 6_378.135;

/// Orbits with a period at or above this many minutes are deep space and use SDP4.
///
/// Source: AIAA 2006-6753 `sgp4init`, `if ((twopi / no) >= 225.0)`.
pub const DEEP_SPACE_PERIOD_MINUTES: f64 = 225.0;

/// One-day resonance band, lower mean motion bound, rad/min.
///
/// Source: AIAA 2006-6753 `dsinit`, the `irez = 1` test.
pub const SYNCHRONOUS_MEAN_MOTION_MIN: f64 = 3.490_658_5e-3;
/// One-day resonance band, upper mean motion bound, rad/min.
pub const SYNCHRONOUS_MEAN_MOTION_MAX: f64 = 5.235_987_7e-3;

/// Half-day resonance band, lower mean motion bound, rad/min.
///
/// Source: AIAA 2006-6753 `dsinit`, the `irez = 2` test.
pub const HALF_DAY_MEAN_MOTION_MIN: f64 = 8.26e-3;
/// Half-day resonance band, upper mean motion bound, rad/min.
pub const HALF_DAY_MEAN_MOTION_MAX: f64 = 9.24e-3;
/// Half-day resonance additionally requires at least this eccentricity.
pub const HALF_DAY_ECCENTRICITY_MIN: f64 = 0.5;

/// The Lyddane modification is used when the perturbed inclination is below this, in rad.
///
/// Source: AIAA 2006-6753 `dpper`; the comment there notes this is 11.45916°.
pub const LYDDANE_INCLINATION_RAD: f64 = 0.2;

/// The reciprocal of the Earth rotation rate used by the deep-space resonance integrator,
/// rad/min (`4.37526908801129966e-3` rad/min ≈ `7.29211514668855e-5` rad/s).
///
/// Source: AIAA 2006-6753 `dsinit` and `dspace`, constant `rptim`.
pub const EARTH_ROTATION_RAD_PER_MIN: f64 = 4.375_269_088_011_3e-3;

/// The resonance integrator's Euler-Maclaurin step, minutes.
///
/// Source: AIAA 2006-6753 `dspace`, constant `stepp`.
pub const RESONANCE_STEP_MINUTES: f64 = 720.0;

/// Half the square of the resonance step, with its sign taken from the step direction.
///
/// Source: AIAA 2006-6753 `dspace`, constant `step2`, used as `step2 = 259200.0` for a
/// positive step and `-259200.0` for a negative one.
pub const RESONANCE_STEP2: f64 = 259_200.0;

/// Minutes per day, used to convert element-set epochs.
pub const MINUTES_PER_DAY: f64 = 1_440.0;

/// Minutes per revolution, `1440 / (2 * pi)`, which is the reference implementation's
/// `xpdotp`.
///
/// A mean motion is converted by dividing by this rather than by multiplying by
/// `2 * pi / 1440`. The two expressions differ by one unit in the last place, and that is
/// visible in the position at the metre level, so the division is used to match the
/// published output rather than for any physical reason.
pub const MINUTES_PER_REVOLUTION: f64 = MINUTES_PER_DAY / (2.0 * core::f64::consts::PI);

/// Day number of the reference implementation's `epoch` zero, 1950 January 0.0.
///
/// Its `epoch` variable counts days from this instant: `twoline2rv` computes
/// `jdsatepoch - 2433281.5`. That scalar is JD 2433281.5 = 1949 December 31 12:00, so the
/// zero sits half a day into day 2433281. Splitting it into the integral day and the half
/// lets the day difference be taken exactly; the half is added back to the fraction.
pub const JD_1950_JAN_0_DAY: f64 = 2_433_281.0;

/// The fractional day of the 1950 January 0.0 zero within [`JD_1950_JAN_0_DAY`].
pub const JD_1950_JAN_0_FRACTION: f64 = 0.5;

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
mod tests {
    use super::*;

    #[test]
    fn wgs72_constants_are_the_published_set() {
        // WGS-72 values, AIAA 2006-6753 `getgravconst`.
        assert_eq!(WGS72.radius_earth_km, 6_378.135);
        assert_eq!(WGS72.mu, 398_600.8);
        assert_eq!(WGS72.j2, 0.001_082_616);
        assert!((WGS72.xke - 0.074_366_916_133_173_42).abs() < 1e-18);
    }

    #[test]
    fn derived_wgs72_quantities_agree_with_their_definitions() {
        // `xke = 60 / sqrt(Re^3 / mu)` and `vkmpersec = Re * xke / 60` are definitions, not
        // fitted values, so a transcription slip shows up here.
        let g = WGS72;
        let tumin = (g.radius_earth_km.powi(3) / g.mu).sqrt();
        assert!(
            (g.tumin - tumin).abs() < 1e-12,
            "tumin {} vs {}",
            g.tumin,
            tumin
        );
        let xke = 60.0 / tumin;
        assert!((g.xke - xke).abs() < 1e-12, "xke {} vs {}", g.xke, xke);
        let vkm = g.radius_earth_km * g.xke / 60.0;
        assert!(
            (g.vkmpersec - vkm).abs() < 1e-12,
            "vkmpersec {} vs {}",
            g.vkmpersec,
            vkm
        );
        assert!((g.j3_over_j2 - g.j3 / g.j2).abs() < 1e-15);
    }

    #[test]
    fn the_1950_epoch_is_the_documented_julian_date() {
        // 2433281.5: the reference implementation subtracts exactly this.
        assert_eq!(JD_1950_JAN_0_DAY + JD_1950_JAN_0_FRACTION, 2_433_281.5);
        assert_eq!(
            JD_1950_JAN_0_DAY.fract(),
            0.0,
            "the day part must be integral"
        );
    }

    #[test]
    fn gravity_field_defaults_to_wgs72() {
        assert_eq!(GravityField::default().constants().mu, WGS72.mu);
    }
}
