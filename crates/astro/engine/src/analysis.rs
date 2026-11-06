//! Running access over a scenario.
//!
//! The access crate answers "is this visible at this instant" and "when does it change", given
//! a target position. This module is the part that holds a [`Propagator`] and answers those
//! questions for a satellite in the scenario.
//!
//! # Cost, stated rather than discovered
//!
//! The window finder evaluates the target position several times per boundary while bisecting,
//! and SGP4 restarts its deep-space integrator whenever a call asks for an earlier time than
//! the last one. So a bracketing search is not free. For the near-Earth orbits where pass
//! structure matters most there is no resonance to integrate and the restart costs almost
//! nothing; for a deep-space satellite it costs an integration per back-step. The alternative
//! would be to reimplement the finder as a forward-only sweep, which would trade that cost for
//! a weaker guarantee. The guarantee is worth more.

use stkcraft_astro_access::{AccessProblem, AccessWindow};
use stkcraft_framework_core::{Duration, EpochOffset, Error, Result};
use stkcraft_framework_time::Epoch;

use crate::propagate::{EARTH, Propagator};
use crate::scenario::{SatelliteId, Scenario, StationId};

/// The access a single satellite has to a single station.
#[derive(Debug, Clone, PartialEq)]
pub struct SatelliteAccess {
    /// The satellite.
    pub satellite: SatelliteId,
    /// The station.
    pub station: StationId,
    /// Every interval during which access holds, in time order.
    pub windows: Vec<AccessWindow>,
    /// The total time in access.
    pub total: Duration,
}

/// The access problem for one station, which depends only on the station.
fn problem_for(scenario: &Scenario, station: StationId) -> Result<AccessProblem> {
    let station = scenario.station(station).ok_or_else(|| Error::NotFound {
        what: format!("station {station:?}"),
    })?;
    let problem = AccessProblem {
        site: station.site,
        sensor: station.sensor,
        ellipsoid: EARTH,
    };
    problem.validate()?;
    Ok(problem)
}

/// Compute one satellite-to-station access over the scenario's timeline.
///
/// # Errors
///
/// - [`Error::NotFound`] if either id is not in the scenario.
/// - [`Error::OutOfRange`] if the scenario has no positive duration.
/// - As [`Propagator::new`] and [`AccessProblem::windows`] if the element set or the station is
///   unusable.
pub fn compute_access(
    scenario: &Scenario,
    satellite: SatelliteId,
    station: StationId,
    step: Duration,
) -> Result<SatelliteAccess> {
    scenario.validate()?;
    let satellite_entry = scenario
        .satellite(satellite)
        .ok_or_else(|| Error::NotFound {
            what: format!("satellite {satellite:?}"),
        })?;
    let problem = problem_for(scenario, station)?;
    let propagator = std::cell::RefCell::new(Propagator::new(&satellite_entry.element_set)?);
    let cache: std::cell::RefCell<std::collections::BTreeMap<i64, Option<[f64; 3]>>> =
        std::cell::RefCell::new(std::collections::BTreeMap::new());

    // The scenario's timeline is expressed as an offset from its own epoch, so the window
    // boundaries are offsets. That keeps the result independent of the epoch's time scale.
    let start = EpochOffset::from_seconds(0.0);
    let end = EpochOffset::from_seconds(scenario.duration_seconds);

    let windows = problem.windows(start, end, step, |offset| {
        // The finder takes a `Fn` deliberately: a predicate that could mutate captured state
        // does not define a function of time, and bisecting something that is not a function
        // has no meaning. The propagator IS stateful, so its mutation is confined behind a
        // `RefCell`, and the cache below ensures each instant is propagated once however many
        // times bracketing asks for it.
        //
        // Keyed on whole microseconds so a repeated evaluation at the same instant hits.
        let key = (offset.as_seconds() * 1e6).round() as i64;
        if let Some(cached) = cache.borrow().get(&key) {
            return *cached;
        }
        let at = Epoch::from_julian(
            scenario.epoch.julian().plus_seconds(offset.as_seconds()),
            scenario.epoch.scale(),
        );
        // An unevaluable point yields `None`, which the access layer treats as "not visible"
        // and will close a window there rather than invent one.
        let position = propagator.borrow_mut().itrf(at).ok();
        cache.borrow_mut().insert(key, position);
        position
    })?;

    let total: f64 = windows.iter().map(|w| w.duration().as_seconds()).sum();
    Ok(SatelliteAccess {
        satellite,
        station,
        windows,
        total: Duration::from_seconds(total),
    })
}

/// One satellite's access to a target: a region, not a point.
///
/// A target is a set of vertices — a point is one, a polygon is its corners — and access to the region holds
/// while **any** vertex is visible. So the target's windows are the **union** of its vertices' windows, which
/// is the hand-computable fact this type exists to make testable: a region is visible whenever some part of
/// it is, and two vertices whose windows overlap produce one window rather than two.
#[derive(Debug, Clone, PartialEq)]
pub struct TargetAccess {
    /// The satellite.
    pub satellite: SatelliteId,
    /// The target's display name, so a report can name it without the scenario.
    pub target: String,
    /// How many vertices the target has.
    pub vertices: usize,
    /// The windows the region is visible, merged and in time order.
    pub windows: Vec<AccessWindow>,
    /// The total time any vertex is visible.
    pub total: Duration,
}

/// Compute one satellite's access to a target made of `vertices`.
///
/// # Errors
///
/// As [`compute_access`], plus [`Error::Invalid`] for a target with no vertices: a region with no points is
/// not a region, and reporting "no windows" for it would hide the mistake.
pub fn compute_target_access(
    scenario: &Scenario,
    satellite: SatelliteId,
    target: &str,
    vertices: &[stkcraft_astro_access::GroundSite],
    sensor: &stkcraft_astro_access::Sensor,
    step: Duration,
) -> Result<TargetAccess> {
    if vertices.is_empty() {
        return Err(Error::invalid(
            "vertices",
            "a target needs at least one vertex",
        ));
    }
    let satellite_entry = scenario
        .satellite(satellite)
        .ok_or_else(|| Error::NotFound {
            what: format!("satellite {satellite:?}"),
        })?;
    let start = EpochOffset::from_seconds(0.0);
    let end = EpochOffset::from_seconds(scenario.duration_seconds);
    let propagator = std::cell::RefCell::new(Propagator::new(&satellite_entry.element_set)?);
    let cache: std::cell::RefCell<std::collections::BTreeMap<i64, Option<[f64; 3]>>> =
        std::cell::RefCell::new(std::collections::BTreeMap::new());

    // One propagator for every vertex: the satellite's positions do not depend on which corner is asking,
    // and propagating them again per vertex would double the cost of a polygon for nothing.
    let mut per_vertex: Vec<Vec<AccessWindow>> = Vec::with_capacity(vertices.len());
    for site in vertices {
        let problem = AccessProblem {
            site: *site,
            sensor: *sensor,
            ellipsoid: EARTH,
        };
        problem.validate()?;
        let windows = problem.windows(start, end, step, |offset| {
            let key = (offset.as_seconds() * 1e6).round() as i64;
            if let Some(cached) = cache.borrow().get(&key) {
                return *cached;
            }
            let at = Epoch::from_julian(
                scenario.epoch.julian().plus_seconds(offset.as_seconds()),
                scenario.epoch.scale(),
            );
            let position = propagator.borrow_mut().itrf(at).ok();
            cache.borrow_mut().insert(key, position);
            position
        })?;
        per_vertex.push(windows);
    }
    let windows = union(&per_vertex);
    let total: f64 = windows
        .iter()
        .map(|window| window.duration().as_seconds())
        .sum();
    Ok(TargetAccess {
        satellite,
        target: target.to_owned(),
        vertices: vertices.len(),
        windows,
        total: Duration::from_seconds(total),
    })
}

/// The union of several window lists: what remains when overlapping and touching intervals merge.
///
/// Two windows that meet at a boundary are one window, because "visible, then not, then visible at the same
/// instant" is not a thing a reader can act on. The result is sorted and disjoint.
#[must_use]
pub fn union(lists: &[Vec<AccessWindow>]) -> Vec<AccessWindow> {
    let mut all: Vec<AccessWindow> = lists.iter().flatten().copied().collect();
    all.sort_by(|a, b| {
        a.start
            .as_seconds()
            .partial_cmp(&b.start.as_seconds())
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut merged: Vec<AccessWindow> = Vec::with_capacity(all.len());
    for window in all {
        match merged.last_mut() {
            Some(current) if window.start.as_seconds() <= current.end.as_seconds() => {
                if window.end.as_seconds() > current.end.as_seconds() {
                    current.end = window.end;
                }
            }
            _ => merged.push(window),
        }
    }
    merged
}

/// Compute every satellite-to-station pair in the scenario.
///
/// Pairs are visited in scenario order, so the result is deterministic.
///
/// # Errors
///
/// As [`compute_access`]. A single unusable satellite fails the whole run rather than being
/// silently skipped: an access report that quietly omits an object is worse than one that
/// reports why it could not be produced.
pub fn compute_all_access(scenario: &Scenario, step: Duration) -> Result<Vec<SatelliteAccess>> {
    scenario.validate()?;
    let mut results = Vec::new();
    for satellite in &scenario.satellites {
        for station in &scenario.stations {
            results.push(compute_access(scenario, satellite.id, station.id, step)?);
        }
    }
    Ok(results)
}

#[cfg(test)]
#[expect(clippy::expect_used)]
mod tests {
    use stkcraft_astro_access::Sensor;
    use stkcraft_astro_sgp4::TwoLineElement;
    use stkcraft_framework_core::Angle;

    use super::*;
    use stkcraft_astro_access::GroundSite;

    /// A geostationary element set: one revolution per sidereal day, essentially equatorial.
    const GEO_L1: &str = "1 13552U 82069A   04166.50000000  .00000000  00000-0  00000+0 0  9992";
    const GEO_L2: &str = "2 13552   0.0207  90.3015 0002000 130.0000 230.0000  1.00273791 80004";

    /// A low Earth element set at 51.6 degrees inclination, ISS-like.
    const LEO_L1: &str = "1 25544U 98067A   24001.50000000  .00016717  00000-0  10270-3 0  9009";
    const LEO_L2: &str = "2 25544  51.6400 208.9163 0006317  69.9862  25.2906 15.49569600 12343";

    fn station_at(latitude_deg: f64) -> (GroundSite, Sensor) {
        (
            GroundSite::new(Angle::from_deg(latitude_deg), Angle::from_deg(0.0), 0.0)
                .expect("valid site"),
            Sensor::above_horizon(Angle::from_deg(0.0)),
        )
    }

    fn scenario_with(line1: &str, line2: &str, latitude_deg: f64) -> Scenario {
        let mut scenario = Scenario::empty("access test");
        let tle = TwoLineElement::parse(line1, line2).expect("valid TLE");
        // Anchor the timeline to the element set's own epoch. `Scenario::empty` starts at
        // "now", which is years away from these verification element sets; propagating that
        // far gives a completely different geometry, and for the low Earth case no passes at
        // all over the 45 degree station. That is a test-setup trap, not an engine bug.
        scenario.epoch = tle.epoch;
        scenario
            .add_satellite("SAT", tle, [255, 255, 255])
            .expect("added");
        let (site, sensor) = station_at(latitude_deg);
        scenario.add_station("STN", site, sensor).expect("added");
        scenario.duration_seconds = 86_400.0;
        scenario
    }

    #[test]
    fn a_geostationary_satellite_over_its_station_has_continuous_access() {
        // The satellite sits over the equator at the station's longitude, so the geometry
        // barely changes over a day and access runs the whole timeline as one window.
        let scenario = scenario_with(GEO_L1, GEO_L2, 0.0);
        let satellite = scenario.satellites.first().expect("one").id;
        let station = scenario.stations.first().expect("one").id;
        let result = compute_access(&scenario, satellite, station, Duration::from_seconds(300.0))
            .expect("computes");

        assert_eq!(result.windows.len(), 1, "windows were {:?}", result.windows);
        let only = result.windows.first().expect("one");
        assert!((only.start.as_seconds() - 0.0).abs() < 1.0);
        assert!((only.end.as_seconds() - 86_400.0).abs() < 1.0);
        assert!((result.total.as_seconds() - 86_400.0).abs() < 2.0);
    }

    #[test]
    fn a_low_earth_satellite_has_several_passes_a_day() {
        // A 51.6 degree orbit repeats about every 92 minutes, so a station on the equator sees
        // it several times a day. The bound is deliberately wide: the exact count depends on
        // the element set, and pinning it would make the test fail on any element-set edit
        // without indicating a real regression.
        let scenario = scenario_with(LEO_L1, LEO_L2, 0.0);
        let satellite = scenario.satellites.first().expect("one").id;
        let station = scenario.stations.first().expect("one").id;
        let result = compute_access(&scenario, satellite, station, Duration::from_seconds(30.0))
            .expect("computes");

        let count = result.windows.len();
        assert!(
            (3..=8).contains(&count),
            "a low Earth satellite gave {count} passes in a day"
        );
        // And each pass is a few minutes, not a few seconds or the whole day.
        for window in &result.windows {
            let minutes = window.duration().as_seconds() / 60.0;
            assert!(
                (1.0..30.0).contains(&minutes),
                "a pass lasted {minutes} minutes"
            );
        }
    }

    #[test]
    fn every_window_is_ordered_and_non_overlapping() {
        let scenario = scenario_with(LEO_L1, LEO_L2, 45.0);
        let satellite = scenario.satellites.first().expect("one").id;
        let station = scenario.stations.first().expect("one").id;
        let result = compute_access(&scenario, satellite, station, Duration::from_seconds(30.0))
            .expect("computes");

        // The `windows` helper needs a slice of windows; compare consecutive pairs.
        for pair in result.windows.windows(2) {
            let earlier = pair.first().expect("pair");
            let later = pair.last().expect("pair");
            assert!(
                later.start.as_seconds() > earlier.end.as_seconds(),
                "windows overlap or are out of order: {earlier:?} then {later:?}"
            );
        }
        for window in &result.windows {
            assert!(window.end.as_seconds() > window.start.as_seconds());
        }
    }

    #[test]
    fn raising_the_horizon_mask_reduces_the_total_access() {
        // A mask of 30 degrees must never grant more access than a mask of 0. This is the
        // property a user relies on when tightening a mask to avoid noisy low passes.
        let mut scenario = scenario_with(LEO_L1, LEO_L2, 45.0);
        let satellite = scenario.satellites.first().expect("one").id;
        let station = scenario.stations.first().expect("one").id;
        let step = Duration::from_seconds(30.0);

        let wide = compute_access(&scenario, satellite, station, step).expect("computes");
        assert!(
            wide.total.as_seconds() > 0.0,
            "the wide mask found no access at all, so the comparison means nothing"
        );

        let stricter = Sensor::above_horizon(Angle::from_deg(30.0));
        let entry = scenario
            .stations
            .iter_mut()
            .find(|s| s.id == station)
            .expect("station");
        entry.sensor = stricter;
        scenario
            .validate()
            .expect("the tightened scenario is valid");

        let narrow = compute_access(&scenario, satellite, station, step).expect("computes");
        assert!(
            narrow.total.as_seconds() < wide.total.as_seconds(),
            "a 30 degree mask gave {} s ({} windows), a 0 degree mask gave {} s ({} windows)",
            narrow.total.as_seconds(),
            narrow.windows.len(),
            wide.total.as_seconds(),
            wide.windows.len()
        );
    }

    #[test]
    fn every_pair_is_computed_in_scenario_order() {
        let mut scenario = Scenario::empty("pairs");
        let geo = TwoLineElement::parse(GEO_L1, GEO_L2).expect("valid");
        let leo = TwoLineElement::parse(LEO_L1, LEO_L2).expect("valid");
        let first = scenario
            .add_satellite("GEO", geo, [1, 1, 1])
            .expect("added");
        let second = scenario
            .add_satellite("LEO", leo, [2, 2, 2])
            .expect("added");
        let (site_a, sensor_a) = station_at(0.0);
        let station_a = scenario.add_station("A", site_a, sensor_a).expect("added");
        let (site_b, sensor_b) = station_at(40.0);
        let station_b = scenario.add_station("B", site_b, sensor_b).expect("added");
        scenario.duration_seconds = 3_600.0;

        let results =
            compute_all_access(&scenario, Duration::from_seconds(120.0)).expect("computes");
        assert_eq!(results.len(), 4);
        let order: Vec<(SatelliteId, StationId)> =
            results.iter().map(|r| (r.satellite, r.station)).collect();
        assert_eq!(
            order,
            vec![
                (first, station_a),
                (first, station_b),
                (second, station_a),
                (second, station_b),
            ]
        );
    }

    #[test]
    fn an_unknown_id_is_refused() {
        let scenario = scenario_with(LEO_L1, LEO_L2, 0.0);
        let satellite = scenario.satellites.first().expect("one").id;
        let station = scenario.stations.first().expect("one").id;
        let step = Duration::from_seconds(60.0);

        assert!(compute_access(&scenario, SatelliteId(99), station, step).is_err());
        assert!(compute_access(&scenario, satellite, StationId(99), step).is_err());
    }

    #[test]
    fn a_scenario_with_no_objects_computes_nothing() {
        let scenario = Scenario::empty("empty");
        let results = compute_all_access(&scenario, Duration::from_seconds(60.0)).expect("valid");
        assert_eq!(results.len(), 0);
    }

    #[test]
    fn a_region_is_visible_while_any_corner_is_and_overlapping_windows_merge() {
        // The hand-computable fact a target rests on: the region's windows are the union of its corners'.
        // Two windows that overlap become one; two that meet at a boundary become one; two that are apart
        // stay two; and a window inside another does not lengthen it.
        let window = |start: f64, end: f64| AccessWindow {
            start: EpochOffset::from_seconds(start),
            end: EpochOffset::from_seconds(end),
        };
        let merged = union(&[vec![window(10.0, 20.0), window(30.0, 40.0)]]);
        assert_eq!(merged.len(), 2, "{merged:?}");
        // Overlapping across two vertices.
        let merged = union(&[vec![window(10.0, 20.0)], vec![window(15.0, 30.0)]]);
        assert_eq!(merged.len(), 1, "{merged:?}");
        let only = merged.first().expect("one window");
        assert!((only.start.as_seconds() - 10.0).abs() < f64::EPSILON);
        assert!((only.end.as_seconds() - 30.0).abs() < f64::EPSILON);
        // Touching at a boundary.
        let merged = union(&[vec![window(10.0, 20.0)], vec![window(20.0, 25.0)]]);
        assert_eq!(merged.len(), 1, "{merged:?}");
        // One inside the other does not lengthen the union.
        let merged = union(&[vec![window(10.0, 40.0)], vec![window(15.0, 20.0)]]);
        assert_eq!(merged.len(), 1, "{merged:?}");
        assert!((merged.first().expect("one window").end.as_seconds() - 40.0).abs() < f64::EPSILON);
        // Nothing at all is no windows, not an error.
        assert_eq!(union(&[Vec::new(), Vec::new()]), Vec::new());
    }

    #[test]
    fn a_target_with_no_vertices_is_refused_rather_than_reported_as_invisible() {
        // "No windows" for a region with no points would hide the mistake: an empty target is not a target.
        let scenario = scenario_with(LEO_L1, LEO_L2, 0.0);
        let satellite = scenario.satellites.first().expect("one").id;
        let sensor = stkcraft_astro_access::Sensor::above_horizon(Angle::from_deg(0.0));
        let answer = compute_target_access(
            &scenario,
            satellite,
            "empty",
            &[],
            &sensor,
            Duration::from_seconds(30.0),
        );
        assert!(answer.is_err(), "an empty target was accepted");
    }

    #[test]
    fn a_polygon_sees_what_its_corners_see_and_a_point_target_is_one_window_list() {
        // A one-vertex target is a point, and its windows must be exactly the station's: the code path for a
        // region has to degenerate to the point case rather than approximating it.
        let scenario = scenario_with(LEO_L1, LEO_L2, 0.0);
        let satellite = scenario.satellites.first().expect("one").id;
        let station = scenario.stations.first().expect("one").id;
        let point = compute_access(&scenario, satellite, station, Duration::from_seconds(30.0))
            .expect("computes");
        let site = scenario.stations.first().expect("one").site;
        let sensor = stkcraft_astro_access::Sensor::above_horizon(Angle::from_deg(0.0));
        let target = compute_target_access(
            &scenario,
            satellite,
            "point",
            &[site],
            &sensor,
            Duration::from_seconds(30.0),
        )
        .expect("computes");
        assert_eq!(target.vertices, 1);
        assert_eq!(target.windows.len(), point.windows.len(), "{target:?}");
        assert!((target.total.as_seconds() - point.total.as_seconds()).abs() < 1e-6);
    }
}
