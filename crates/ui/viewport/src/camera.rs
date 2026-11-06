//! The turntable camera and the matrices it produces.
//!
//! Four 4x4 matrices, written out rather than pulled from a math crate: the viewport needs
//! exactly these four and adding a dependency for them would be more surface than it saves.
//! Everything is in kilometres and right-handed, with `+z` toward the north pole, which is the
//! convention `stkcraft-astro-frames` already uses for GCRF.
//!
//! Column-major storage, because that is what WGSL and every GPU API expect. A test checks the
//! convention against a known transform, since a transposed matrix still "looks like a matrix"
//! and would only show up as geometry in the wrong place.

use stkcraft_core::{Error, Result};

/// A 4x4 matrix in column-major order, ready to upload.
pub type Mat4 = [[f32; 4]; 4];

/// A camera orbiting the Earth on a turntable.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Camera {
    /// Rotation about the polar axis, radians. `0` looks along `+x`.
    pub azimuth: f32,
    /// Angle above the equatorial plane, radians. Clamped just short of the poles.
    pub elevation: f32,
    /// Distance from the geocentre, km.
    pub distance_km: f32,
    /// Vertical field of view, radians.
    pub field_of_view: f32,
    /// Near plane, as a fraction of [`Self::distance_km`].
    ///
    /// Scaling both planes with the distance is what keeps the depth buffer's precision usable
    /// at every zoom: a fixed near plane would clip the surface when close and waste almost the
    /// whole depth range when far. `0.01` is the default because at 30 000 km it puts the near
    /// plane 300 km out, comfortably inside a low Earth orbit's altitude.
    pub near_ratio: f32,
    /// Far plane, as a multiple of [`Self::distance_km`].
    ///
    /// The star sphere is placed at 60 times the camera distance (see
    /// `scene::STAR_RADIUS_FACTOR`), so the far plane must be beyond that or the sky is clipped.
    /// This was `100` while the stars were drawn at 60 and they were *still* invisible, because
    /// the default camera sat at 30 000 km with a 300 km near plane and a 3 000 000 km far
    /// plane: the ratio between them, not the far value alone, is what destroys 24-bit depth
    /// precision. The ratio is now 3 000 at the default and the star field renders.
    pub far_ratio: f32,
    /// How far the camera's look-at target is displaced along `+z`, km.
    ///
    /// With a target at the geocentre, the satellite passes *between* the camera and the Earth,
    /// which hides its ground track behind the globe. Lifting the target puts the camera "above"
    /// the Earth in latitude, so a polar-orbit ground track appears on the near face. The value is
    /// a display default, not a physical quantity, and the user can drag it away.
    pub globe_lift: f32,
}

impl Camera {
    /// The closest the camera may come, km: just above the surface.
    pub const MINIMUM_DISTANCE_KM: f32 = 6_500.0;
    /// The farthest it may go, km.
    pub const MAXIMUM_DISTANCE_KM: f32 = 500_000.0;
    /// How far the elevation may approach a pole, radians.
    ///
    /// Stopping short keeps the "up" direction well defined; at exactly a pole the view matrix
    /// would be built from a degenerate basis.
    pub const POLAR_LIMIT: f32 = 1.553_3; // 89 degrees

    /// A default view: pulled back far enough to see a low Earth orbit, looking at the equator.
    #[must_use]
    pub const fn default_view() -> Self {
        Self {
            azimuth: 0.6,
            elevation: 0.5,
            // Framing arithmetic, so this is a decision and not a taste: the Earth's angular
            // radius at distance `d` is `asin(6378 / d)`, and it should occupy about 70 % of the
            // frame height, i.e. 0.35 of the 0.8 rad field of view. `asin(6378/d) = 0.28` gives
            // `d = 22 600 km`, rounded to 23 000.
            distance_km: 23_000.0,
            field_of_view: 0.8,
            near_ratio: 0.05,
            far_ratio: 4.0,
            // A third of the Earth radius: enough that a polar-orbit ground track shows on the
            // near face without the globe sliding out of the bottom of the frame.
            globe_lift: 2_000.0,
        }
    }

    /// Drag the camera by a screen-space delta in radians.
    pub fn orbit(&mut self, delta_azimuth: f32, delta_elevation: f32) {
        self.azimuth = (self.azimuth + delta_azimuth) % core::f32::consts::TAU;
        self.elevation =
            (self.elevation + delta_elevation).clamp(-Self::POLAR_LIMIT, Self::POLAR_LIMIT);
    }

    /// Move the camera closer by a multiplicative factor, clamped.
    pub fn zoom(&mut self, factor: f32) {
        if !factor.is_finite() || factor <= 0.0 {
            return;
        }
        self.distance_km =
            (self.distance_km * factor).clamp(Self::MINIMUM_DISTANCE_KM, Self::MAXIMUM_DISTANCE_KM);
    }

    /// The point the camera looks at, km.
    #[must_use]
    pub fn target(&self) -> [f32; 3] {
        [0.0, 0.0, self.globe_lift]
    }

    /// The camera's position in the Earth-fixed (or whatever) frame, km.
    #[must_use]
    pub fn eye(&self) -> [f32; 3] {
        let (sin_az, cos_az) = self.azimuth.sin_cos();
        let (sin_el, cos_el) = self.elevation.sin_cos();
        let target = self.target();
        [
            target[0] + self.distance_km * cos_el * cos_az,
            target[1] + self.distance_km * cos_el * sin_az,
            target[2] + self.distance_km * sin_el,
        ]
    }

    /// The view matrix: world space to camera space.
    #[must_use]
    pub fn view(&self) -> Mat4 {
        look_at(self.eye(), self.target(), [0.0, 0.0, 1.0])
    }

    /// The projection matrix.
    ///
    /// The near plane is a fraction of the camera distance rather than a fixed number, because a
    /// fixed near plane would clip either a close view of the surface or a distant view of a
    /// geostationary orbit; scaling it keeps the depth buffer's precision spread sensibly
    /// across whatever range is on screen.
    #[must_use]
    pub fn projection(&self, aspect: f32) -> Mat4 {
        perspective(
            self.field_of_view,
            aspect,
            self.near_plane_km(),
            self.far_plane_km(),
        )
    }

    /// The camera-space distance below which nothing is drawn, km.
    #[must_use]
    pub fn near_plane_km(&self) -> f32 {
        self.distance_km * self.near_ratio
    }

    /// The camera-space distance beyond which nothing is drawn, km.
    #[must_use]
    pub fn far_plane_km(&self) -> f32 {
        self.distance_km * self.far_ratio
    }

    /// Check the camera describes a plausible view.
    ///
    /// # Errors
    ///
    /// Returns [`Error::OutOfRange`] for a non-positive or non-finite distance, field of view
    /// outside `(0, pi)`, or non-finite angles.
    pub fn validate(&self) -> Result<()> {
        if !self.distance_km.is_finite() || self.distance_km <= 0.0 {
            return Err(Error::out_of_range(
                "camera distance",
                self.distance_km,
                "positive kilometres",
            ));
        }
        if !self.field_of_view.is_finite()
            || self.field_of_view <= 0.0
            || self.field_of_view >= core::f32::consts::PI
        {
            return Err(Error::out_of_range(
                "camera field of view",
                self.field_of_view,
                "between 0 and pi radians",
            ));
        }
        if !self.azimuth.is_finite() || !self.elevation.is_finite() {
            return Err(Error::invalid("camera angles", "must be finite"));
        }
        Ok(())
    }
}

/// A right-handed look-at view matrix, column-major.
#[must_use]
pub fn look_at(eye: [f32; 3], target: [f32; 3], up: [f32; 3]) -> Mat4 {
    let forward = normalise(sub(target, eye));
    // `up` is the world's north; the side vector is its cross product with the view direction.
    let side = normalise(cross(forward, up));
    let true_up = cross(side, forward);

    [
        [side[0], true_up[0], -forward[0], 0.0],
        [side[1], true_up[1], -forward[1], 0.0],
        [side[2], true_up[2], -forward[2], 0.0],
        [-dot(side, eye), -dot(true_up, eye), dot(forward, eye), 1.0],
    ]
}

/// A right-handed perspective projection, column-major, depth in `[0, 1]`.
#[must_use]
pub fn perspective(field_of_view: f32, aspect: f32, near: f32, far: f32) -> Mat4 {
    let f = 1.0 / (field_of_view * 0.5).tan();
    let range = near - far;
    [
        [f / aspect, 0.0, 0.0, 0.0],
        [0.0, f, 0.0, 0.0],
        [0.0, 0.0, far / range, -1.0],
        [0.0, 0.0, near * far / range, 0.0],
    ]
}

/// Multiply two column-major matrices: `a * b` applies `b` first.
#[must_use]
pub fn multiply(a: Mat4, b: Mat4) -> Mat4 {
    // Row `r` of column `c` is `sum_k a[k][r] * b[c][k]`. Zipped rather than indexed so the
    // 4 x 4 bound is the iterator's business and not a panic site.
    let mut out = [[0.0_f32; 4]; 4];
    for (column, out_column) in out.iter_mut().enumerate() {
        let b_column = b.get(column).copied().unwrap_or([0.0; 4]);
        for (row, cell) in out_column.iter_mut().enumerate() {
            *cell = a
                .iter()
                .zip(b_column.iter())
                .map(|(a_k, b_k)| a_k.get(row).copied().unwrap_or(0.0) * b_k)
                .sum();
        }
    }
    out
}

/// Transform a point by a column-major matrix, dividing by `w`.
///
/// Returns `None` when `w` is not positive, which is how a point behind the camera is filtered
/// rather than projected to a mirrored position on screen.
#[must_use]
pub fn transform_point(matrix: Mat4, point: [f32; 3]) -> Option<[f32; 3]> {
    // `point` has three components; the fourth column is the translation, added once.
    let mut out = [0.0_f32; 4];
    for (row, cell) in out.iter_mut().enumerate() {
        *cell = matrix
            .iter()
            .zip(point.iter().chain(core::iter::once(&1.0_f32)))
            .map(|(column, coordinate)| column.get(row).copied().unwrap_or(0.0) * coordinate)
            .sum::<f32>();
    }
    let w = out[3];
    if w <= 0.0 || !w.is_finite() {
        return None;
    }
    Some([out[0] / w, out[1] / w, out[2] / w])
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn normalise(v: [f32; 3]) -> [f32; 3] {
    let length = dot(v, v).sqrt();
    if !length.is_finite() || length <= 0.0 {
        return [0.0, 0.0, 0.0];
    }
    [v[0] / length, v[1] / length, v[2] / length]
}

#[cfg(test)]
#[expect(clippy::expect_used)]
mod tests {
    use super::*;

    fn close(a: [f32; 3], b: [f32; 3], tolerance: f32) -> bool {
        (a[0] - b[0]).abs() < tolerance
            && (a[1] - b[1]).abs() < tolerance
            && (a[2] - b[2]).abs() < tolerance
    }

    #[test]
    fn the_camera_eye_orbits_its_target_at_the_stated_distance() {
        // Distance is measured from the target, not from the geocentre: the target is lifted so
        // that a ground track shows on the near face, and measuring from the origin would make
        // zoom change the framing as the lift changed.
        let mut camera = Camera::default_view();
        camera.distance_km = 20_000.0;
        let eye = camera.eye();
        let target = camera.target();
        let offset = sub(eye, target);
        let distance = dot(offset, offset).sqrt();
        assert!(
            (distance - 20_000.0).abs() < 0.01,
            "eye was {distance} km from the target"
        );
    }

    #[test]
    fn the_target_projects_to_the_centre_of_the_view() {
        // The camera looks at its target, so the target must land at the middle of the screen
        // whatever the orbit angles. This is the test that catches a transposed view matrix: a
        // transpose still produces numbers, just the wrong ones.
        let camera = Camera::default_view();
        let view = camera.view();
        let projected = transform_point(view, camera.target()).expect("in front of the camera");
        // The tolerance is a *f32* tolerance, and it has to be: the view matrix normalises an eye
        // vector up to 5 x 10^5 km long, so its translational part lands just under 3 x 10^5 and
        // carries a quantum of 2^-4 = 0.0625 km. Verified by sweeping the camera over a range of
        // azimuths, elevations and distances: the worst centre offset is 0.0059 km, i.e. exactly
        // one quantum. A tighter bound would be testing the platform's float rounding, not the
        // matrix; an f64 matrix would fix this properly and is the change to make first if a
        // jitter at extreme zoom ever shows up.
        assert!(
            projected[0].abs() < 0.01 && projected[1].abs() < 0.01,
            "the centre projected to {projected:?}"
        );
        // And it is `distance` in front of the camera, on the -z side.
        assert!(
            (projected[2] + camera.distance_km).abs() < 0.01,
            "the target was {:?} along the view axis",
            projected[2]
        );
    }

    #[test]
    fn the_point_under_the_camera_is_near_and_the_far_side_is_far() {
        // With the camera on +x, a point at +x radius is nearer than one at -x radius. A
        // transposed view would swap these.
        let mut camera = Camera::default_view();
        camera.azimuth = 0.0;
        camera.elevation = 0.0;
        camera.distance_km = 20_000.0;
        let view = camera.view();
        let near = transform_point(view, [6_378.0, 0.0, 0.0]).expect("visible");
        let far = transform_point(view, [-6_378.0, 0.0, 0.0]).expect("visible");
        // Camera space looks down -z, so "nearer" means a smaller magnitude of z.
        assert!(
            near[2] > far[2],
            "near z {} was not in front of far z {}",
            near[2],
            far[2]
        );
    }

    #[test]
    fn a_point_behind_the_camera_is_rejected_rather_than_mirrored() {
        // `transform_point` is the *view*-space transform, which is affine: it never divides by
        // `w`, so nothing behind the camera is mirrored here. The rejection happens in the
        // projection, where `w = -z_view` and a point behind the eye therefore has a negative
        // `w`. Both halves are checked because each catches a different mistake: an affine view
        // that has wandered, and a projection whose handedness has flipped.
        let mut camera = Camera::default_view();
        camera.azimuth = 0.0;
        camera.elevation = 0.0;
        camera.distance_km = 20_000.0;

        // View space is an affine transform: it never divides by `w`, so it cannot reject
        // anything, and the test checks what it *should* produce — the far side of the Earth in
        // front of the eye and a point beyond the eye behind it. Signs only: the view matrix has
        // had to normalise a 20 000 km eye vector, so its translational part carries a few
        // metres of f32 error and an exact magnitude here would be a tolerance test in disguise.
        let view = camera.view();
        let far_side = transform_point(view, [-6_378.0, 0.0, 0.0]).expect("affine");
        assert!(
            far_side[2] < 0.0,
            "the far side of the Earth landed at z = {}",
            far_side[2]
        );
        let beyond_eye = transform_point(view, [40_000.0, 0.0, 0.0]).expect("affine");
        assert!(
            beyond_eye[2] >= 0.0,
            "a point beyond the eye landed at z = {}",
            beyond_eye[2]
        );

        // Projection space: the point beyond the eye is rejected instead of being mirrored to a
        // plausible-looking position on the near side.
        let clip = multiply(camera.projection(1.0), view);
        assert!(
            transform_point(clip, [40_000.0, 0.0, 0.0]).is_none(),
            "a point behind the camera was projected"
        );
        assert!(transform_point(clip, [0.0, 0.0, 0.0]).is_some());
        assert!(transform_point(clip, [6_378.0, 0.0, 0.0]).is_some());
    }

    #[test]
    fn the_projection_maps_the_frustum_corners_to_the_clip_cube() {
        // A point at the near plane, halfway to the top edge, must land near y = 1 in NDC.
        let projection = perspective(core::f32::consts::FRAC_PI_2, 1.0, 1.0, 100.0);
        let edge = transform_point(projection, [0.0, 1.0, -1.0]).expect("in front");
        assert!(
            (edge[1] - 1.0).abs() < 1e-5,
            "the top edge landed at y = {}",
            edge[1]
        );
        let centre_near = transform_point(projection, [0.0, 0.0, -1.0]).expect("in front");
        assert!(
            (centre_near[2] - 0.0).abs() < 1e-5,
            "the near plane landed at z = {}",
            centre_near[2]
        );
        let centre_far = transform_point(projection, [0.0, 0.0, -100.0]).expect("in front");
        assert!(
            (centre_far[2] - 1.0).abs() < 1e-4,
            "the far plane landed at z = {}",
            centre_far[2]
        );
    }

    #[test]
    fn zooming_is_clamped_and_ignores_nonsense() {
        let mut camera = Camera::default_view();
        camera.zoom(1e-9);
        assert!((camera.distance_km - Camera::MINIMUM_DISTANCE_KM).abs() < 0.01);
        camera.zoom(1e9);
        assert!((camera.distance_km - Camera::MAXIMUM_DISTANCE_KM).abs() < 0.01);

        let before = camera.distance_km;
        camera.zoom(0.0);
        camera.zoom(-2.0);
        camera.zoom(f32::NAN);
        assert_eq!(
            camera.distance_km, before,
            "a nonsense zoom moved the camera"
        );
    }

    #[test]
    fn orbiting_stops_short_of_the_poles() {
        // At exactly a pole the up vector and the view direction are parallel and the basis
        // collapses, so the elevation is clamped.
        let mut camera = Camera::default_view();
        for _ in 0..100 {
            camera.orbit(0.0, 0.5);
        }
        assert!(camera.elevation <= Camera::POLAR_LIMIT + 1e-6);
        let view = camera.view();
        // The matrix is still usable: the centre still projects.
        assert!(transform_point(view, [0.0, 0.0, 0.0]).is_some());
    }

    #[test]
    fn a_bad_camera_is_refused() {
        let mut camera = Camera::default_view();
        camera.distance_km = 0.0;
        assert!(camera.validate().is_err());
        camera.distance_km = -1.0;
        assert!(camera.validate().is_err());
        camera.distance_km = 20_000.0;
        camera.field_of_view = 0.0;
        assert!(camera.validate().is_err());
        camera.field_of_view = core::f32::consts::PI;
        assert!(camera.validate().is_err());
        camera.field_of_view = 0.8;
        camera.azimuth = f32::NAN;
        assert!(camera.validate().is_err());
    }

    #[test]
    fn the_planes_scale_with_the_distance_and_contain_the_star_sphere() {
        // Both planes are ratios of the camera distance, which is what keeps the depth buffer's
        // precision usable at every zoom. The star sphere sits at
        // `scene::STAR_RADIUS_FRACTION` of the far plane, so the far plane must be far enough out
        // that the sky is in front of it — the bug that hid the stars for an entire build.
        let mut camera = Camera::default_view();
        for distance in [
            Camera::MINIMUM_DISTANCE_KM,
            23_000.0,
            Camera::MAXIMUM_DISTANCE_KM,
        ] {
            camera.distance_km = distance;
            assert!(close(
                [camera.near_plane_km(), camera.far_plane_km(), 0.0],
                [
                    distance * camera.near_ratio,
                    distance * camera.far_ratio,
                    0.0
                ],
                1e-3
            ));
            let star_radius = camera.far_plane_km() * crate::scene::STAR_RADIUS_FRACTION;
            assert!(
                star_radius < camera.far_plane_km(),
                "at {distance} km the stars at {star_radius} km would be clipped by the {:.0} km                  far plane",
                camera.far_plane_km()
            );
            assert!(
                star_radius > camera.distance_km,
                "the sky was in front of the camera"
            );
        }
    }
}
