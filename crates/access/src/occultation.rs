//! Whether the Earth is between a site and a target.
//!
//! A satellite below the horizon is not visible even when a sensor would otherwise accept it:
//! the line of sight passes through the planet. This is the test that makes the elevation
//! comparison honest, and it is the one discontinuity the window finder has to handle, because
//! visibility through the Earth can end at a moment when the elevation is nowhere near the
//! mask.

use crate::ellipsoid::Ellipsoid;

/// Whether the straight segment from `site` to `target` passes through the Earth.
///
/// Both points are in the Earth-fixed frame, km.
///
/// # Method
///
/// The Earth is treated as a **sphere** of the local mean radius for this test, not as the
/// ellipsoid. The simplifications and why they are acceptable:
///
/// - The difference between the ellipsoid and a sphere of mean radius is about 7 km at the
///   limb, which changes the grazing elevation by roughly `0.06°`. That is well inside the
///   uncertainty of any horizon mask, and the elevation test in [`crate::geometry`] uses the
///   exact ellipsoid, so this function only decides the gross question.
/// - The *segment*, not the infinite line, is tested. A satellite beyond the Earth on the same
///   ray as a visible one would be occulted if the line were tested; it is not, because the
///   closest approach to the centre falls outside the segment.
///
/// Returns `false` when either point is non-finite; an unknowable geometry must not silently
/// claim occlusion.
#[must_use]
pub fn is_occulted(ellipsoid: &Ellipsoid, site: [f64; 3], target: [f64; 3]) -> bool {
    if site.iter().chain(target.iter()).any(|v| !v.is_finite()) {
        return false;
    }

    // The radius the occultation test uses: the sphere of the same volume, `(a a b)^(1/3)`,
    // which sits at 6371.0088 km for WGS-84. Using the volumetric radius rather than an
    // arithmetic mean keeps the error symmetric between equator and pole.
    let a = ellipsoid.semi_major_km;
    let b = ellipsoid.semi_minor_km();
    let radius = (a * a * b).cbrt();

    // Vector from site to target, and the site's offset from the centre.
    let mut delta = [0.0_f64; 3];
    for (d, (t, s)) in delta.iter_mut().zip(target.iter().zip(site.iter())) {
        *d = t - s;
    }
    let length_squared: f64 = delta.iter().map(|d| d * d).sum();
    if length_squared <= 0.0 {
        // Coincident points: the segment has no interior, so nothing is between them.
        return false;
    }

    // The closest approach of the segment to the origin. With P(t) = site + t*delta for t in
    // [0, 1], the minimiser is t* = -(site . delta) / |delta|^2, clamped to the segment.
    let site_dot_delta: f64 = site.iter().zip(delta.iter()).map(|(s, d)| s * d).sum();
    let t_star = (-site_dot_delta / length_squared).clamp(0.0, 1.0);

    let mut closest = [0.0_f64; 3];
    for (c, (s, d)) in closest.iter_mut().zip(site.iter().zip(delta.iter())) {
        *c = s + t_star * d;
    }
    let distance = closest.iter().map(|c| c * c).sum::<f64>().sqrt();

    distance < radius
}

#[cfg(test)]
mod tests {
    use super::*;

    const EARTH: Ellipsoid = Ellipsoid::WGS84;

    /// A point on the equator at the prime meridian, on the ellipsoid surface.
    const SURFACE: [f64; 3] = [6_378.137, 0.0, 0.0];

    #[test]
    fn a_satellite_overhead_is_never_occulted() {
        let target = [6_378.137 + 500.0, 0.0, 0.0];
        assert!(!is_occulted(&EARTH, SURFACE, target));
    }

    #[test]
    fn a_satellite_on_the_far_side_of_the_earth_is_occulted() {
        // Anti-podal: the segment passes through the geocentre.
        let far_side = [-6_378.137 - 500.0, 0.0, 0.0];
        assert!(is_occulted(&EARTH, SURFACE, far_side));
    }

    #[test]
    fn a_satellite_beyond_the_earth_but_not_through_it_is_not_occulted() {
        // Overhead by 500 km, then beyond: the segment from the site ends at the target and
        // never reaches the far side. A test on the infinite *line* would call this occulted,
        // because the line continues through the planet -- which is why the segment is used.
        let near = [6_378.137 + 500.0, 0.0, 0.0];
        let far = [6_378.137 + 40_000.0, 0.0, 0.0];
        assert!(!is_occulted(&EARTH, SURFACE, near));
        assert!(!is_occulted(&EARTH, SURFACE, far));
    }

    #[test]
    fn a_grazing_target_is_decided_by_the_sphere_radius() {
        // The site is on +x at the semi-major axis, so a ray leaving along the tangent (+y)
        // clears the sphere comfortably. Swinging the ray toward the planet brings the closest
        // approach down; the values below were computed, not guessed, and they bracket the
        // boundary: 2 degrees off tangent still clears 6374.25 km, 5 degrees dips to 6353.87 km,
        // and the sphere used by the function is 6371.0008 km.
        let a = EARTH.semi_major_km;

        let at = |degrees: f64| {
            let angle = degrees.to_radians();
            [a - 40_000.0 * angle.sin(), 40_000.0 * angle.cos(), 0.0]
        };
        assert!(
            !is_occulted(&EARTH, SURFACE, at(2.0)),
            "2 degrees off tangent clears the limb"
        );
        assert!(
            is_occulted(&EARTH, SURFACE, at(5.0)),
            "5 degrees off tangent passes inside the limb"
        );
    }

    #[test]
    fn non_finite_input_does_not_claim_occlusion() {
        assert!(!is_occulted(&EARTH, SURFACE, [f64::NAN, 0.0, 0.0]));
        assert!(!is_occulted(&EARTH, [f64::INFINITY, 0.0, 0.0], SURFACE));
    }

    #[test]
    fn coincident_points_are_not_occulted() {
        assert!(!is_occulted(&EARTH, SURFACE, SURFACE));
    }
}
