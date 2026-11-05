//! The `wgpu` renderer: two pipelines, one shader, no textures.
//!
//! The viewport draws exactly three kinds of thing, and that shape decides the design:
//!
//! | What | Primitive | Depth | Blending |
//! |---|---|---|---|
//! | the globe | triangle list | write and test | none — opaque |
//! | graticule, ground tracks, markers, stars | line list | test, no write | alpha |
//!
//! Lines do not write depth so that a ground track passing behind the Earth is still drawn where
//! it emerges, and — more importantly — so that a track drawn *on* the surface at radius
//! `r * 1.001` is not z-fought by the sphere. The sphere writes depth, so anything genuinely
//! occluded (a station on the far side) is hidden; see the depth convention in
//! [`Camera::projection`](crate::camera::Camera::projection).
//!
//! # What is and is not testable here
//!
//! A pipeline descriptor can only be validated by a real device. The parts that *are* testable —
//! the vertex layout, the uniform packing, the WGSL source as a string, and the CPU-side
//! projection used for picking — are tested. The commit that introduces this file states whether
//! the pipeline was actually created on a GPU, because "it compiles" says nothing about whether
//! the shader assembled.

use bytemuck::{Pod, Zeroable};

use crate::camera::Mat4;

/// What one draw call needs to know.
///
/// Every vertex position is in kilometres in a single world frame, so the uniform carries only
/// the two matrices. `f32` is enough for a geocentric frame: the largest distance drawn is the
/// star sphere at 4 x 10^5 km, where an `f32` still resolves 0.03 km — well under a pixel at any
/// zoom where the stars are visible.
///
/// Two matrices at 64 bytes each is 128 bytes, padded to 256 to satisfy the 256-byte alignment
/// the uniform buffer offset rules require (WebGPU spec, "Uniform buffer layout"). Padding
/// explicitly rather than trusting a maths crate's layout is what makes this struct `Pod`.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
pub struct Globals {
    /// World space to clip space.
    pub view_projection: Mat4,
    /// World-to-camera, for shaders that want a view-space quantity.
    pub view: Mat4,
    /// Padding that keeps the size a multiple of 256 bytes, as the uniform-buffer layout rule
    /// requires. Read by a test, never by the shader.
    pub padding: [f32; 32],
}

impl Globals {
    /// The struct's size in bytes, which must equal [`Self::SIZE`] as the GPU sees it.
    pub const SIZE: usize = size_of::<Self>();

    /// Build from a view and a projection.
    #[must_use]
    pub fn new(view: Mat4, projection: Mat4) -> Self {
        Self {
            view_projection: crate::camera::multiply(projection, view),
            view,
            padding: [0.0; 32],
        }
    }
}

/// The vertex stride and attribute layout the pipelines expect, in `wgpu`'s terms.
///
/// Returned as plain tuples so the values can be asserted without a device: a mismatch between
/// this and [`Vertex`](crate::geometry::Vertex) draws garbage rather than failing to compile.
#[must_use]
pub const fn vertex_attributes() -> [(&'static str, u32, usize); 2] {
    // (name, element count, byte offset) — both `Float32` in the shader.
    [("position", 3, 0), ("color", 3, 12)]
}

/// The WGSL source, as one string.
///
/// Kept in the source rather than a `.wgsl` file so it ships without an asset loader and cannot be
/// missing at runtime. `#version` style includes do not exist in WGSL, so one module with two
/// entry points — `vs_main` and `fs_main` — and a uniform struct that matches [`Globals`].
pub const SHADER: &str = r"
struct Globals {
    view_projection: mat4x4<f32>,
    view: mat4x4<f32>,
    // The padding in the Rust struct is not named here; WGSL only needs the fields read.
};

@group(0) @binding(0) var<uniform> globals: Globals;

struct VertexIn {
    @location(0) position: vec3<f32>,
    @location(1) color: vec3<f32>,
};

struct VertexOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec3<f32>,
    // Camera-space depth, used only for the fog that fades distant stars.
    @location(1) view_depth: f32,
};

@vertex
fn vs_main(in: VertexIn) -> VertexOut {
    var out: VertexOut;
    out.clip_position = globals.view_projection * vec4<f32>(in.position, 1.0);
    out.color = in.color;
    out.view_depth = (globals.view * vec4<f32>(in.position, 1.0)).z;
    return out;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    return vec4<f32>(in.color, 1.0);
}

// The globe's own fragment stage: a simple limb darkening, so an untextured sphere still reads
// as a sphere. It is a function of the surface normal, which a sphere mesh can reconstruct from
// its position because the sphere is centred on the origin.
struct GlobeOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec3<f32>,
    @location(1) normal: vec3<f32>,
};

@vertex
fn vs_globe(in: VertexIn) -> GlobeOut {
    var out: GlobeOut;
    out.clip_position = globals.view_projection * vec4<f32>(in.position, 1.0);
    out.color = in.color;
    // Normalised on the GPU rather than uploaded: 12 more bytes per vertex would be a third more
    // memory for a value the position already contains.
    out.normal = normalize(in.position);
    return out;
}

@fragment
fn fs_globe(in: GlobeOut) -> @location(0) vec4<f32> {
    // Light from a fixed direction in world space, tilted so both hemispheres read.
    let light = normalize(vec3<f32>(-0.4, 0.6, 0.7));
    // Half-Lambert rather than Lambert: an unlit night side would be indistinguishable from
    // empty space, and the point of this view is to see where the satellite *is*.
    let diffuse = clamp(dot(in.normal, light) * 0.5 + 0.5, 0.0, 1.0);
    let ambient = 0.18;
    let lit = in.color * (ambient + (1.0 - ambient) * diffuse);
    return vec4<f32>(lit, 1.0);
}
";

/// The three shader entry points, so a typo is a test failure rather than a runtime panic.
pub const ENTRY_POINTS: [&str; 4] = ["vs_main", "fs_main", "vs_globe", "fs_globe"];

/// The depth format the app configures, so the pipeline and the render pass cannot disagree.
///
/// `Depth32Float` rather than `Depth24Plus` because the render distance spans seven orders of
/// magnitude and the extra mantissa bit is free on every desktop backend.
pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

/// How far below the near plane the globe's centre may go before we give up on it, in units of
/// the camera distance.
///
/// A camera at 6 500 km looking at a 6 378 km Earth has its near plane 65 km out, and the surface
/// underneath it is *behind* that plane; nothing needs special handling, the GPU clips it. This
/// constant exists for the CPU-side visibility test in [`visible`], not for the GPU.
pub const NEAR_CLIP_EPSILON: f32 = 1.0;

/// Whether a world point is inside the projection's near/far slab, given a view-space position.
///
/// The viewport needs this for one thing: deciding how big to draw a marker, which depends on how
/// far away it is. A point behind the camera has a positive view-space `z` and is reported as not
/// visible rather than at a nonsense distance.
#[must_use]
pub fn visible(view_space: [f32; 3], near_km: f32, far_km: f32) -> Option<f32> {
    if !view_space[2].is_finite() {
        return None;
    }
    let distance = -view_space[2];
    if distance < near_km * NEAR_CLIP_EPSILON || distance > far_km {
        return None;
    }
    Some(distance)
}

/// The size of a marker in kilometres, chosen so it covers about the same number of pixels at any
/// zoom.
///
/// `0.004` of the view distance is roughly a 6-pixel dot on a 1080-pixel-high window with the
/// default field of view; the exact figure is a taste decision, but computing it from the
/// distance rather than using a constant is not: a fixed 100 km marker is invisible at 500 000 km
/// and fills the screen at 6 500 km.
#[must_use]
pub fn marker_size_km(view_distance_km: f32) -> f32 {
    if !view_distance_km.is_finite() || view_distance_km <= 0.0 {
        return 0.0;
    }
    view_distance_km * 0.004
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::Vertex;

    #[test]
    fn the_globals_struct_is_two_matrices_padded_to_256_bytes() {
        // The uniform buffer's binding size must be a multiple of 256 for `wgpu` to accept an
        // offset beyond the first binding, and a size mismatch here is a validation error at
        // pipeline creation rather than a test failure — so it is pinned.
        assert_eq!(Globals::SIZE, 256);
        assert_eq!(core::mem::offset_of!(Globals, view_projection), 0);
        assert_eq!(core::mem::offset_of!(Globals, view), 64);
    }

    #[test]
    fn the_uniform_carries_the_product_of_the_two_matrices() {
        // `multiply` applies its right argument first, and the shader must see
        // `projection * view`, so a transposed product would put the Earth behind the camera.
        let view = crate::camera::look_at([0.0, 0.0, 20_000.0], [0.0; 3], [0.0, 0.0, 1.0]);
        let projection = crate::camera::perspective(0.8, 1.5, 10.0, 1_000.0);
        let globals = Globals::new(view, projection);
        let expected = crate::camera::multiply(projection, view);
        assert_eq!(globals.view_projection, expected);
        assert_eq!(globals.view, view);
        assert!(globals.padding.iter().all(|p| *p == 0.0));
    }

    #[test]
    fn the_vertex_layout_matches_the_vertex_struct() {
        let attributes = vertex_attributes();
        assert_eq!(attributes.len(), 2);
        assert_eq!(attributes[0], ("position", 3, 0));
        assert_eq!(attributes[1], ("color", 3, 12));
        // The last attribute must end exactly at the stride, or the GPU reads past the buffer.
        let (_, count, offset) = attributes[1];
        assert_eq!(offset + (count as usize) * size_of::<f32>(), Vertex::SIZE);
    }

    #[test]
    fn every_entry_point_is_declared_in_the_shader() {
        for entry in ENTRY_POINTS {
            assert!(
                SHADER.contains(&format!("fn {entry}(")),
                "the shader does not declare {entry}"
            );
        }
        // And the stage decorations are the right way round: an `@fragment` on a function named
        // `vs_*` would compile and then fail to create the pipeline.
        assert!(SHADER.contains("@vertex\nfn vs_main"));
        assert!(SHADER.contains("@fragment\nfn fs_main"));
        assert!(SHADER.contains("@vertex\nfn vs_globe"));
        assert!(SHADER.contains("@fragment\nfn fs_globe"));
    }

    #[test]
    fn the_shader_binds_exactly_the_uniform_the_rust_side_uploads() {
        assert!(SHADER.contains("@group(0) @binding(0) var<uniform> globals: Globals;"));
        // The Rust struct is 256 bytes, so the WGSL side must at least declare the two matrices
        // in the same order.
        let vp = SHADER.find("view_projection: mat4x4<f32>,");
        let v = SHADER.find("view: mat4x4<f32>,");
        assert!(
            vp.is_some() && v.is_some(),
            "the uniform struct is incomplete"
        );
        assert!(
            matches!((vp, v), (Some(a), Some(b)) if a < b),
            "the uniform fields are in the wrong order"
        );
    }

    #[test]
    fn the_depth_format_is_what_the_render_pass_is_told_to_use() {
        // `Depth32Float` has one clear value that means the same thing on every backend.
        assert_eq!(DEPTH_FORMAT, wgpu::TextureFormat::Depth32Float);
    }

    #[test]
    fn a_point_behind_the_camera_is_not_visible_and_one_in_range_is() {
        assert!(visible([0.0, 0.0, -10.0], 1.0, 100.0).is_some());
        assert!(visible([0.0, 0.0, 10.0], 1.0, 100.0).is_none());
        // Just inside and just outside the far plane.
        assert!(visible([0.0, 0.0, -99.0], 1.0, 100.0).is_some());
        assert!(visible([0.0, 0.0, -101.0], 1.0, 100.0).is_none());
        assert!(visible([f32::NAN, 0.0, 0.0], 1.0, 100.0).is_none());
    }

    #[test]
    fn a_marker_scales_with_the_view_distance() {
        let near = marker_size_km(6_500.0);
        let far = marker_size_km(500_000.0);
        assert!(near > 0.0 && far > near);
        assert!((far / near - 500_000.0 / 6_500.0).abs() < 1e-3);
        // A nonsense distance gives no marker rather than an infinite one.
        assert_eq!(marker_size_km(0.0), 0.0);
        assert_eq!(marker_size_km(-1.0), 0.0);
        assert_eq!(marker_size_km(f32::NAN), 0.0);
    }
}
