//! Whether the Earth is between a site and a target.
//!
//! A satellite below the horizon is not visible even when a sensor would otherwise accept it:
//! the line of sight passes through the planet. This is the test that makes the elevation
//! comparison honest, and it is the one discontinuity the window finder has to handle, because
//! visibility through the Earth can end at a moment when the elevation is nowhere near the
//! mask.

use crate::ellipsoid::Ellipsoid;

/// How far inside the body a segment may pass before it counts as occulted, in kilometres.
///
/// A millimetre of clearance is not a physical distinction; it exists so a segment that grazes
/// the surface exactly is not called occluded by floating-point noise.
const SEGMENT_EPSILON: f64 = 1e-9;

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
    // 6371.0008 km for WGS-84.
    //
    // This is a normalised sphere, not a claim about the Earth's shape. The actual crossing
    // test below asks whether the *segment* leaves the station's own radius and re-enters the
    // sphere, which is what makes it correct for a station at ANY latitude -- see the note
    // there about why the naive version fails above about 40 degrees.
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

    // The closest approach is no longer needed: the crossing test below answers the question
    // directly, and is the only form that is correct for a station inside the sphere.
    let _ = t_star;

    // The naive test is `distance < radius`, and it is WRONG for a station whose own radius is
    // already below the sphere's. On WGS-84 a station above about 40 degrees latitude sits at
    // radius 6367.5 km or less -- inside the 6371.0 km sphere -- so the closest approach of ANY
    // segment from it is its own radius, the test always answers "occulted", and no satellite
    // is ever visible from a mid- or high-latitude station.
    //
    // The correct question is whether the segment *crosses* the sphere: does it leave the
    // surface and come back through? For a station outside the sphere the two agree, which is
    // why the equator cases passed and hid the bug.
    //
    // Solve |site + t*delta|^2 = radius^2, i.e. a t^2 + 2b t + c = 0 with a = |delta|^2,
    // b = site . delta, c = |site|^2 - radius^2. The two roots are where the *line* crosses
    // the sphere's surface; the segment is occluded when it passes through the body, which
    // means both crossings lie strictly inside (0, 1).
    //
    // Note the `c < 0` case: a station inside the sphere starts inside the body. Its segment
    // then has one negative root (behind the station) and at most one exit, so `near < 0 < far`
    // and the condition below is correctly false. That single case is why the earlier
    // closest-approach form failed, and why the roots are compared rather than the distance.
    let site_squared: f64 = site.iter().map(|c| c * c).sum();
    let discriminant =
        site_dot_delta * site_dot_delta - length_squared * (site_squared - radius * radius);
    if discriminant <= 0.0 {
        // No real root: the line never reaches the sphere at all.
        return false;
    }
    let root = discriminant.sqrt();
    let enter = (-site_dot_delta - root) / length_squared;
    let leave = (-site_dot_delta + root) / length_squared;

    // Where along the segment the line comes closest to the geocentre. For a station *outside*
    // the sphere this is what the old test used, and it is correct there.
    let closest_at = (-site_dot_delta / length_squared).clamp(0.0, 1.0);

    // `inside` is true for every station above about 40 degrees latitude on WGS-84, because the
    // volumetric sphere's radius (6371.0 km) is larger than those stations' own radius (6367.5
    // km at 45 degrees). That single fact is what the first implementation missed: it asked
    // whether the closest approach was within the sphere, which for such a station is its own
    // radius, so every segment looked occulted.
    let inside = site_squared < radius * radius;

    if !inside {
        // Outside the sphere: the body is in the way only if the segment passes in through one
        // side and out through the other.
        return enter > SEGMENT_EPSILON && leave < 1.0;
    }

    // Inside the sphere the segment starts within the body, so "did it enter" is meaningless.
    // The body is in the way exactly when the segment dips closer to the geocentre than the
    // station already is: the station sits at the body's local surface, and anything between it
    // and a target beyond must come from dipping further in.
    //
    // Overhead, the closest approach IS the station's own radius, so nothing is between.
    // Towards the antipode the line passes near the centre, so the planet is squarely in the
    // way. Comparing the closest approach against the station's own radius expresses that
    // without needing to know where along the segment the crossings fall.
    let closest = {
        let mut point = [0.0_f64; 3];
        for (c, (s, d)) in point.iter_mut().zip(site.iter().zip(delta.iter())) {
            *c = s + closest_at * d;
        }
        point.iter().map(|c| c * c).sum::<f64>().sqrt()
    };
    let site_radius = site_squared.sqrt();
    closest < site_radius - SEGMENT_EPSILON
}

#[cfg(test)]
#[expect(clippy::expect_used)]
mod tests {
    use super::*;

    const EARTH: Ellipsoid = Ellipsoid::WGS84;

    /// The Earth-fixed position of a station at a latitude on the prime meridian.
    fn ground_site(latitude_deg: f64) -> [f64; 3] {
        let site = crate::site::GroundSite::new(
            stkcraft_core::Angle::from_deg(latitude_deg),
            stkcraft_core::Angle::from_deg(0.0),
            0.0,
        )
        .expect("valid site");
        site.to_itrf(&EARTH).expect("valid position")
    }

    /// A point on the equator at the prime meridian, on the ellipsoid surface.
    const SURFACE: [f64; 3] = [6_378.137, 0.0, 0.0];

    #[test]
    fn a_high_latitude_station_can_still_see_its_satellites() {
        // Regression. The first implementation asked whether the segment's closest approach was
        // inside a sphere of volumetric radius, 6371.0008 km. A station at 45 N sits at radius
        // 6367.49 km -- already INSIDE that sphere -- so the closest approach of any segment from
        // it was its own radius, every segment was called occulted, and no satellite was ever
        // visible from a station above about 40 degrees latitude. The equator tests all passed,
        // which is exactly why the bug reached the engine.
        for latitude in [45.0, 60.0, 75.0, 89.0] {
            let site = ground_site(latitude);
            // Directly overhead: unambiguously visible.
            let overhead = [site[0] * 1.1, site[1] * 1.1, site[2] * 1.1];
            assert!(
                !is_occulted(&EARTH, site, overhead),
                "a satellite above a {latitude} degree station was called occulted"
            );

            // ... and the antipode is still blocked, so the fix did not simply disable the test.
            // The antipode is a point well outside the Earth on the opposite side, not a
            // mirror of the station's own radius: the station is inside the volumetric sphere,
            // so mirroring it would land inside the body and test nothing.
            let antipode_radius = Ellipsoid::WGS84.semi_major_km + 500.0;
            let scale = -antipode_radius
                / (site[0] * site[0] + site[1] * site[1] + site[2] * site[2]).sqrt();
            let antipode = [site[0] * scale, site[1] * scale, site[2] * scale];
            assert!(
                is_occulted(&EARTH, site, antipode),
                "the antipode of a {latitude} degree station was called visible"
            );
        }
    }

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
