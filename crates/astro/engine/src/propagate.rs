//! From a two-line element set to an Earth-fixed position.
//!
//! This is the join that had no home until this crate existed: `stkcraft-astro-sgp4` produces a TEME
//! state, `stkcraft-astro-frames` converts it, and `stkcraft-astro-access` needs an Earth-fixed position.
//! Neither of the two numerical crates can depend on the other (they are both L2), so the
//! composition belongs here.
//!
//! # Statefulness
//!
//! SGP4 is not a pure function of time. Its deep-space resonance integrator carries `atime`,
//! `xli` and `xni` between calls, and the reference definition restarts all three when a call
//! asks for a time *before* the previous one. That means:
//!
//! - A [`Propagator`] is **not** `Sync`, and must not be shared between threads.
//! - Asking for times out of order is **allowed and correct** — the integrator restarts, as the
//!   reference says it should — but costs an integration from the epoch.
//!
//! The window finder in `stkcraft-astro-access` back-tracks while bisecting, so it pays that cost.
//! It is correct, and for a near-Earth orbit the restart is cheap because there is no
//! resonance to integrate.

use std::sync::Arc;

use stkcraft_astro_access::Ellipsoid;
use stkcraft_astro_frames::{
    CartesianState, EarthOrientation, EquinoxTerms, Finals2000A, teme_to_gcrf, teme_to_itrf,
};
use stkcraft_astro_sgp4::{Elements, Satellite, StateVector, TwoLineElement};
use stkcraft_framework_core::{Error, Result};
use stkcraft_framework_time::Epoch;

/// A satellite prepared to propagate, with its frame context.
#[derive(Debug, Clone)]
pub struct Propagator {
    /// The SGP4 model. `Satellite` is `Clone`, so a propagator can be duplicated cheaply when
    /// a caller needs two independent integrator states.
    satellite: Satellite,
    /// The element set's epoch, for converting absolute instants to offsets.
    epoch: Epoch,
    /// The Earth orientation used when no model is loaded.
    eop: EarthOrientation,
    /// The IERS series, when the caller loaded one. It is queried at each instant, because EOP
    /// changes over a multi-day scenario; a single value at the epoch is not enough.
    eop_model: Option<Arc<Finals2000A>>,
}

impl Propagator {
    /// Prepare a propagator from an element set.
    ///
    /// # Errors
    ///
    /// As [`Satellite::new`]: a sub-orbital or already-decayed element set is refused rather
    /// than producing a position that means nothing.
    pub fn new(tle: &TwoLineElement) -> Result<Self> {
        let elements: Elements = tle.elements()?;
        Ok(Self {
            satellite: Satellite::new(elements)?,
            epoch: tle.epoch,
            // Polar motion and UT1 - UTC are zero unless the caller supplies a model; the answer
            // carries that limitation rather than hiding it.
            eop: EarthOrientation::negligible(),
            eop_model: None,
        })
    }

    /// Use an IERS `finals2000A` series for the Earth-fixed conversion.
    ///
    /// The series is queried at each instant; it takes precedence over a static
    /// [`Self::with_earth_orientation`] value. A date outside the series is an error, not a
    /// silent fallback to zero.
    #[must_use]
    pub fn with_eop_model(mut self, model: Arc<Finals2000A>) -> Self {
        self.eop_model = Some(model);
        self
    }

    /// The element set's epoch.
    #[must_use]
    pub const fn epoch(&self) -> Epoch {
        self.epoch
    }

    /// Use specific Earth-orientation parameters for the Earth-fixed conversion.
    #[must_use]
    pub fn with_earth_orientation(mut self, eop: EarthOrientation) -> Self {
        self.eop = eop;
        self
    }

    /// The minutes between an instant and the element set's epoch.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Invalid`] if the two epochs are on different time scales, because
    /// subtracting them without a conversion would be a silent error of up to a minute.
    fn minutes_from_epoch(&self, at: Epoch) -> Result<f64> {
        let seconds = self.epoch.seconds_until(at)?;
        if !seconds.is_finite() {
            return Err(Error::invalid("propagation time", "must be finite"));
        }
        Ok(seconds / 60.0)
    }

    /// The TEME state at an instant.
    ///
    /// # Errors
    ///
    /// As [`Satellite::propagate`].
    pub fn teme(&mut self, at: Epoch) -> Result<StateVector> {
        let minutes = self.minutes_from_epoch(at)?;
        self.satellite.propagate_minutes(minutes)
    }

    /// The TEME state a number of minutes after the epoch, which is the interface the published
    /// verification file is tabulated in.
    ///
    /// # Errors
    ///
    /// As [`Satellite::propagate_minutes`].
    pub fn teme_minutes(&mut self, minutes: f64) -> Result<StateVector> {
        self.satellite.propagate_minutes(minutes)
    }

    /// The GCRF state at an instant, for the viewport and for inter-satellite geometry.
    ///
    /// # Errors
    ///
    /// As [`Self::teme`] and [`teme_to_gcrf`].
    pub fn gcrf(&mut self, at: Epoch) -> Result<CartesianState> {
        let teme = self.teme(at)?;
        let ttt = at
            .to_scale(stkcraft_framework_time::TimeScale::Tt, None)?
            .julian()
            .centuries_since_j2000();
        teme_to_gcrf(ttt, &cartesian(&teme))
    }

    /// The Earth-fixed position at an instant, km.
    ///
    /// Earth orientation parameters are applied, but only those this propagator was given;
    /// see [`Self::with_earth_orientation`].
    ///
    /// # Errors
    ///
    /// As [`Self::teme`] and [`teme_to_itrf`].
    pub fn itrf(&mut self, at: Epoch) -> Result<[f64; 3]> {
        let state = self.itrf_state(at)?;
        Ok(state.position.map(stkcraft_framework_core::Distance::as_km))
    }

    /// The Earth-fixed state at an instant, km and km/s.
    ///
    /// # Errors
    ///
    /// As [`Self::teme`] and [`teme_to_itrf`].
    pub fn itrf_state(&mut self, at: Epoch) -> Result<CartesianState> {
        let teme = self.teme(at)?;
        let ttt = at
            .to_scale(stkcraft_framework_time::TimeScale::Tt, None)?
            .julian()
            .centuries_since_j2000();
        // The IERS series is indexed by UTC. Converting first matters: a TDB epoch used directly
        // would shift the EOP lookup by the periodic term.
        let utc = at.to_utc()?;
        let eop = if let Some(model) = &self.eop_model {
            model.at(utc.julian().to_mjd())?
        } else {
            self.eop
        };
        // The Earth orientation parameters carry their own UT1 offset, so the UT1 Julian Date
        // is the UTC one plus `dut1`. Passing the UTC date here would silently ignore the
        // offset the caller supplied.
        let utc_jd = utc.julian().sum();
        let jdut1 = utc_jd + eop.dut1_seconds / 86_400.0;
        teme_to_itrf(jdut1, ttt, &eop, EquinoxTerms::Kinematic, &cartesian(&teme))
    }

    /// The element set this propagator was built from.
    #[must_use]
    pub fn is_deep_space(&self) -> bool {
        self.satellite.is_deep_space()
    }
}

/// Convert a TEME state into the frame crate's state type.
///
/// The two types exist separately because `stkcraft-astro-sgp4` and `stkcraft-astro-frames` are on the same
/// layer and cannot depend on each other. The conversion is total: both are three-component
/// kilometre-and-kilometre-per-second states.
fn cartesian(state: &StateVector) -> CartesianState {
    CartesianState {
        position: state.position,
        velocity: state.velocity,
    }
}

/// The Earth model the engine uses everywhere.
///
/// WGS-84, because it is the ellipsoid the GPS constellation and every published station
/// coordinate are defined in.
pub const EARTH: Ellipsoid = Ellipsoid::WGS84;

#[cfg(test)]
#[expect(clippy::expect_used)]
mod tests {
    use stkcraft_astro_sgp4::TwoLineElement;
    use stkcraft_framework_core::{Angle, Distance};

    use super::*;

    /// The paper's canonical near-Earth verification set.
    const L1: &str = "1 00005U 58002B   00179.78495062  .00000023  00000-0  28098-4 0  4753";
    const L2: &str = "2 00005  34.2682 348.7242 1859667 331.7664  19.3264 10.82419157413667";

    /// A geostationary element set, for the frame-chain sanity check.
    const GEO_L1: &str = "1 13552U 82069A   04166.50000000  .00000000  00000-0  00000+0 0  9992";
    const GEO_L2: &str = "2 13552   0.0207  90.3015 0002000 130.0000 230.0000  1.00273791 80004";

    fn propagator() -> Propagator {
        let tle = TwoLineElement::parse(L1, L2).expect("valid TLE");
        Propagator::new(&tle).expect("initialises")
    }

    #[test]
    fn a_near_earth_element_set_reproduces_its_published_teme_state() {
        // The published tforverf.out state for case 00005 at t = 0, through the engine but
        // before any frame conversion, so this checks the join rather than the model.
        let mut p = propagator();
        let state = p.teme_minutes(0.0).expect("propagates");
        assert!((state.position[0].as_km() - 7_022.465_292_66).abs() < 1e-6);
        assert!((state.position[1].as_km() - -1_400.082_967_55).abs() < 1e-6);
        assert!((state.position[2].as_km() - 0.039_951_55).abs() < 1e-6);
    }

    #[test]
    fn the_gcrf_and_itrf_radii_both_match_teme() {
        // Both conversions are rotations, so the radius must survive them. This catches a
        // frame chain wired up wrongly without needing a reference table.
        let mut p = propagator();
        // Stay on the element set's own scale: `seconds_until` refuses to subtract across
        // scales, which is the right default and is what caught this test when it tried.
        let epoch_plus = stkcraft_framework_time::Epoch::from_julian(
            p.epoch().julian().plus_seconds(600.0),
            p.epoch().scale(),
        );

        let teme = p.teme(epoch_plus).expect("propagates");
        let teme_radius: f64 = teme
            .position
            .iter()
            .map(|d| d.as_km().powi(2))
            .sum::<f64>()
            .sqrt();

        let gcrf = p.gcrf(epoch_plus).expect("converts");
        assert!((gcrf.radius() - teme_radius).abs() < 1e-7);

        let itrf = p.itrf_state(epoch_plus).expect("converts");
        assert!((itrf.radius() - teme_radius).abs() < 1e-7);
    }

    #[test]
    fn an_iers_series_supplies_earth_orientation_per_instant() {
        let text = "\
00 6 27 51722.00 I  0.0 0.0  0.0 0.0  I 0.9000000 0.0  0.0 0.0  P
00 6 28 51723.00 I  0.0 0.0  0.0 0.0  I 0.0000000 0.0  0.0 0.0  P
";
        let model = Arc::new(Finals2000A::parse(text).expect("two EOP rows"));
        let tle = TwoLineElement::parse(L1, L2).expect("valid TLE");
        let mut zero = Propagator::new(&tle).expect("initialises");
        let mut with_eop = Propagator::new(&tle)
            .expect("initialises")
            .with_eop_model(model);
        let at = tle.epoch;
        let a = zero.itrf_state(at).expect("zero-EOP conversion");
        let b = with_eop.itrf_state(at).expect("IERS conversion");
        let mut delta = 0.0;
        for (left, right) in a.position.iter().zip(b.position.iter()) {
            delta += (left.as_km() - right.as_km()).powi(2);
        }
        assert!(
            delta.sqrt() > 0.01,
            "the EOP series made no difference: {} km",
            delta.sqrt()
        );
    }

    #[test]
    fn a_geostationary_satellite_stays_near_one_earth_fixed_point() {
        // The end-to-end sanity check for the frame chain and sidereal time together. A
        // geostationary satellite is fixed over the equator, so its Earth-fixed position over
        // a day must stay within a few tens of kilometres. A sign error in the sidereal time
        // would send it round the Equator instead.
        let tle = TwoLineElement::parse(GEO_L1, GEO_L2).expect("valid TLE");
        let mut p = Propagator::new(&tle).expect("initialises");

        let start = p.epoch();
        let first = p.itrf(start).expect("propagates");
        let mut worst = 0.0_f64;
        for hour in 1..=24 {
            let minutes = f64::from(hour) * 60.0;
            let at = stkcraft_framework_time::Epoch::from_julian(
                p.epoch().julian().plus_days(minutes / 1_440.0),
                p.epoch().scale(),
            );
            let position = p.itrf(at).expect("propagates");
            let mut delta = 0.0;
            for (a, b) in first.iter().zip(position.iter()) {
                delta += (a - b) * (a - b);
            }
            worst = worst.max(delta.sqrt());
        }
        assert!(
            worst < 100.0,
            "a geostationary satellite drifted {worst} km in the Earth-fixed frame"
        );
    }

    #[test]
    fn asking_for_times_out_of_order_gives_the_same_answer() {
        // SGP4's resonance integrator restarts when the time goes backwards; the answer must
        // not depend on call order, and the window finder depends on that.
        let mut forwards = propagator();
        let mut backwards = propagator();

        let forward_result = forwards.teme_minutes(600.0).expect("propagates");
        let _ = backwards.teme_minutes(1_200.0).expect("propagates");
        let _ = backwards.teme_minutes(900.0).expect("propagates");
        let backward_result = backwards.teme_minutes(600.0).expect("propagates");

        for (a, b) in forward_result
            .position
            .iter()
            .zip(backward_result.position.iter())
        {
            assert!((a.as_km() - b.as_km()).abs() < 1e-9);
        }
    }

    #[test]
    fn a_decayed_element_set_reports_an_error_rather_than_a_position() {
        // Case 29141 from the published verification set: the last element set issued for a
        // decaying object, lost within 420 minutes. The engine must pass the failure on rather
        // than inventing a position.
        let l1 = "1 29141U 85108AA  06170.26783845  .99999999  00000-0  13519-0 0   718";
        let l2 = "2 29141  82.4288 273.4882 0015848 277.2124  83.9133 15.93343074  6828";
        let tle = TwoLineElement::parse(l1, l2).expect("valid TLE");
        let mut p = Propagator::new(&tle).expect("initialises");

        let mut failures = 0;
        for step in 0..=60 {
            let minutes = f64::from(step) * 20.0;
            if p.teme_minutes(minutes).is_err() {
                failures += 1;
            }
        }
        assert!(failures > 0, "a decaying element set never failed");
    }

    #[test]
    fn a_sub_orbital_element_set_is_refused_at_construction() {
        let l1 = "1 28872U 05037B   05333.02012661  .25992681  00000-0  24476-3 0   151";
        let l2 = "2 28872  96.4736 157.9986 0303955 244.0492 110.6523 16.46015936  5373";
        let tle = TwoLineElement::parse(l1, l2).expect("valid TLE");
        assert!(Propagator::new(&tle).is_err());
    }

    #[test]
    fn the_site_and_satellite_are_measurable_against_each_other() {
        // A smoke test that the pieces compose: place a station and a low satellite and confirm
        // the elevation is a real number in range. The full access path has its own tests.
        use stkcraft_astro_access::{GroundSite, Sensor, look_angle};

        let mut p = propagator();
        let at = p.epoch();
        let position = p.itrf(at).expect("propagates");

        let site =
            GroundSite::new(Angle::from_deg(0.0), Angle::from_deg(0.0), 0.0).expect("valid site");
        let site_position = site.to_itrf(&EARTH).expect("valid");
        let look = look_angle(&site.local_frame(), site_position, position).expect("non-zero");
        assert!(look.elevation.as_deg() >= -90.0 && look.elevation.as_deg() <= 90.0);
        assert!(look.range.as_km() > 200.0);
        let _ = Sensor::above_horizon(Angle::from_deg(0.0));
        let _ = Distance::from_km(1.0);
    }
}
