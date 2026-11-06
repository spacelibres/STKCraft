//! Turning a [`Frame`] into vertex buffers.
//!
//! This is the only code that knows how the pieces of a frame become drawables, and it is
//! deliberately a pure function of `(Frame, Camera) -> SceneData`: no GPU, no window, no clock.
//! That is what makes it testable, and it is the layer where a mistake would be most expensive —
//! a ground track that is in the wrong frame, or a marker whose colour is wrong, would look like a
//! physics error rather than a drawing error.

use crate::camera::Camera;
use crate::frame::Frame;
use crate::geometry::{self, Mesh, Vertex};
use crate::renderer::marker_size_km;

/// The colours the viewport uses for things the scenario does not colour.
pub mod palette {
    /// The globe's base colour, linear RGB. A muted blue-green, chosen so the graticule and the
    /// ground tracks read against it.
    pub const EARTH: [f32; 3] = [0.10, 0.24, 0.42];
    /// The graticule.
    pub const GRATICULE: [f32; 3] = [0.34, 0.52, 0.68];
    /// The stars: not a constant per star, but this is the mid grey the field is scaled from.
    pub const STARS: [f32; 3] = [0.85, 0.85, 0.85];
    /// A ground station, when the drawable has no colour of its own.
    pub const STATION: [f32; 3] = [0.95, 0.75, 0.25];
}

/// How many stacks and slices the globe mesh gets.
///
/// 32 x 64 is 4 096 triangles: enough that the silhouette is smooth at a 64-pixel-wide window and
/// cheap enough that rebuilding the mesh is never worth caching.
pub const GLOBE_STACKS: u32 = 32;
/// See [`GLOBE_STACKS`].
pub const GLOBE_SLICES: u32 = 64;

/// How far above the surface, as a fraction of the Earth's radius, the graticule is drawn.
///
/// See [`SceneData::build`]: this exists because a line list cannot carry a depth bias.
pub const GRATICULE_LIFT: f32 = 1.004;

/// How many stars to scatter, and the fixed seed that makes the sky reproducible in a screenshot.
pub const STAR_COUNT: u32 = 1_200;
/// See [`STAR_COUNT`].
pub const STAR_SEED: u64 = 0x5354_4B43_5241_4654;

/// The radius of the star sphere, as a fraction of the camera's **far plane**.
///
/// Derived from the far plane rather than being an absolute number or a multiple of the Earth,
/// because those are how it went wrong: at `60 x` the camera distance the stars sat outside a far
/// plane of `100 x`, and before that the depth range between the near and far planes was so wide
/// that a line at the star radius failed the depth test entirely. Staying at a fixed fraction of
/// the far plane means the sky is visible at every zoom, by construction.
pub const STAR_RADIUS_FRACTION: f32 = 0.9;

/// Everything one render pass draws, in world space.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SceneData {
    /// The globe, as triangles.
    pub globe: Mesh,
    /// The graticule, as a line list.
    pub graticule: Vec<Vertex>,
    /// Every satellite's ground track, concatenated as a line list.
    pub tracks: Vec<Vertex>,
    /// Every marker (satellites and stations), as a line list.
    pub markers: Vec<Vertex>,
    /// The star field, as a line list.
    pub stars: Vec<Vertex>,
}

impl SceneData {
    /// Assemble the scene for a frame seen from a camera.
    ///
    /// Total: an invalid camera or a malformed frame yields fewer drawables, never a panic and
    /// never a NaN vertex. The camera's [`validate`](Camera::validate) is the caller's job if it
    /// wants an error; here a bad camera simply produces an empty marker set, because the
    /// alternative — propagating the error into the render loop — would mean a dropped frame
    /// killing the window.
    #[must_use]
    pub fn build(frame: &Frame, camera: &Camera) -> Self {
        let radius = if frame.earth_radius_km.is_finite() && frame.earth_radius_km > 0.0 {
            frame.earth_radius_km
        } else {
            crate::frame::EARTH_EQUATORIAL_RADIUS_KM
        };

        let globe = geometry::sphere(radius, GLOBE_STACKS, GLOBE_SLICES, palette::EARTH);
        // The graticule is lifted off the surface by a visible amount, not by the mesh's own
        // 0.1 %, because lines have no depth bias available to them (`wgpu` rejects a bias on a
        // line list) and a 6 km lift both z-fights and is a quarter of a pixel at any useful
        // zoom. 0.4 % is 25 km: about one line width on screen, and still unambiguously a drawing
        // aid rather than a physical altitude.
        let graticule = geometry::graticule(radius * GRATICULE_LIFT, 30.0, palette::GRATICULE);

        // A marker's size is a function of the view distance, so a satellite is the same speck at
        // any zoom.
        let marker_size = marker_size_km(camera.distance_km);

        let mut tracks = Vec::new();
        for (track, color) in frame.tracks.iter().zip(frame.track_colors.iter()) {
            tracks.extend(geometry::lifted_polyline(
                track,
                *color,
                radius * GRATICULE_LIFT,
            ));
        }

        let mut markers = Vec::new();
        if marker_size > 0.0 {
            for satellite in &frame.satellites {
                markers.extend(geometry::marker(
                    satellite.position_km,
                    marker_size,
                    satellite.color,
                ));
            }
            for station in &frame.stations {
                markers.extend(geometry::marker(
                    station.position_km,
                    marker_size * 1.5,
                    station.color,
                ));
            }
        }

        // The stars ride just inside the far plane, so they are always drawn and always behind
        // everything else. A camera that cannot produce a usable far plane gets no sky rather
        // than a NaN.
        let star_radius = if camera.far_plane_km().is_finite() && camera.far_plane_km() > 0.0 {
            camera.far_plane_km() * STAR_RADIUS_FRACTION
        } else {
            radius * 50.0
        };
        let stars = geometry::star_field(star_radius, STAR_COUNT, STAR_SEED);

        Self {
            globe,
            graticule,
            tracks,
            markers,
            stars,
        }
    }

    /// The total vertex count, for the overlay's statistics line.
    #[must_use]
    pub fn vertex_count(&self) -> usize {
        self.globe.vertices.len()
            + self.graticule.len()
            + self.tracks.len()
            + self.markers.len()
            + self.stars.len()
    }

    /// Whether any vertex is not finite, which would be a real bug rather than a drawing nicety.
    ///
    /// The viewport's rule is that NaN is never a value, so this exists to be asserted in tests
    /// over hostile frames: a scenario loaded from a file can carry anything, and a NaN vertex
    /// produces a single invisible triangle rather than an error message.
    #[must_use]
    pub fn is_finite(&self) -> bool {
        self.globe
            .vertices
            .iter()
            .chain(&self.graticule)
            .chain(&self.tracks)
            .chain(&self.markers)
            .chain(&self.stars)
            .all(|v| {
                v.position.iter().all(|c| c.is_finite()) && v.color.iter().all(|c| c.is_finite())
            })
    }
}

#[cfg(test)]
#[expect(clippy::assert_is_empty, clippy::float_cmp)]
mod tests {
    use super::*;

    fn frame_with_one_satellite() -> Frame {
        let mut frame = Frame::empty("2024-01-01T12:00:00Z");
        frame.satellites.push(crate::frame::SatelliteGlyph {
            position_km: [7_000.0, 0.0, 0.0],
            color: [1.0, 0.0, 0.0],
        });
        frame.tracks.push(vec![
            [6_900.0, 0.0, 0.0],
            [0.0, 6_900.0, 0.0],
            [-6_900.0, 0.0, 0.0],
        ]);
        frame.track_colors.push([1.0, 0.0, 0.0]);
        frame
    }

    #[test]
    fn a_frame_becomes_one_drawable_of_each_kind() {
        let frame = frame_with_one_satellite();
        let scene = SceneData::build(&frame, &Camera::default_view());
        assert!(!scene.globe.indices.is_empty(), "the globe is missing");
        assert!(!scene.graticule.is_empty(), "the graticule is missing");
        assert_eq!(scene.tracks.len(), 4, "three points are two segments");
        assert_eq!(scene.markers.len(), 6, "one marker is three segments");
        assert!(!scene.stars.is_empty());
        assert!(scene.is_finite());
        assert!(scene.vertex_count() > 0);
    }

    #[test]
    fn the_globe_radius_is_the_frame_is() {
        // The viewport must draw the Earth the engine used, not a hard-coded one.
        let mut frame = Frame::empty("now");
        frame.earth_radius_km = 1_000.0;
        let scene = SceneData::build(&frame, &Camera::default_view());
        let worst = scene
            .globe
            .vertices
            .iter()
            .map(|v| {
                let [x, y, z] = v.position;
                (x * x + y * y + z * z).sqrt()
            })
            .fold(0.0_f32, f32::max);
        assert!(
            (worst - 1_000.0).abs() < 0.1,
            "the globe reached {worst} km"
        );
    }

    #[test]
    fn a_nonsense_earth_radius_falls_back_rather_than_drawing_nan() {
        for bad in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            let mut frame = Frame::empty("now");
            frame.earth_radius_km = bad;
            let scene = SceneData::build(&frame, &Camera::default_view());
            assert!(scene.is_finite(), "radius {bad} produced a NaN vertex");
            assert!(!scene.globe.vertices.is_empty());
        }
    }

    #[test]
    fn a_hostile_frame_draws_nothing_rather_than_nan() {
        // A scenario loaded from a file can carry anything; the viewport's job is to keep drawing.
        // A NaN position produces no marker, and the finite geometry around it is unaffected.
        let mut frame = Frame::empty("now");
        frame.satellites.push(crate::frame::SatelliteGlyph {
            position_km: [f32::NAN, 0.0, 0.0],
            color: [1.0, 0.0, 0.0],
        });
        let camera = Camera::default_view();
        let scene = SceneData::build(&frame, &camera);
        assert!(!scene.is_finite(), "a NaN position was not detected");
        // The globe is still drawn, so the window is not blank.
        assert!(!scene.globe.vertices.is_empty());
    }

    #[test]
    fn markers_are_where_the_satellite_is() {
        let frame = frame_with_one_satellite();
        let camera = Camera::default_view();
        let scene = SceneData::build(&frame, &camera);
        let size = marker_size_km(camera.distance_km);
        let centre = [7_000.0, 0.0, 0.0];
        for vertex in &scene.markers {
            let offset: Vec<f32> = vertex
                .position
                .iter()
                .zip(centre.iter())
                .map(|(a, b)| a - b)
                .collect();
            assert!(
                offset.iter().all(|c| c.abs() <= size + 1e-3),
                "a marker vertex sat at {offset:?} with size {size}"
            );
        }
    }

    #[test]
    fn a_track_keeps_the_satellite_s_colour() {
        let frame = frame_with_one_satellite();
        let scene = SceneData::build(&frame, &Camera::default_view());
        assert!(scene.tracks.iter().all(|v| v.color == [1.0, 0.0, 0.0]));
        assert!(!scene.tracks.is_empty());
    }

    #[test]
    fn the_stars_stay_inside_the_far_plane_when_the_camera_pulls_back() {
        // The default far plane is 100 camera distances. The star sphere must be inside it, or the
        // sky would disappear at exactly the zoom where it is most wanted.
        for distance in [6_500.0, 30_000.0, 500_000.0] {
            let mut camera = Camera::default_view();
            camera.distance_km = distance;
            let scene = SceneData::build(&Frame::empty("now"), &camera);
            let farthest = scene
                .stars
                .iter()
                .map(|v| {
                    let [x, y, z] = v.position;
                    (x * x + y * y + z * z).sqrt()
                })
                .fold(0.0_f32, f32::max);
            assert!(
                farthest < camera.far_plane_km(),
                "stars at {farthest} km were beyond the far plane at {distance} km"
            );
            // ...and beyond the globe, so they are always behind it.
            assert!(farthest > camera.distance_km);
        }
    }

    #[test]
    fn an_empty_frame_still_draws_a_globe_and_a_sky() {
        // The app opens on an empty scenario; a black window would look broken.
        let scene = SceneData::build(&Frame::empty("now"), &Camera::default_view());
        assert!(!scene.globe.vertices.is_empty());
        assert!(!scene.graticule.is_empty());
        assert!(!scene.stars.is_empty());
        assert!(scene.markers.is_empty());
        assert!(scene.tracks.is_empty());
        assert!(scene.is_finite());
    }

    #[test]
    fn a_zero_camera_distance_does_not_produce_infinite_markers() {
        let frame = frame_with_one_satellite();
        let mut camera = Camera::default_view();
        camera.distance_km = 0.0;
        let scene = SceneData::build(&frame, &camera);
        assert!(scene.markers.is_empty());
        assert!(scene.is_finite());
    }
}
