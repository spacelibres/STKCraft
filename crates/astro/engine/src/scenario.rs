//! The scenario document: what the user is analysing.
//!
//! A scenario is **data, not behaviour**. Satellites carry their element set; stations carry
//! their geodetic position and sensor. Nothing here knows how to propagate, which keeps the
//! document honest: a saved scenario never changes meaning when the physics improves, it only
//! produces different numbers — and that is the correct outcome.

use serde::{Deserialize, Serialize};
use stkcraft_astro_access::{GroundSite, Sensor};
use stkcraft_astro_body::Body;
use stkcraft_astro_sgp4::TwoLineElement;
use stkcraft_framework_core::{Error, Result};
use stkcraft_framework_time::Epoch;

/// The central body a scenario without one is about.
const fn default_central_body() -> Body {
    Body::Earth
}

/// A satellite's identifier within one scenario.
///
/// A newtype rather than a bare `u32` so a station id cannot be passed where a satellite id is
/// expected. The ids are assigned by the scenario and are stable for the life of the document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SatelliteId(pub u32);

/// A ground station's identifier within one scenario.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct StationId(pub u32);

/// One satellite in the scenario.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Satellite {
    /// The assigned identifier.
    pub id: SatelliteId,
    /// A display name, which the user chooses and which need not be unique.
    pub name: String,
    /// The element set. This is the source of truth for the orbit: everything else about the
    /// satellite is derived from it, so editing the orbit means replacing this.
    pub element_set: TwoLineElement,
    /// A display colour, so the viewport can tell satellites apart without inventing one.
    pub color: [u8; 3],
}

/// One ground station in the scenario.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Station {
    /// The assigned identifier.
    pub id: StationId,
    /// A display name.
    pub name: String,
    /// Where the station is.
    pub site: GroundSite,
    /// What the station can see.
    pub sensor: Sensor,
}

/// A complete analysis: the objects, and the interval they are analysed over.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Scenario {
    /// The scenario's name.
    pub name: String,
    /// The body the analysis is about: the centre every orbit and every site is measured from.
    ///
    /// Earth unless the document says otherwise, so every scenario written before this field
    /// existed still loads and still means what it meant.
    #[serde(default = "default_central_body")]
    pub central_body: Body,
    /// The satellites.
    pub satellites: Vec<Satellite>,
    /// The ground stations.
    pub stations: Vec<Station>,
    /// The instant the timeline starts.
    pub epoch: Epoch,
    /// How long the timeline runs, in seconds.
    ///
    /// Seconds rather than a `Duration` in the document so a hand-written JSON file reads
    /// naturally; the API converts once at the boundary.
    pub duration_seconds: f64,
    /// The next satellite id to hand out.
    ///
    /// Monotonic, and never derived from the current contents: taking `max(existing) + 1`
    /// would reuse the id of a removed satellite, silently rebinding any reference a caller
    /// still holds. Ids therefore grow for the life of the document even as objects come and go.
    #[serde(default)]
    pub next_satellite: u32,
    /// The next station id to hand out. Monotonic for the same reason.
    #[serde(default)]
    pub next_station: u32,
}

impl Scenario {
    /// An empty scenario starting now.
    ///
    /// "Now" is the current UTC instant, which is the useful default for a new analysis. A
    /// programmatic caller that wants a specific epoch sets it explicitly.
    #[must_use]
    pub fn empty(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            central_body: Body::Earth,
            satellites: Vec::new(),
            stations: Vec::new(),
            // `Epoch::now` cannot fail for a well-formed system clock; when it does, the J2000
            // reference epoch is a valid, honest fallback rather than an uninitialised value.
            epoch: Epoch::now().unwrap_or(Epoch::j2000()),
            duration_seconds: 86_400.0,
            next_satellite: 1,
            next_station: 1,
        }
    }

    /// The body this analysis is measured from.
    #[must_use]
    pub const fn central_body(&self) -> Body {
        self.central_body
    }

    /// Move the analysis to another body.
    ///
    /// The document is not validated here: whether the orbits and sites still make sense about a
    /// different body is a question for [`Self::validate`] and for whoever placed them.
    pub const fn set_central_body(&mut self, body: Body) {
        self.central_body = body;
    }

    /// Check the document is internally consistent.
    ///
    /// # Errors
    ///
    /// - [`Error::OutOfRange`] if the duration is not positive and finite.
    /// - [`Error::Invalid`] if a sensor does not validate, or if two objects share an id.
    pub fn validate(&self) -> Result<()> {
        if !self.duration_seconds.is_finite() || self.duration_seconds <= 0.0 {
            return Err(Error::out_of_range(
                "scenario duration",
                format!("{} s", self.duration_seconds),
                "positive and finite",
            ));
        }

        for station in &self.stations {
            station.sensor.validate()?;
        }

        let mut satellite_ids: Vec<SatelliteId> = self.satellites.iter().map(|s| s.id).collect();
        satellite_ids.sort_unstable();
        satellite_ids.dedup();
        if satellite_ids.len() != self.satellites.len() {
            return Err(Error::invalid(
                "scenario satellites",
                "two satellites share an id",
            ));
        }

        let mut station_ids: Vec<StationId> = self.stations.iter().map(|s| s.id).collect();
        station_ids.sort_unstable();
        station_ids.dedup();
        if station_ids.len() != self.stations.len() {
            return Err(Error::invalid(
                "scenario stations",
                "two stations share an id",
            ));
        }

        Ok(())
    }

    /// The id the next satellite will be given.
    #[must_use]
    pub const fn next_satellite_id(&self) -> SatelliteId {
        SatelliteId(self.next_satellite)
    }

    /// The id the next station will be given.
    #[must_use]
    pub const fn next_station_id(&self) -> StationId {
        StationId(self.next_station)
    }

    /// Add a satellite, assigning the next id.
    ///
    /// # Errors
    ///
    /// Returns [`Error::OutOfRange`] if no id remains, which needs four billion satellites and
    /// is therefore a guard rather than a real case.
    pub fn add_satellite(
        &mut self,
        name: impl Into<String>,
        element_set: TwoLineElement,
        color: [u8; 3],
    ) -> Result<SatelliteId> {
        let id = SatelliteId(self.next_satellite);
        let next = self.next_satellite.checked_add(1).ok_or_else(|| {
            Error::out_of_range(
                "satellite count",
                self.satellites.len(),
                "fewer than 2^32 - 1",
            )
        })?;
        self.next_satellite = next;
        self.satellites.push(Satellite {
            id,
            name: name.into(),
            element_set,
            color,
        });
        Ok(id)
    }

    /// Add a station, assigning the next id.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Invalid`] if the sensor does not validate, so an unusable station can
    /// never enter the document.
    pub fn add_station(
        &mut self,
        name: impl Into<String>,
        site: GroundSite,
        sensor: Sensor,
    ) -> Result<StationId> {
        sensor.validate()?;
        let id = StationId(self.next_station);
        let next = self.next_station.checked_add(1).ok_or_else(|| {
            Error::out_of_range("station count", self.stations.len(), "fewer than 2^32 - 1")
        })?;
        self.next_station = next;
        self.stations.push(Station {
            id,
            name: name.into(),
            site,
            sensor,
        });
        Ok(id)
    }

    /// Find a satellite by id.
    #[must_use]
    pub fn satellite(&self, id: SatelliteId) -> Option<&Satellite> {
        self.satellites.iter().find(|s| s.id == id)
    }

    /// Find a station by id.
    #[must_use]
    pub fn station(&self, id: StationId) -> Option<&Station> {
        self.stations.iter().find(|s| s.id == id)
    }

    /// Remove a satellite, reporting whether it was there.
    pub fn remove_satellite(&mut self, id: SatelliteId) -> bool {
        let before = self.satellites.len();
        self.satellites.retain(|s| s.id != id);
        self.satellites.len() != before
    }

    /// Remove a station, reporting whether it was there.
    pub fn remove_station(&mut self, id: StationId) -> bool {
        let before = self.stations.len();
        self.stations.retain(|s| s.id != id);
        self.stations.len() != before
    }
}

#[cfg(test)]
#[expect(clippy::expect_used)]
mod tests {
    use stkcraft_astro_access::Ellipsoid;
    use stkcraft_framework_core::{Angle, Distance};

    use super::*;

    const ISS: &str = "1 25544U 98067A   24001.50000000  .00016717  00000-0  10270-3 0  9009";
    const ISS_LINE2: &str = "2 25544  51.6400 208.9163 0006317  69.9862  25.2906 15.49569600 12343";

    fn element_set() -> TwoLineElement {
        TwoLineElement::parse(ISS, ISS_LINE2).expect("valid TLE")
    }

    fn site() -> GroundSite {
        GroundSite::new(Angle::from_deg(0.0), Angle::from_deg(0.0), 0.0).expect("valid site")
    }

    #[test]
    fn a_new_scenario_is_empty() {
        let scenario = Scenario::empty("test");
        assert_eq!(scenario.name, "test");
        assert_eq!(scenario.satellites.len(), 0);
        assert_eq!(scenario.stations.len(), 0);
        scenario.validate().expect("an empty scenario is valid");
    }

    #[test]
    fn adding_a_satellite_assigns_a_distinct_id() {
        let mut scenario = Scenario::empty("test");
        let a = scenario
            .add_satellite("A", element_set(), [255, 0, 0])
            .expect("added");
        let b = scenario
            .add_satellite("B", element_set(), [0, 255, 0])
            .expect("added");
        assert_ne!(a, b);
        assert_eq!(scenario.satellite(a).map(|s| s.name.as_str()), Some("A"));
        assert_eq!(scenario.satellite(b).map(|s| s.name.as_str()), Some("B"));
    }

    #[test]
    fn adding_a_station_refuses_an_invalid_sensor() {
        // An unusable station must never reach the document: everything downstream would
        // otherwise have to re-check, and one caller would forget.
        let mut scenario = Scenario::empty("test");
        let bad_sensor = Sensor::above_horizon(Angle::from_deg(120.0));
        assert!(
            scenario.add_station("bad", site(), bad_sensor).is_err(),
            "a 120 degree horizon mask was accepted"
        );
        assert_eq!(scenario.stations.len(), 0);
    }

    #[test]
    fn a_scenario_round_trips_through_json() {
        // The document is the unit of persistence, so everything must survive serialisation:
        // the TLE's epoch and its implied-exponent fields, the site, the sensor.
        let mut scenario = Scenario::empty("round trip");
        scenario
            .add_satellite("ISS", element_set(), [12, 34, 56])
            .expect("added");
        scenario
            .add_station(
                "Equator",
                site(),
                Sensor {
                    minimum_elevation: Angle::from_deg(5.0),
                    half_angle: Angle::from_deg(180.0),
                    boresight: (Angle::ZERO, Angle::from_deg(90.0)),
                    maximum_range: Some(Distance::from_km(4_000.0)),
                },
            )
            .expect("added");

        let json = serde_json::to_string_pretty(&scenario).expect("serialises");
        let back: Scenario = serde_json::from_str(&json).expect("deserialises");
        assert_eq!(back, scenario);
    }

    #[test]
    fn duplicate_names_are_allowed() {
        // Names are for people; ids are for the program. Forbidding duplicate names would
        // make renaming a chore for no benefit.
        let mut scenario = Scenario::empty("test");
        scenario
            .add_satellite("same", element_set(), [1, 1, 1])
            .expect("added");
        scenario
            .add_satellite("same", element_set(), [2, 2, 2])
            .expect("added");
        assert_eq!(scenario.satellites.len(), 2);
        scenario.validate().expect("valid");
    }

    #[test]
    fn duplicate_ids_are_refused_by_validation() {
        // The add methods cannot produce this, but a hand-edited JSON file can, so validation
        // has to check it rather than assume.
        let mut scenario = Scenario::empty("test");
        scenario
            .add_satellite("A", element_set(), [1, 1, 1])
            .expect("added");
        let mut copy = scenario.satellites.first().cloned().expect("one");
        copy.name = "B".to_owned();
        scenario.satellites.push(copy);
        assert!(scenario.validate().is_err(), "duplicate ids were accepted");
    }

    #[test]
    fn a_duration_that_is_not_positive_is_refused() {
        let mut scenario = Scenario::empty("test");
        for bad in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            scenario.duration_seconds = bad;
            assert!(scenario.validate().is_err(), "duration {bad} was accepted");
        }
        scenario.duration_seconds = 1.0;
        scenario.validate().expect("one second is valid");
    }

    #[test]
    fn removing_an_object_reports_whether_it_was_there() {
        let mut scenario = Scenario::empty("test");
        let id = scenario
            .add_satellite("A", element_set(), [1, 1, 1])
            .expect("added");
        assert!(scenario.remove_satellite(id));
        assert!(!scenario.remove_satellite(id));
        assert_eq!(scenario.satellites.len(), 0);
    }

    #[test]
    fn ids_are_not_reused_after_removal() {
        // Reusing an id would silently rebind any reference the caller still holds.
        let mut scenario = Scenario::empty("test");
        let a = scenario
            .add_satellite("A", element_set(), [1, 1, 1])
            .expect("added");
        scenario.remove_satellite(a);
        let b = scenario
            .add_satellite("B", element_set(), [2, 2, 2])
            .expect("added");
        assert_ne!(a, b);
    }

    #[test]
    fn the_wgs84_ellipsoid_is_the_one_the_document_assumes() {
        // Nothing in the document records which Earth model is in use; the engine fixes it.
        // That is stated here so a future per-scenario choice is a deliberate change.
        assert_eq!(Ellipsoid::WGS84.semi_major_km, 6_378.137);
    }
}
