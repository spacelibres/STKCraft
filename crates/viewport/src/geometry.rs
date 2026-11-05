//! Geometry generated in code.
//!
//! The Earth is a UV sphere, the graticule is two families of circles, and the star field is a
//! deterministic scatter. All three are *drawn* rather than loaded, which is the stated
//! preference in `AGENTS.md`: generated geometry carries no licence, needs no `ATTRIBUTION.md`
//! row and cannot go missing at runtime.

/// One vertex: position in kilometres and a linear RGB colour.
///
/// 24 bytes, which is a multiple of 4 and matches the WGSL struct exactly. A test asserts the
/// size and the offsets, because a mismatch here shows up as garbled geometry rather than as a
/// compile error.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Vertex {
    /// Position, km.
    pub position: [f32; 3],
    /// Linear RGB in `[0, 1]`.
    pub color: [f32; 3],
}

impl Vertex {
    /// The size of one vertex in bytes.
    pub const SIZE: usize = size_of::<Self>();

    /// Build a vertex.
    #[must_use]
    pub const fn new(position: [f32; 3], color: [f32; 3]) -> Self {
        Self { position, color }
    }
}

/// A triangle mesh ready for one draw call.
///
/// `Copy` implies `PartialEq` on the vertex list, which is what lets a test compare two scenes
/// without walking them by hand.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Mesh {
    /// The vertices.
    pub vertices: Vec<Vertex>,
    /// Indices into [`Self::vertices`], three per triangle.
    pub indices: Vec<u32>,
}

/// A UV sphere centred on the origin.
///
/// `stacks` runs from the south pole to the north, `slices` around the equator. The minimum is
/// two of each, so a caller cannot ask for a degenerate mesh.
#[must_use]
pub fn sphere(radius_km: f32, stacks: u32, slices: u32, color: [f32; 3]) -> Mesh {
    let stacks = stacks.max(2);
    let slices = slices.max(3);

    let mut vertices = Vec::with_capacity(((stacks + 1) * (slices + 1)) as usize);
    for stack in 0..=stacks {
        let polar = core::f32::consts::PI * (stack as f32) / (stacks as f32);
        let (sin_polar, cos_polar) = polar.sin_cos();
        // Latitude runs from -90 at the south pole to +90 at the north.
        let latitude = core::f32::consts::FRAC_PI_2 - polar;
        for slice in 0..=slices {
            let azimuth = core::f32::consts::TAU * (slice as f32) / (slices as f32);
            let (sin_azimuth, cos_azimuth) = azimuth.sin_cos();
            let position = [
                radius_km * sin_polar * cos_azimuth,
                radius_km * sin_polar * sin_azimuth,
                radius_km * cos_polar,
            ];
            // A gentle shading from the equator to the poles, so the globe reads as a sphere
            // without a texture or a light. Purely a function of latitude, and stated as such.
            let shade = 0.72 + 0.28 * latitude.cos();
            vertices.push(Vertex::new(
                position,
                [color[0] * shade, color[1] * shade, color[2] * shade],
            ));
        }
    }

    let mut indices = Vec::with_capacity((stacks * slices * 6) as usize);
    let row = slices + 1;
    for stack in 0..stacks {
        for slice in 0..slices {
            let a = stack * row + slice;
            let b = a + row;
            // Two triangles, wound counter-clockwise when seen from outside.
            indices.extend_from_slice(&[a, b, a + 1]);
            indices.extend_from_slice(&[a + 1, b, b + 1]);
        }
    }

    Mesh { vertices, indices }
}

/// Latitude and longitude lines on a sphere, as a line list.
///
/// Every pair of consecutive vertices forms one segment, so the caller draws with
/// `PrimitiveTopology::LineList` and no index buffer.
#[must_use]
pub fn graticule(radius_km: f32, step_deg: f32, color: [f32; 3]) -> Vec<Vertex> {
    const SEGMENTS: u32 = 180;

    let mut vertices = Vec::new();
    if !(step_deg.is_finite() && step_deg > 0.0 && step_deg < 180.0) {
        return vertices;
    }

    // The radius is nudged out by a hair so the lines are not z-fighting with the surface.
    let radius = radius_km * 1.001;

    // Parallels, excluding the poles where they degenerate to a point.
    let mut latitude = -90.0 + step_deg;
    while latitude < 90.0 - step_deg * 0.5 {
        let phi = latitude.to_radians();
        let (sin_phi, cos_phi) = phi.sin_cos();
        let ring: Vec<[f32; 3]> = (0..=SEGMENTS)
            .map(|i| {
                let azimuth = core::f32::consts::TAU * (i as f32) / (SEGMENTS as f32);
                let (sin_azimuth, cos_azimuth) = azimuth.sin_cos();
                [
                    radius * cos_phi * cos_azimuth,
                    radius * cos_phi * sin_azimuth,
                    radius * sin_phi,
                ]
            })
            .collect();
        push_ring(&mut vertices, &ring, color);
        latitude += step_deg;
    }

    // Meridians.
    let mut longitude = 0.0;
    while longitude < 360.0 - step_deg * 0.5 {
        let lambda = longitude.to_radians();
        let (sin_lambda, cos_lambda) = lambda.sin_cos();
        let meridian: Vec<[f32; 3]> = (0..=SEGMENTS)
            .map(|i| {
                let phi = -core::f32::consts::FRAC_PI_2
                    + core::f32::consts::PI * (i as f32) / (SEGMENTS as f32);
                let (sin_phi, cos_phi) = phi.sin_cos();
                [
                    radius * cos_phi * cos_lambda,
                    radius * cos_phi * sin_lambda,
                    radius * sin_phi,
                ]
            })
            .collect();
        push_ring(&mut vertices, &meridian, color);
        longitude += step_deg;
    }

    vertices
}

fn push_ring(vertices: &mut Vec<Vertex>, ring: &[[f32; 3]], color: [f32; 3]) {
    for pair in ring.windows(2) {
        if let [a, b] = pair {
            vertices.push(Vertex::new(*a, color));
            vertices.push(Vertex::new(*b, color));
        }
    }
}

/// A deterministic star field on a sphere of the given radius, as a line list of dots.
///
/// Each star is a short segment rather than a point, because a point primitive's size is not
/// portable across backends. The stars are placed by a small linear congruential generator, so
/// the same seed always gives the same sky — a viewport that reshuffles its stars every frame
/// would be distracting and would make a screenshot impossible to compare.
#[must_use]
pub fn star_field(radius_km: f32, count: u32, seed: u64) -> Vec<Vertex> {
    let mut state = seed | 1;
    let mut next = move || {
        // Numerical Recipes' LCG constants. Not cryptographic, not meant to be: the only
        // requirement is that it is deterministic and spreads points over the sphere.
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (state >> 33) as f32 / (u32::MAX >> 1) as f32
    };

    let mut vertices = Vec::with_capacity(count as usize * 2);
    for _ in 0..count {
        // Uniform on the sphere: cos(polar) uniform in [-1, 1], azimuth uniform in [0, 2pi).
        let cos_polar = 2.0 * next() - 1.0;
        let sin_polar = (1.0 - cos_polar * cos_polar).max(0.0).sqrt();
        let azimuth = core::f32::consts::TAU * next();
        let (sin_azimuth, cos_azimuth) = azimuth.sin_cos();
        let direction = [sin_polar * cos_azimuth, sin_polar * sin_azimuth, cos_polar];
        // A short radial tick, so the star survives as a visible speck at any zoom.
        let length = radius_km * 0.004;
        let inner = [
            direction[0] * (radius_km - length),
            direction[1] * (radius_km - length),
            direction[2] * (radius_km - length),
        ];
        let outer = [
            direction[0] * radius_km,
            direction[1] * radius_km,
            direction[2] * radius_km,
        ];
        let brightness = 0.45 + 0.55 * next();
        let color = [brightness, brightness, brightness];
        vertices.push(Vertex::new(inner, color));
        vertices.push(Vertex::new(outer, color));
    }
    vertices
}

/// A polyline as a line list, given points in kilometres.
#[must_use]
pub fn polyline(points: &[[f32; 3]], color: [f32; 3]) -> Vec<Vertex> {
    let mut vertices = Vec::with_capacity(points.len() * 2);
    for pair in points.windows(2) {
        if let [a, b] = pair {
            vertices.push(Vertex::new(*a, color));
            vertices.push(Vertex::new(*b, color));
        }
    }
    vertices
}

/// A polyline whose points are pushed out to a fixed radius, as a line list.
///
/// A ground track hands us sub-satellite points at the satellite's own altitude — hundreds of
/// kilometres up, which is exactly where the track should be. Points *below* the target radius
/// are pushed out to it first: the Earth-fixed sub-satellite point of a satellite at 400 km is
/// still 6 378 km from the geocentre, so nothing needs lifting in practice, but a track sampled
/// through a decayed orbit can dip under the surface, and a line under a solid sphere is invisible.
/// Keeping every track vertex at or above `minimum_radius_km` makes that robust without pretending
/// the satellite is somewhere it is not.
#[must_use]
pub fn lifted_polyline(
    points: &[[f32; 3]],
    color: [f32; 3],
    minimum_radius_km: f32,
) -> Vec<Vertex> {
    let lifted: Vec<[f32; 3]> = points
        .iter()
        .map(|point| {
            let radius = dot(*point, *point).sqrt();
            if radius.is_finite() && radius > 0.0 && radius < minimum_radius_km {
                let scale = minimum_radius_km / radius;
                [point[0] * scale, point[1] * scale, point[2] * scale]
            } else {
                *point
            }
        })
        .collect();
    polyline(&lifted, color)
}

/// The dot product of two vectors.
fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// A small cross at a position, so a satellite is visible at any zoom.
#[must_use]
pub fn marker(position_km: [f32; 3], size_km: f32, color: [f32; 3]) -> Vec<Vertex> {
    let mut vertices = Vec::with_capacity(6);
    for axis in 0..3 {
        // Built by mapping over the components rather than mutating an index, so this cannot
        // panic however the position is shaped.
        let offset = |delta: f32| -> [f32; 3] {
            let mut out = position_km;
            if let Some(component) = out.get_mut(axis) {
                *component += delta;
            }
            out
        };
        vertices.push(Vertex::new(offset(-size_km), color));
        vertices.push(Vertex::new(offset(size_km), color));
    }
    vertices
}

#[cfg(test)]
// Empty-and-non-empty assertions read better as `assert!` than as `assert_eq!` against an empty
// literal; the lint is a readable-diagnostics preference that makes these tests worse.
#[expect(clippy::assert_is_empty)]
mod tests {
    use super::*;

    #[test]
    fn a_vertex_is_twenty_four_bytes_with_the_expected_layout() {
        // The GPU reads this struct by offset. A mismatch is garbled geometry, not a compile
        // error, so the layout is pinned.
        assert_eq!(Vertex::SIZE, 24);
        assert_eq!(core::mem::offset_of!(Vertex, position), 0);
        assert_eq!(core::mem::offset_of!(Vertex, color), 12);
    }

    #[test]
    fn every_sphere_vertex_lies_on_the_radius() {
        let radius = 6_378.137_f32;
        let mesh = sphere(radius, 16, 24, [0.1, 0.3, 0.7]);
        assert!(!mesh.vertices.is_empty());
        for vertex in &mesh.vertices {
            let r = (vertex.position[0] * vertex.position[0]
                + vertex.position[1] * vertex.position[1]
                + vertex.position[2] * vertex.position[2])
                .sqrt();
            assert!((r - radius).abs() < 0.05, "a vertex sat at radius {r}");
        }
    }

    #[test]
    fn a_sphere_has_the_expected_number_of_triangles() {
        let mesh = sphere(1.0, 8, 12, [1.0, 1.0, 1.0]);
        assert_eq!(mesh.indices.len(), (8 * 12 * 6) as usize);
        assert_eq!(mesh.vertices.len(), ((8 + 1) * (12 + 1)) as usize);
        // Every index is in range.
        for index in &mesh.indices {
            assert!((*index as usize) < mesh.vertices.len());
        }
    }

    #[test]
    fn a_degenerate_sphere_request_is_clamped_rather_than_returning_nothing() {
        let mesh = sphere(1.0, 0, 0, [1.0, 1.0, 1.0]);
        assert!(
            mesh.vertices.len() >= 12,
            "stacks and slices were not clamped"
        );
        assert!(mesh.indices.len() >= 12);
    }

    #[test]
    fn the_graticule_sits_just_outside_the_surface() {
        // Z-fighting between the lines and the sphere would make them flicker; the graticule
        // is therefore drawn a hair larger.
        let radius = 6_378.137_f32;
        let lines = graticule(radius, 30.0, [1.0, 1.0, 1.0]);
        assert!(!lines.is_empty());
        for vertex in &lines {
            let r = (vertex.position[0] * vertex.position[0]
                + vertex.position[1] * vertex.position[1]
                + vertex.position[2] * vertex.position[2])
                .sqrt();
            assert!(
                r > radius && r < radius * 1.01,
                "a graticule vertex sat at radius {r}"
            );
        }
    }

    #[test]
    fn a_nonsense_graticule_step_returns_nothing_rather_than_hanging() {
        for bad in [0.0, -10.0, 180.0, 360.0, f32::NAN] {
            assert!(
                graticule(1.0, bad, [1.0, 1.0, 1.0]).is_empty(),
                "step {bad} produced geometry"
            );
        }
    }

    #[test]
    fn the_star_field_is_deterministic_and_on_the_sphere() {
        let radius = 400_000.0_f32;
        let a = star_field(radius, 200, 42);
        let b = star_field(radius, 200, 42);
        assert_eq!(a.len(), 400);
        assert_eq!(a, b, "the same seed gave a different sky");
        for vertex in &a {
            let r = (vertex.position[0] * vertex.position[0]
                + vertex.position[1] * vertex.position[1]
                + vertex.position[2] * vertex.position[2])
                .sqrt();
            assert!(
                (r - radius).abs() < radius * 0.01,
                "a star sat at radius {r}"
            );
        }
    }

    #[test]
    fn a_different_seed_gives_a_different_sky() {
        let a = star_field(1_000.0, 50, 1);
        let b = star_field(1_000.0, 50, 2);
        assert_ne!(a, b);
    }

    #[test]
    fn a_polyline_of_n_points_has_n_minus_one_segments() {
        let points = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [2.0, 0.0, 0.0]];
        let vertices = polyline(&points, [1.0, 0.0, 0.0]);
        assert_eq!(vertices.len(), 4);
        assert_eq!(polyline(&[], [1.0; 3]).len(), 0);
        assert_eq!(polyline(&[[0.0; 3]], [1.0; 3]).len(), 0);
    }

    #[test]
    fn a_marker_is_three_crossing_segments_centred_on_its_position() {
        let vertices = marker([10.0, 20.0, 30.0], 1.0, [1.0, 1.0, 1.0]);
        assert_eq!(vertices.len(), 6);
        for vertex in &vertices {
            let offset = [
                vertex.position[0] - 10.0,
                vertex.position[1] - 20.0,
                vertex.position[2] - 30.0,
            ];
            // Exactly one component is displaced, and only by the marker size.
            let moved = offset.iter().filter(|c| c.abs() > 1e-6).count();
            assert_eq!(moved, 1, "a marker vertex moved in more than one axis");
            assert!(offset.iter().all(|c| c.abs() <= 1.0 + 1e-6));
        }
    }
}
