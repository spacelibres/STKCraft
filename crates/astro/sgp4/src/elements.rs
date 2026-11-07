//! The mean elements SGP4 is initialized from.
//!
//! A TLE states its angles in degrees, its mean motion in revolutions per day, and its epoch
//! as a day of year. SGP4 is written in canonical units — radians, minutes, Earth radii — so
//! exactly one conversion happens, here, and nowhere else. Keeping that boundary in one
//! place is why the propagator takes [`Elements`] rather than a TLE.
//!
//! The epoch convention deserves the note it gets on [`Elements::epoch`]: SGP4 does not use
//! the full date. It uses the fraction of a day (for the inertial orientation of the orbit)
//! and the day count since 1950 (for the lunar and solar arguments), which is why the
//! propagation result is unaffected by a leap second.

use serde::{Deserialize, Serialize};
use stkcraft_framework_core::{Angle, Error, Result};
use stkcraft_framework_time::{Epoch, JulianDate};

use crate::constants::{GravityField, MINUTES_PER_REVOLUTION};

/// The mean elements of an element set, in the units SGP4 is defined in.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Elements {
    /// The instant these elements describe.
    ///
    /// SGP4 uses two derived quantities from it: the fraction of a day, which sets the
    /// orientation of the orbit in inertial space, and the number of days since 1950
    /// January 0.0, which sets the lunar and solar arguments. A whole-second shift in the
    /// epoch therefore changes the answer by about the rotation of the Earth in a second,
    /// which is the correct behaviour and not a bug.
    pub epoch: Epoch,
    /// Eccentricity, dimensionless, `0 <= e < 1`.
    pub eccentricity: f64,
    /// Inclination, `0 <= i <= π`.
    pub inclination: Angle,
    /// Right ascension of the ascending node, wrapped to `[0, 2π)`.
    pub right_ascension: Angle,
    /// Argument of perigee, wrapped to `[0, 2π)`.
    pub argument_of_perigee: Angle,
    /// Mean anomaly at epoch, wrapped to `[0, 2π)`.
    pub mean_anomaly: Angle,
    /// Mean motion, rad/min, in the "Kozai" sense the TLE states it.
    ///
    /// The TLE's `n` is a Kozai mean motion, which is not the same as the mean motion of the
    /// `j2`-corrected ellipse; `sgp4init` converts it. Calling this field "mean motion" is a
    /// historical compromise; [`Elements::semi_major_axis`] documents the difference.
    pub mean_motion: f64,
    /// The `B*` drag term, in units of 1 / Earth radii.
    pub bstar: f64,
    /// The fraction of the epoch day, `0 <= f < 1`, as the element set states it.
    fraction_of_day: f64,
    /// The gravity field this element set is to be propagated in.
    ///
    /// Always [`GravityField::Wgs72`] for a real element set; see the type's documentation.
    pub gravity: GravityField,
}

impl Elements {
    /// Build elements from TLE-native units: degrees, revolutions per day.
    ///
    /// This is the single conversion point between the TLE's units and SGP4's.
    ///
    /// # Errors
    ///
    /// Returns [`Error::OutOfRange`] for an eccentricity outside `[0, 1)`, an inclination
    /// outside `[0, π]`, or a non-positive mean motion — all of which a TLE can express and
    /// none of which describes an orbit.
    #[allow(clippy::too_many_arguments)]
    pub fn from_tle(
        epoch: Epoch,
        eccentricity: f64,
        inclination: Angle,
        right_ascension: Angle,
        argument_of_perigee: Angle,
        mean_anomaly: Angle,
        mean_motion_rev_per_day: f64,
        bstar: f64,
    ) -> Result<Self> {
        if !(0.0..1.0).contains(&eccentricity) {
            return Err(Error::out_of_range(
                "eccentricity",
                eccentricity,
                "0 <= e < 1",
            ));
        }
        if eccentricity == 0.0 {
            // A perfectly circular orbit is representable — the propagator floors the
            // eccentricity at 1e-6 itself — so this is not an error, only a note that the
            // argument of perigee is undefined. It is accepted deliberately.
        }
        let incl = inclination.as_rad();
        if !(0.0..=core::f64::consts::PI).contains(&incl) {
            return Err(Error::out_of_range(
                "inclination",
                inclination.as_deg(),
                "0 <= i <= 180 deg",
            ));
        }
        // A non-finite mean motion or drag term passes every `<=` comparison, so it is refused
        // before the conversion rather than becoming a NaN state vector downstream. The angles
        // are checked the same way: `Angle::from_deg(NaN)` is a valid `Angle` value.
        if !mean_motion_rev_per_day.is_finite() {
            return Err(Error::invalid("mean motion", "must be a finite number"));
        }
        if mean_motion_rev_per_day <= 0.0 {
            return Err(Error::out_of_range(
                "mean motion",
                mean_motion_rev_per_day,
                "greater than zero",
            ));
        }
        if !bstar.is_finite() {
            return Err(Error::invalid("bstar", "must be a finite number"));
        }
        for (what, angle) in [
            ("right ascension", right_ascension),
            ("argument of perigee", argument_of_perigee),
            ("mean anomaly", mean_anomaly),
        ] {
            if !angle.as_deg().is_finite() {
                return Err(Error::invalid(what, "must be a finite number"));
            }
        }
        // rev/day -> rad/min. The reference implementation divides by
        // `xpdotp = 1440 / (2 * pi)` rather than multiplying by `2 * pi / 1440`, and the two
        // differ by one unit in the last place. That is worth one to two metres of position
        // by the time it reaches the state vector, so the division is mirrored exactly.
        let mean_motion = mean_motion_rev_per_day / MINUTES_PER_REVOLUTION;
        Ok(Self {
            epoch,
            eccentricity,
            inclination: Angle::from_rad(incl),
            right_ascension,
            argument_of_perigee,
            mean_anomaly,
            mean_motion,
            bstar,
            fraction_of_day: 0.0,
            gravity: GravityField::Wgs72,
        })
    }

    /// The epoch expressed as the two numbers SGP4 actually uses.
    ///
    /// Returns the fraction of a day since the epoch's midnight in `[0, 1)`, and the number
    /// of whole days from 1950 January 0.0 to that same midnight.
    ///
    /// The reference implementation computes both from a single `epoch` variable that counts
    /// days from 1950 January 0.0, and takes the fraction with `epoch - trunc(epoch)`. The two
    /// numbers added together are that `epoch`, which is what `dscom` wants; the sidereal-time
    /// routine wants them apart. Returning them apart keeps both exact.
    #[must_use]
    pub fn sgp4_epoch(self) -> (f64, f64) {
        let jd = self.epoch.julian();
        let whole_days = jd.day - crate::constants::JD_1950_JAN_0_DAY;
        (self.fraction_of_day, whole_days)
    }

    /// The semi-major axis implied by the un-Kozai'd mean motion, in Earth radii.
    ///
    /// This is the value `sgp4init` computes as `a`, not the TLE's nominal `a` from the raw
    /// Kozai mean motion. The two differ by roughly `1e-3` relative for a low-Earth orbit.
    #[must_use]
    pub fn semi_major_axis(self, gravity: GravityField) -> f64 {
        let g = gravity.constants();
        let unkozai = self.unkozai_mean_motion(gravity);
        (g.xke / unkozai).powf(2.0 / 3.0)
    }

    /// The mean motion after the Kozai correction, rad/min.
    ///
    /// Source: AIAA 2006-6753 `initl`, which solves `n = n0(1 + del)` iteratively once.
    #[must_use]
    pub fn unkozai_mean_motion(self, gravity: GravityField) -> f64 {
        let g = gravity.constants();
        let no = self.mean_motion;
        let cosio2 = self.inclination.as_rad().cos().powi(2);
        let omeosq = 1.0 - self.eccentricity * self.eccentricity;
        if omeosq <= 0.0 || no <= 0.0 {
            return no;
        }
        let rteosq = omeosq.sqrt();
        let ak = (g.xke / no).powf(2.0 / 3.0);
        let d1 = 0.75 * g.j2 * (3.0 * cosio2 - 1.0) / (rteosq * omeosq);
        let del = d1 / (ak * ak);
        let adel = ak * (1.0 - del * del - del * (1.0 / 3.0 + 134.0 * del * del / 81.0));
        let del = d1 / (adel * adel);
        no / (1.0 + del)
    }

    /// The Julian Date of the epoch.
    #[must_use]
    pub fn julian(self) -> JulianDate {
        self.epoch.julian()
    }

    /// Record the fraction of the day the element set was written with.
    ///
    /// A TLE states its epoch as a day-of-year fraction; that fraction is what SGP4's
    /// trigonometric arguments need, and recovering it from a Julian Date would cost the
    /// last two digits. `from_tle` sets it to zero, so a caller building elements from a
    /// decoded date must call this.
    #[must_use]
    pub fn with_fraction_of_day(mut self, fraction: f64) -> Self {
        self.fraction_of_day = fraction;
        self
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
mod tests {
    use super::*;
    use crate::tle::TwoLineElement;
    use stkcraft_framework_time::TimeScale;

    fn elements_of(line1: &str, line2: &str) -> Elements {
        TwoLineElement::parse(line1, line2)
            .expect("test TLE parses")
            .elements()
            .expect("test elements are valid")
    }

    const L1: &str = "1 00005U 58002B   00179.78495062  .00000023  00000-0  28098-4 0  4753";
    const L2: &str = "2 00005  34.2682 348.7242 1859667 331.7664  19.3264 10.82419157413667     ";

    #[test]
    fn mean_motion_converts_from_revolutions_per_day_to_radians_per_minute() {
        let e = elements_of(L1, L2);
        let expected = 10.824_191_57 * core::f64::consts::TAU / 1_440.0;
        assert!((e.mean_motion - expected).abs() < 1e-15);
    }

    #[test]
    fn angles_convert_from_degrees_and_stay_in_radians() {
        let e = elements_of(L1, L2);
        assert!((e.inclination.as_deg() - 34.2682).abs() < 1e-12);
        assert!((e.inclination.as_rad() - 34.2682_f64.to_radians()).abs() < 1e-15);
    }

    #[test]
    fn an_eccentricity_of_one_or_more_is_refused() {
        let epoch = Epoch::from_jd(2_451_545.0, TimeScale::Tai);
        let err = Elements::from_tle(
            epoch,
            1.0,
            Angle::from_deg(10.0),
            Angle::ZERO,
            Angle::ZERO,
            Angle::ZERO,
            14.0,
            0.0,
        )
        .expect_err("parabolic orbits are not SGP4 elements");
        assert!(err.to_string().contains("eccentricity"), "{err}");
    }

    #[test]
    fn an_inclination_above_180_degrees_is_refused() {
        let epoch = Epoch::from_jd(2_451_545.0, TimeScale::Tai);
        let err = Elements::from_tle(
            epoch,
            0.1,
            Angle::from_deg(181.0),
            Angle::ZERO,
            Angle::ZERO,
            Angle::ZERO,
            14.0,
            0.0,
        )
        .expect_err("retrograde past 180 degrees is not a distinct orbit");
        assert!(err.to_string().contains("inclination"), "{err}");
    }

    #[test]
    fn a_non_positive_mean_motion_is_refused() {
        let epoch = Epoch::from_jd(2_451_545.0, TimeScale::Tai);
        let err = Elements::from_tle(
            epoch,
            0.1,
            Angle::from_deg(10.0),
            Angle::ZERO,
            Angle::ZERO,
            Angle::ZERO,
            0.0,
            0.0,
        )
        .expect_err("zero mean motion is not an orbit");
        assert!(err.to_string().contains("mean motion"), "{err}");
    }

    #[test]
    fn the_sgp4_epoch_fraction_matches_the_tle_day_fraction() {
        // TLE day 179.78495062 must give a fraction-of-day of exactly 0.78495062, because the
        // reference implementation's trigonometric arguments depend on it.
        let e = elements_of(L1, L2);
        let (fraction, _) = e.sgp4_epoch();
        assert!(
            (fraction - 0.784_950_62).abs() < 1e-9,
            "fraction of day was {fraction}"
        );
    }

    #[test]
    fn the_unkozai_mean_motion_differs_from_the_stated_one() {
        // The correction is small but not zero, which is the whole reason SGP4 needs it.
        let e = elements_of(L1, L2);
        let unkozai = e.unkozai_mean_motion(GravityField::Wgs72);
        let relative = (unkozai - e.mean_motion) / e.mean_motion;
        assert!(relative.abs() > 1e-5, "correction was {relative}");
        assert!(
            relative.abs() < 1e-2,
            "correction was suspiciously large: {relative}"
        );
    }

    #[test]
    fn the_sgp4_epoch_counts_whole_days_from_the_1950_zero() {
        // Regression: `sgp4_epoch` once returned only the fraction of a day, because the whole
        // day count sat in a variable nobody read. `dscom` builds `day` from it, so the lunar
        // and solar arguments lost their multi-decade ramp and deep-space positions drifted by
        // tens of kilometres. Checked against a hand computation rather than against the
        // function itself.
        //
        // 2000 day 179.78495062 is 2000-06-28, which is JD 2451722.5 at midnight. The SGP4
        // zero is JD 2433281.5, so the whole-day difference is 2451722 - 2433281 = 18441.
        let (fraction, whole_days) = elements_of(L1, L2).sgp4_epoch();
        assert_eq!(
            whole_days, 18_441.0,
            "whole day count from 1950 January 0.0"
        );
        assert!(
            (fraction - 0.784_950_62).abs() < 1e-9,
            "fraction of day was {fraction}"
        );
        // The two together are the reference implementation's `epoch` argument.
        assert!(
            ((whole_days + fraction) - 18_441.784_950_62).abs() < 1e-6,
            "epoch was {}",
            whole_days + fraction
        );
    }
}
