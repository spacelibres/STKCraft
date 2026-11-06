//! What the viewport is asked to draw: one instant, already computed.
//!
//! [`Frame`] is the whole interface between the engine and this crate. The engine fills it; the
//! viewport renders it. Keeping it a plain data structure with no references to the scenario is
//! what makes the viewport swappable — see `AGENTS.md`, "the UI is thin and swappable".
//!
//! Units are kilometres and the frame is GCRF (geocentric, right-handed, `+z` toward the mean
//! north pole of J2000), matching `stkcraft-frames`.
//!
//! # Why plain `f32`
//!
//! The engine's `Distance` and `Angle` newtypes exist so a unit mix-up is a compile error *in the
//! engine*. Here every number is a screen-space quantity destined for a vertex buffer, and the
//! conversion happens once, in [`Frame::from_scenario`], through `as_km()`. Nothing downstream of
//! this point can mix units up because nothing downstream of this point has units.

use stkcraft_core::Distance;

/// A satellite's marker at one instant.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SatelliteGlyph {
    /// Position in GCRF, km.
    pub position_km: [f32; 3],
    /// Marker colour, linear RGB in `[0, 1]`.
    pub color: [f32; 3],
}

/// A ground station's marker at one instant.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StationGlyph {
    /// Position in Earth-fixed coordinates, km.
    ///
    /// Earth-fixed rather than GCRF because a station is *fixed* to the Earth: the frame this
    /// glyph is drawn in is the Earth-fixed one, which is where the globe's graticule lives too.
    pub position_km: [f32; 3],
    /// Marker colour.
    pub color: [f32; 3],
}

/// One frame, ready to draw.
///
/// Everything in this struct is already in the **Earth-fixed** frame (ITRF). Satellites are
/// converted once per frame by the engine; the cost is a rotation per satellite and it means the
/// globe, the graticule, the stations and the ground track all share one consistent frame. Keeping
/// satellites in GCRF instead would need a rotating transformation matrix in the shader and would
/// visibly desynchronise the ground track from the globe.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Frame {
    /// The instant being shown, formatted for the overlay.
    pub epoch_label: String,
    /// The Earth's radius used for the globe, km. WGS-84 equatorial radius.
    pub earth_radius_km: f32,
    /// Satellite markers.
    pub satellites: Vec<SatelliteGlyph>,
    /// Station markers.
    pub stations: Vec<StationGlyph>,
    /// Ground tracks: one polyline per satellite, in Earth-fixed km.
    ///
    /// A *ground track* — the sub-satellite point — rather than an inertial orbit, because that is
    /// what a mission analyst reads off a globe: where the satellite passes over the Earth.
    pub tracks: Vec<Vec<[f32; 3]>>,
    /// Where each track's current position is, so a marker can sit on it.
    pub track_colors: Vec<[f32; 3]>,
}

impl Frame {
    /// An empty frame at the given instant.
    #[must_use]
    pub fn empty(epoch_label: impl Into<String>) -> Self {
        Self {
            epoch_label: epoch_label.into(),
            earth_radius_km: EARTH_EQUATORIAL_RADIUS_KM,
            ..Self::default()
        }
    }

    /// How many drawables the frame holds, for the overlay's statistics line.
    #[must_use]
    pub fn drawable_count(&self) -> usize {
        self.satellites.len() + self.stations.len() + self.tracks.len()
    }

    /// The point of every ground track, which is what a bounding calculation would need.
    #[must_use]
    pub fn track_point_count(&self) -> usize {
        self.tracks.iter().map(Vec::len).sum()
    }
}

/// The WGS-84 equatorial radius, km.
///
/// NIMA TR8350.2, third edition, Table 3.1: `a = 6378137.0` m exactly.
pub const EARTH_EQUATORIAL_RADIUS_KM: f32 = 6_378.137;

/// The WGS-84 polar radius `b = a (1 - f)`, km.
///
/// Derived from NIMA TR8350.2 Table 3.1, `1/f = 298.257223563`. A globe drawn as a sphere of the
/// equatorial radius is 21 km too large at the poles; this constant is provided so a later
/// oblate rendering is a parameter change, and its value is checked against the access crate's
/// ellipsoid in the tests.
pub const EARTH_POLAR_RADIUS_KM: f32 = 6_356.752_3;

/// A position in kilometres, converted once from the engine's typed distance.
///
/// This is the only place a `Distance` becomes an `f32`, and it is deliberately a function so a
/// caller cannot do it by accident: `as_km()` is the single conversion the engine guarantees.
#[must_use]
pub fn position_to_f32(position: &[Distance; 3]) -> [f32; 3] {
    [
        position[0].as_km() as f32,
        position[1].as_km() as f32,
        position[2].as_km() as f32,
    ]
}

/// A colour from the scenario's byte triple to linear RGB.
///
/// The values are treated as **sRGB-encoded** and converted, because a scenario written by a
/// person says `[255, 128, 0]` meaning "orange as you see it", not "linear orange". Skipping the
/// conversion would make every user-chosen colour visibly too bright.
#[must_use]
pub fn color_from_bytes(rgb: [u8; 3]) -> [f32; 3] {
    [
        srgb_to_linear(rgb[0]),
        srgb_to_linear(rgb[1]),
        srgb_to_linear(rgb[2]),
    ]
}

/// One channel of the IEC 61966-2-1 sRGB transfer function.
fn srgb_to_linear(channel: u8) -> f32 {
    let encoded = f32::from(channel) / 255.0;
    if encoded <= 0.040_45 {
        encoded / 12.92
    } else {
        ((encoded + 0.055) / 1.055).powf(2.4)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_earth_radii_agree_with_wgs84() {
        // The same two numbers `stkcraft-access` flattens the Earth with.
        let flattening = 1.0 / 298.257_223_563_f64;
        let polar = f64::from(EARTH_EQUATORIAL_RADIUS_KM) * (1.0 - flattening);
        assert!(
            (polar - f64::from(EARTH_POLAR_RADIUS_KM)).abs() < 0.01,
            "the derived polar radius was {polar} km"
        );
        assert_eq!(EARTH_EQUATORIAL_RADIUS_KM, 6_378.137);
    }

    #[test]
    fn an_empty_frame_still_knows_the_earth() {
        let frame = Frame::empty("2024-01-01T12:00:00Z");
        assert_eq!(frame.earth_radius_km, EARTH_EQUATORIAL_RADIUS_KM);
        assert_eq!(frame.drawable_count(), 0);
        assert_eq!(frame.epoch_label, "2024-01-01T12:00:00Z");
    }

    #[test]
    fn colours_are_converted_from_srgb() {
        // The endpoints are exact in any transfer function, and a mid grey must be darker in
        // linear space than its encoded value — that is the whole point of the conversion.
        let black = color_from_bytes([0, 0, 0]);
        let white = color_from_bytes([255, 255, 255]);
        assert!(black.iter().all(|c| c.abs() < 1e-6));
        assert!(white.iter().all(|c| (c - 1.0).abs() < 1e-4));

        let mid = color_from_bytes([128, 128, 128]);
        assert!(mid[0] > 0.2 && mid[0] < 0.23, "mid grey was {:?}", mid[0]);
    }

    #[test]
    fn positions_are_converted_from_typed_distances() {
        let position = [
            Distance::from_km(1.0),
            Distance::from_km(-2.5),
            Distance::from_km(6_378.137),
        ];
        let f = position_to_f32(&position);
        assert_eq!(f, [1.0, -2.5, 6_378.137]);
    }

    #[test]
    fn the_track_count_sums_the_polylines() {
        let mut frame = Frame::empty("now");
        frame.tracks.push(vec![[0.0; 3]; 4]);
        frame.tracks.push(vec![[0.0; 3]; 6]);
        assert_eq!(frame.track_point_count(), 10);
        assert_eq!(frame.drawable_count(), 2);
    }
}
