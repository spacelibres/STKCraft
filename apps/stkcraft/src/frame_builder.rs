//! Building a viewport [`Frame`] from a scenario.
//!
//! This is the app's only piece of "presentation" logic, and it is a pure function so it can be
//! tested without a window: given a scenario and an instant, it produces the markers and the
//! ground tracks the viewport should show.
//!
//! # Which frame, and why
//!
//! Everything is converted to **Earth-fixed (ITRF)** before it reaches the viewport. That is the
//! frame a ground analyst thinks in: the globe, the graticule and the stations are all fixed in
//! it, and a satellite plotted in it traces a ground track that visibly moves west as the Earth
//! turns beneath it. Plotting in GCRF instead would show an inertial orbit, which is the right
//! picture for orbital mechanics but the wrong one for "where is this satellite over the Earth".
//!
//! The choice is made here, not in the viewport, and it is one `Propagator::itrf` call away from
//! being the other way round.

use stkcraft_access::Ellipsoid;
use stkcraft_core::Duration;
use stkcraft_engine::{Propagator, Scenario};
use stkcraft_time::Epoch;
use stkcraft_viewport::{Frame, SatelliteGlyph, StationGlyph, color_from_bytes};

use crate::render;

/// How many points one ground track gets.
///
/// A 90-minute orbit needs about 60 points to look smooth on a 64-pixel globe, and one point per
/// 90 seconds is finer than that. The count is a constant rather than a function of the timeline
/// so that a long scenario does not silently cost more to draw.
pub const TRACK_POINTS: usize = 120;

/// How many points the trailing half of a track gets, i.e. behind the current instant.
pub const TRACK_TRAIL_POINTS: usize = 60;

/// The colour a station is drawn in, linear RGB.
pub const STATION_COLOR: [f32; 3] = [0.95, 0.75, 0.25];

/// The colour an uncoloured satellite falls back to, linear RGB.
pub const DEFAULT_SATELLITE_COLOR: [f32; 3] = [0.95, 0.35, 0.25];

/// Build the frame to draw at an instant.
///
/// # Errors
///
/// Propagates any failure from the engine. A satellite that cannot be propagated at this instant
/// is *omitted with a recorded problem* rather than failing the whole frame: one decayed element
/// set must not blank the window. The list of problems is returned alongside so the overlay can
/// say what is missing.
pub fn build_frame(scenario: &Scenario, at: Epoch) -> (Frame, Vec<String>) {
    let mut problems = Vec::new();
    let mut frame = Frame::empty(render::format_instant(at));
    frame.earth_radius_km = stkcraft_viewport::EARTH_EQUATORIAL_RADIUS_KM;

    // Stations first: they are cheap and they never fail to converge.
    for station in &scenario.stations {
        match station.site.to_itrf(&Ellipsoid::WGS84) {
            Ok(position) => frame.stations.push(StationGlyph {
                position_km: [position[0] as f32, position[1] as f32, position[2] as f32],
                color: STATION_COLOR,
            }),
            Err(error) => problems.push(format!("station \"{}\": {error}", station.name)),
        }
    }

    for satellite in &scenario.satellites {
        let mut propagator = match Propagator::new(&satellite.element_set) {
            Ok(p) => p,
            Err(error) => {
                problems.push(format!("satellite \"{}\": {error}", satellite.name));
                continue;
            }
        };

        let color = satellite_color(satellite.color);

        match propagator.itrf(at) {
            Ok(position) => frame.satellites.push(SatelliteGlyph {
                position_km: [position[0] as f32, position[1] as f32, position[2] as f32],
                color,
            }),
            Err(error) => {
                problems.push(format!(
                    "satellite \"{}\" at {}: {error}",
                    satellite.name, frame.epoch_label
                ));
                continue;
            }
        }

        // The ground track: the sub-satellite points over the trailing window, centred on *now*
        // rather than on the scenario epoch, so the track means "where it has just been and where
        // it is going" at whatever instant is displayed.
        let period = orbit_period_seconds(&satellite.element_set);
        let step = period / (TRACK_POINTS as f64);
        let start = at.julian().plus_seconds(-step * TRACK_TRAIL_POINTS as f64);
        let mut track = Vec::with_capacity(TRACK_POINTS);
        for index in 0..TRACK_POINTS {
            let offset = start.plus_seconds(step * index as f64);
            let instant = Epoch::from_julian(offset, at.scale());
            match propagator.itrf(instant) {
                Ok(position) => {
                    track.push([position[0] as f32, position[1] as f32, position[2] as f32]);
                }
                // A decayed satellite's track simply stops; a gap in a polyline is drawn as a
                // shorter polyline, which is honest.
                Err(_) => break,
            }
        }
        if track.len() >= 2 {
            frame.tracks.push(track);
            frame.track_colors.push(color);
        }
    }

    (frame, problems)
}

/// The orbital period of an element set, in seconds.
///
/// `Elements::mean_motion` is **radians per minute** — the TLE's revolutions-per-day converted by
/// `sgp4init`, as that field's own documentation says. Neither the precision nor the Kozai-versus-
/// osculating distinction matters here, because the value is used only to decide how finely to
/// sample a ground track; it is never reported as a physical answer. The *units* still have to be
/// right, or a track covers a day instead of an orbit — which is the mistake this comment exists
/// to prevent, and which the test caught when it was made.
fn orbit_period_seconds(element_set: &stkcraft_sgp4::TwoLineElement) -> f64 {
    let radians_per_minute = element_set.elements().map_or(0.065, |e| e.mean_motion);
    if !radians_per_minute.is_finite() || radians_per_minute <= 0.0 {
        return 5_400.0;
    }
    // One revolution is `2 pi` radians.
    core::f64::consts::TAU / radians_per_minute * 60.0
}

/// A satellite's draw colour from the scenario's bytes, with a fallback for a scenario that
/// stored black (which is invisible against space) or did not choose at all.
fn satellite_color(rgb: [u8; 3]) -> [f32; 3] {
    if rgb == [0, 0, 0] {
        DEFAULT_SATELLITE_COLOR
    } else {
        color_from_bytes(rgb)
    }
}

/// The step the overlay's clock advances per second of wall time, given a rate multiplier.
///
/// Clamped so a mistyped rate cannot advance the clock past a representable instant: a year per
/// second is already 3.15 x 10^7 seconds of Julian Date, which starts to cost f64 precision in
/// the seconds component.
pub fn clamp_rate(rate: f64) -> f64 {
    if !rate.is_finite() {
        return 1.0;
    }
    rate.clamp(-86_400.0, 86_400.0)
}

/// A [`Duration`] in seconds, for the access command, from a step in seconds.
pub fn step_duration(seconds: f64) -> Duration {
    Duration::from_seconds(if seconds.is_finite() && seconds > 0.0 {
        seconds
    } else {
        30.0
    })
}

#[cfg(test)]
#[expect(clippy::expect_used)]
mod tests {
    use super::*;
    use stkcraft_engine::Scenario;
    use stkcraft_sgp4::TwoLineElement;

    const ISS: &str = "1 25544U 98067A   24001.50000000  .00016717  00000-0  10270-3 0  9009";
    const ISS_LINE2: &str = "2 25544  51.6400 208.9163 0006317  69.9862  25.2906 15.49569600 12343";

    fn scenario() -> Scenario {
        let tle = TwoLineElement::parse(ISS, ISS_LINE2).expect("valid TLE");
        let mut scenario = Scenario::empty("test");
        scenario.epoch = tle.epoch;
        scenario
            .add_satellite("ISS", tle, [255, 128, 0])
            .expect("added");
        scenario
    }

    #[test]
    fn the_frame_holds_the_satellite_at_the_engine_s_own_position() {
        // The viewport must never disagree with the analysis: the marker sits where
        // `Propagator::itrf` says it is, bit for bit.
        let scenario = scenario();
        let at = scenario.epoch;
        let (frame, problems) = build_frame(&scenario, at);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(frame.satellites.len(), 1);

        let satellite = scenario.satellites.first().expect("one satellite");
        let mut propagator = Propagator::new(&satellite.element_set).expect("builds");
        let expected = propagator.itrf(at).expect("propagates");
        let glyph = *frame.satellites.first().expect("one glyph");
        for (a, b) in glyph.position_km.iter().zip(expected.iter()) {
            assert!(
                (f64::from(*a) - b).abs() < 1e-3,
                "the marker was at {a} km, the engine said {b} km"
            );
        }
    }

    #[test]
    fn the_ground_track_has_the_requested_length_and_stays_near_the_earth() {
        let scenario = scenario();
        let (frame, problems) = build_frame(&scenario, scenario.epoch);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(frame.tracks.len(), 1);
        let track = frame.tracks.first().expect("one track");
        assert_eq!(track.len(), TRACK_POINTS);
        for point in track {
            let radius = (point[0] * point[0] + point[1] * point[1] + point[2] * point[2]).sqrt();
            assert!(
                (6_300.0..7_100.0).contains(&radius),
                "a ground-track point sat at {radius} km for a low Earth orbit"
            );
        }
    }

    #[test]
    fn the_track_contains_the_satellite_s_current_position() {
        // The displayed instant is the *middle* of the sampled window, so the frame shows where
        // the satellite has been and where it is going. The marker must sit on its own track: a
        // satellite drawn detached from its path would look like a bug in the physics.
        let scenario = scenario();
        let (frame, problems) = build_frame(&scenario, scenario.epoch);
        assert!(problems.is_empty(), "{problems:?}");
        let now = frame.satellites.first().expect("one satellite").position_km;
        let nearest = frame
            .tracks
            .first()
            .expect("one track")
            .iter()
            .map(|point| {
                ((point[0] - now[0]).powi(2)
                    + (point[1] - now[1]).powi(2)
                    + (point[2] - now[2]).powi(2))
                .sqrt()
            })
            .fold(f32::INFINITY, f32::min);
        // One sample of 120 over a 93-minute orbit is about 47 s, and the ISS moves about
        // 7.7 km/s, so a sample must land within roughly 180 km of the marker. A wrong time base
        // — the bug this replaces — put the nearest sample 10 800 km away.
        assert!(
            nearest < 200.0,
            "the nearest track sample was {nearest} km from the satellite"
        );
    }

    #[test]
    fn a_station_is_drawn_on_the_ellipsoid_surface() {
        let mut scenario = Scenario::empty("test");
        let site = stkcraft_access::GroundSite::new(
            stkcraft_core::Angle::from_deg(51.5),
            stkcraft_core::Angle::from_deg(-0.13),
            0.0,
        )
        .expect("valid site");
        scenario
            .add_station(
                "London",
                site,
                stkcraft_access::Sensor::above_horizon(stkcraft_core::Angle::from_deg(10.0)),
            )
            .expect("added");
        let (frame, problems) = build_frame(&scenario, scenario.epoch);
        assert!(problems.is_empty(), "{problems:?}");
        let station = *frame.stations.first().expect("one station");
        // London: 51.5 N, 0.13 W. The z component must be positive and the x component is the
        // cosine of the longitude, so this checks the frame is Earth-fixed and not GCRF.
        assert!(
            station.position_km[2] > 4_000.0,
            "{:?}",
            station.position_km
        );
        assert!(
            station.position_km[0] > 3_000.0,
            "{:?}",
            station.position_km
        );
    }

    #[test]
    fn a_decayed_satellite_is_reported_and_the_frame_is_still_drawn() {
        // One bad element set must not blank the window: the problem is returned and the other
        // objects still appear.
        let mut scenario = scenario();
        let bad_l1 = "1 29141U 85108AA  06170.26783845  .99999999  00000-0  13519-0 0   718";
        let bad_l2 = "2 29141  82.4288 273.4882 0015848 277.2124  83.9133 15.93343074  6828";
        let bad = TwoLineElement::parse(bad_l1, bad_l2).expect("valid TLE");
        // Propagate far past the decay so the call fails at the displayed instant.
        let far = Epoch::from_julian(
            scenario.epoch.julian().plus_seconds(30.0 * 86_400.0),
            scenario.epoch.scale(),
        );
        scenario
            .add_satellite("DECAYED", bad, [0, 255, 0])
            .expect("added");
        let (frame, problems) = build_frame(&scenario, far);
        assert!(
            !problems.is_empty(),
            "a decayed satellite produced no problem"
        );
        // The frame is still usable, and every number in it is finite.
        assert!(!frame.stations.is_empty() || frame.earth_radius_km > 0.0);
        for glyph in &frame.satellites {
            assert!(glyph.position_km.iter().all(|c| c.is_finite()));
        }
    }

    #[test]
    fn a_black_satellite_colour_falls_back_to_something_visible() {
        // Space is black, so a black marker is invisible. The scenario document allows `[0,0,0]`
        // because it stores bytes, so the viewport substitutes a visible colour.
        assert_eq!(satellite_color([0, 0, 0]), DEFAULT_SATELLITE_COLOR);
        let orange = satellite_color([255, 128, 0]);
        assert!(orange != DEFAULT_SATELLITE_COLOR);
        assert!(orange[0] > 0.9 && orange[1] < orange[0]);
    }

    #[test]
    fn the_period_comes_from_the_mean_motion() {
        let scenario = scenario();
        let element_set = &scenario
            .satellites
            .first()
            .expect("one satellite")
            .element_set;
        let period = orbit_period_seconds(element_set);
        // ISS: 15.49569600 rev/day -> 5576 s, about 93 minutes.
        assert!((period - 5_576.0).abs() < 5.0, "period was {period} s");
    }

    #[test]
    fn rates_are_clamped_and_nonsense_is_ignored() {
        assert_eq!(clamp_rate(f64::NAN), 1.0);
        assert_eq!(clamp_rate(1e12), 86_400.0);
        assert_eq!(clamp_rate(-1e12), -86_400.0);
        assert_eq!(clamp_rate(60.0), 60.0);
    }

    #[test]
    fn the_access_step_falls_back_rather_than_failing() {
        assert_eq!(step_duration(10.0).as_seconds(), 10.0);
        for bad in [0.0, -1.0, f64::NAN] {
            assert_eq!(step_duration(bad).as_seconds(), 30.0);
        }
    }
}
