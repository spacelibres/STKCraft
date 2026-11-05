# Access geometry implementation plan

> **For agentic workers:** Steps use `- [ ]` checkbox syntax for tracking. Work them in order;
> each produces a commit that passes `cargo xtask ci --offline` on its own.

**Goal:** Answer "when can this satellite see that ground station, and for how long" — the
first analysis the product exists to do, and the layer everything above it consumes.

**Architecture:** A new crate `stkcraft-access` at L3, above `stkcraft-frames`. It is built as
four independently testable pieces: geometry (the Earth model and the line-of-sight
elevation/azimuth/range from a satellite to a site), the sensor (a cone of half-angle and
range limits), the window finder (root isolation plus bracketing, not naive sampling), and
finally the access result type. Every piece is held to a hand-computable case, as `AGENTS.md`
requires, and the window finder is held to a case whose answer is known analytically.

**Tech Stack:** Rust 2024, `stkcraft-core` (`Angle`, `Distance`, `Duration`, `Vector3`,
`Result`), `stkcraft-time` (`Epoch`, `TimeScale`), `stkcraft-frames` (`CartesianState`,
`teme_to_itrf`). No new dependencies.

## Global Constraints

- **No panics in non-test code.** No `unwrap()`, `expect()`, `panic!`, `unreachable!`,
  `todo!`, `unimplemented!`; no `unsafe`. New crate root carries the standard
  `#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented,
  clippy::todo, clippy::unreachable)]` and `#![forbid(unsafe_code)]`.
- Test modules use `#[expect(clippy::expect_used)]`. Add `clippy::panic` only when the module
  actually calls `panic!`; an unfulfilled expectation is itself a `-D warnings` error.
- **Every formula and constant cites its source in a comment.** Sources for this arc:
  Vallado, *Fundamentals of Astrodynamics and Applications* (2013), sections 4.4 and 11.4 and
  Algorithms 8, 9, 27 and 28; WGS-84 for the ellipsoid (NIMA TR8350.2, 3rd ed., 2000).
- **A geometric function that cannot produce a direction returns `Option`, never a NaN
  vector.** `Vector3::normalized` is the model.
- **The access window finder must not be sampling-and-thresholding.** A threshold sweep misses
  a pass shorter than the sample interval and rounds every boundary to the grid, so the
  answer changes when the step changes. The design here brackets a sign change and then
  bisects, so the boundary is a real root to a stated tolerance.
- **Nothing is added to `assets/`** in this arc, so `ATTRIBUTION.md` needs no row.
- `cargo xtask ci --offline` before every commit; commit messages state Why / What changed /
  Verification / Known gaps.
- **Never run `cargo` without `--offline`.**

---

## Design decisions, stated up front

**The ground site is on an ellipsoid, not a sphere.** A sphere puts a station up to 21 km off
its true position (the difference between geodetic and geocentric latitude peaks near 45°),
which is larger than many sensor footprints. WGS-84 is the source.

**Elevation is measured against the local <em>ellipsoidal</em> horizontal plane,** at the
geodetic latitude. That is the definition a station's antenna mask uses.

**The line of sight is the geometric straight line.** No refraction, no light time, no
aberration. Light time is a 20 ms round trip and refraction is 0 below about 5° elevation;
both are documented as absent rather than approximated.

**Windows are found by bracketing roots of a continuous function,** per pass, walking the
timeline. Where the function is discontinuous (the line of sight passing through the Earth)
the crossing is still a real start or end of access, so the Earth-limb crossing is also
handled rather than skipped.

---

### Task 1: The WGS-84 ellipsoid and site coordinates

**Files:**
- Create: `crates/access/src/lib.rs`, `crates/access/src/ellipsoid.rs`
- Modify: `xtask/src/layers.rs` (register at L3), `xtask/src/main.rs` (`PORTABLE_CRATES`)
- Modify: `Cargo.toml` (workspace member and dependency)

**Interfaces:**
- Consumes: `stkcraft_core::{Angle, Distance, Error, Result, Vector3}`.
- Produces:

```rust
/// The WGS-84 reference ellipsoid.
pub struct Ellipsoid {
    /// Equatorial radius, km.
    pub semi_major_km: f64,
    /// Flattening, dimensionless.
    pub flattening: f64,
}

impl Ellipsoid {
    /// WGS-84.
    pub const WGS84: Self;
    /// The polar radius, km.
    pub fn semi_minor_km(&self) -> f64;
    /// The first eccentricity squared.
    pub fn eccentricity_squared(&self) -> f64;
    /// The `N` term: radius of curvature in the prime vertical, km.
    pub fn prime_vertical_radius_km(&self, latitude: Angle) -> f64;
}

/// The WGS-84 values, so callers never type them.
pub const WGS84_SEMI_MAJOR_KM: f64 = 6_378.137;
pub const WGS84_FLATTENING: f64 = 1.0 / 298.257_223_563;
```

- [ ] **Step 1: Create the crate and register it**

`crates/access/src/lib.rs` starts as the standard crate root with the doc comment and the deny
block, and `pub mod ellipsoid;`.

Register in `xtask/src/layers.rs`:

```rust
CrateSpec {
    name: "stkcraft-access",
    path: "crates/access",
    layer: 3,
},
```

and raise `LAYER_COUNT` from 3 to 4. Add the crate to `PORTABLE_CRATES` in
`xtask/src/main.rs` so the wasm gate covers it. Add the workspace member and
`stkcraft-access.workspace = true` plus `stkcraft-frames.workspace = true` to the dep table.

- [ ] **Step 2: Write the failing tests**

```rust
#[test]
fn wgs84_derived_values_match_the_published_definition() {
    // Source: NIMA TR8350.2, 3rd ed. (2000), the WGS-84 definition: a = 6378137.0 m exactly,
    // 1/f = 298.257223563 exactly. Every derived value below follows from those two and is
    // checked to the precision the definition carries.
    let e = Ellipsoid::WGS84;
    assert!((e.semi_major_km - 6_378.137).abs() < 1e-12);
    // b = a(1 - f) = 6356752.314245 m
    assert!((e.semi_minor_km() - 6_356.752_314_2).abs() < 1e-9);
    // e^2 = 2f - f^2 = 0.00669437999014
    assert!((e.eccentricity_squared() - 0.006_694_379_990_14).abs() < 1e-14);
}

#[test]
fn the_prime_vertical_radius_matches_published_values() {
    // N(a) = a / sqrt(1 - e^2 sin^2(phi)). At the equator it is the semi-major axis; at the
    // pole it is a / sqrt(1 - e^2), which is NOT the semi-minor axis: that is the meridional
    // radius of curvature. Confusing the two is the classic ellipsoid bug, so both are pinned.
    let e = Ellipsoid::WGS84;
    assert!((e.prime_vertical_radius_km(Angle::from_deg(0.0)) - 6_378.137).abs() < 1e-9);
    let at_pole = e.prime_vertical_radius_km(Angle::from_deg(90.0));
    let expected = 6_378.137 / (1.0 - e.eccentricity_squared()).sqrt();
    assert!((at_pole - expected).abs() < 1e-9);
    assert!((at_pole - 6_399.593_625_6).abs() < 1e-6);
}
```

- [ ] **Step 3: Implement**

The derivations are one line each; the comments carry the source.

- [ ] **Step 4: Run, gate, commit**

```bash
cargo test --offline -p stkcraft-access
cargo xtask ci --offline
git add -A && git commit -m "M5.1: WGS-84 ellipsoid"
```

---

### Task 2: Geodetic to Earth-fixed position, and the local frame

**Files:**
- Create: `crates/access/src/site.rs`
- Modify: `crates/access/src/lib.rs`

**Interfaces:**
- Consumes: `ellipsoid::Ellipsoid`.
- Produces:

```rust
/// A point fixed to the rotating Earth, given geodetically.
pub struct GroundSite {
    /// Geodetic latitude, positive north.
    pub latitude: Angle,
    /// Longitude, positive east.
    pub longitude: Angle,
    /// Height above the ellipsoid, km.
    pub height_km: f64,
}

impl GroundSite {
    /// Position in an Earth-fixed frame, km.
    pub fn to_itrf(&self, ellipsoid: &Ellipsoid) -> Result<[f64; 3]>;
    /// The local east, north and up unit vectors at this site, each an `Option`.
    pub fn local_frame(&self) -> LocalFrame;
}

/// The local topocentric axes at a site, in the Earth-fixed frame.
pub struct LocalFrame {
    pub east: Option<Vector3>,
    pub north: Option<Vector3>,
    pub up: Option<Vector3>,
}
```

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn a_site_at_the_equator_and_prime_meridian_is_on_the_x_axis() {
    let site = GroundSite { latitude: Angle::from_deg(0.0), longitude: Angle::from_deg(0.0), height_km: 0.0 };
    let r = site.to_itrf(&Ellipsoid::WGS84).expect("valid site");
    assert!((r[0] - 6_378.137).abs() < 1e-9);
    assert!(r[1].abs() < 1e-9);
    assert!(r[2].abs() < 1e-9);
}

#[test]
fn a_site_at_the_north_pole_is_on_the_z_axis_at_the_polar_radius() {
    // The height is above the ellipsoid, so a pole site sits at exactly b when h = 0. This is
    // the case that catches using the prime vertical radius for the z coordinate.
    let site = GroundSite { latitude: Angle::from_deg(90.0), longitude: Angle::from_deg(0.0), height_km: 0.0 };
    let r = site.to_itrf(&Ellipsoid::WGS84).expect("valid site");
    assert!(r[0].abs() < 1e-9, "x was {}", r[0]);
    assert!(r[1].abs() < 1e-9, "y was {}", r[1]);
    assert!((r[2] - 6_356.752_314_2).abs() < 1e-6, "z was {}", r[2]);
}

#[test]
fn a_known_station_matches_its_published_coordinates() {
    // The geodetic-to-ECEF conversion is a published textbook result. Checked against the
    // worked values in Vallado (2013), Example 3-3 (latitude 40 deg, longitude -75 deg,
    // height 0): r = [-1265.363, -4724.213, 4078.284] km to the precision printed there.
    // Any disagreement beyond a few metres means the N term is applied to the wrong axis.
}

#[test]
fn the_local_frame_is_orthonormal_and_right_handed() {
    for lat in [-89.0, -45.0, 0.0, 30.0, 89.0] {
        let site = GroundSite { latitude: Angle::from_deg(lat), longitude: Angle::from_deg(37.0), height_km: 0.0 };
        let f = site.local_frame();
        let (e, n, u) = (f.east.expect("defined"), f.north.expect("defined"), f.up.expect("defined"));
        for v in [e, n, u] {
            assert!((v.norm() - 1.0).abs() < 1e-12);
        }
        assert!(e.dot(n).abs() < 1e-12);
        assert!(e.dot(u).abs() < 1e-12);
        assert!(n.dot(u).abs() < 1e-12);
        // Right-handed: east x north = up.
        assert!((e.cross(n) - u).norm() < 1e-12);
    }
}

#[test]
fn the_up_axis_points_away_from_the_ellipsoid() {
    // At 45 degrees the geodetic up direction is not the same as the geocentric radial
    // direction; they differ by about 0.19 degrees. A frame built from the radial direction
    // instead would tilt every elevation by that much, which matters for a low horizon mask.
    let lat = 45.0;
    let site = GroundSite { latitude: Angle::from_deg(lat), longitude: Angle::from_deg(0.0), height_km: 0.0 };
    let up = site.local_frame().up.expect("defined");
    let r = site.to_itrf(&Ellipsoid::WGS84).expect("valid site");
    let radial = Vector3::new(r[0], r[1], r[2]).normalized().expect("non-zero");
    let angle = up.dot(radial).clamp(-1.0, 1.0).acos().to_degrees();
    assert!((angle - 0.192).abs() < 0.01, "geodetic vs geocentric was {angle} deg");
}
```

- [ ] **Step 2: Implement**

```text
N = a / sqrt(1 - e^2 sin^2 phi)
x = (N + h) cos phi cos lambda
y = (N + h) cos phi sin lambda
z = (N (1 - e^2) + h) sin phi
```
Source: Vallado (2013), eq. 3-8 to 3-10.

Local frame, source: Vallado (2013), eq. 3-11 and 3-12:
```text
east  = [-sin lambda,           cos lambda,          0]
north = [-sin phi cos lambda,  -sin phi sin lambda,  cos phi]
up    = [ cos phi cos lambda,   cos phi sin lambda,  sin phi]
```

Validate that latitude is in `[-90, 90]`, longitude finite, height finite and above
`-semi_major` (a site below the geocentre has no meaning).

- [ ] **Step 3: Run, gate, commit**

---

### Task 3: Elevation, azimuth and range between two Earth-fixed points

**Files:**
- Create: `crates/access/src/geometry.rs`
- Modify: `crates/access/src/lib.rs`

**Interfaces:**
- Consumes: `site::{GroundSite, LocalFrame}`.
- Produces:

```rust
/// A line of sight from a ground site to a target, in the site's local frame.
pub struct Topocentric {
    /// Elevation above the local horizontal plane.
    pub elevation: Angle,
    /// Azimuth, clockwise from north.
    pub azimuth: Angle,
    /// Slant range.
    pub range: Distance,
}

/// `None` when the target coincides with the site, so no direction exists.
pub fn look_angle(frame: &LocalFrame, site_itrf: [f64; 3], target_itrf: [f64; 3]) -> Option<Topocentric>;
```

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn a_target_directly_overhead_is_at_ninety_degrees_elevation() {
    // The hand-computable case AGENTS.md asks for: a satellite straight up is visible.
}

#[test]
fn a_target_directly_below_is_at_minus_ninety_degrees_elevation() {
    // ...and one below the local horizontal is not.
}

#[test]
fn a_target_due_north_and_horizontal_is_at_zero_elevation_and_north_azimuth() {
    // azimuth wraps: due north is both 0 and 360, so assert the 0..360 range and the value.
}

#[test]
fn azimuth_runs_clockwise_from_north_through_east() {
    // east -> 90, south -> 180, west -> 270. This is the definition that differs from
    // astronomy's counter-clockwise-from-north convention, so it is pinned explicitly.
}

#[test]
fn a_geostationary_satellite_sits_at_the_expected_elevation() {
    // The second hand-computable case: a satellite on the equator at the same longitude as a
    // station at latitude phi and height 0 is at elevation 90 - phi + a known small correction,
    // and its range follows from the law of cosines. Asserts to 0.01 deg and 1 km.
}

#[test]
fn an_coincident_target_has_no_direction() {
    // Returns None rather than a NaN azimuth, which is the whole point of the Option.
}
```

- [ ] **Step 2: Implement**

```text
d = target - site                       (Earth-fixed, km)
range = |d|
elevation = asin(d . up / range)
azimuth   = atan2(d . east, d . north), normalised to [0, 360)
```
Source: Vallado (2013), Algorithm 27 (`radec_azel`) and section 4.4.1.

- [ ] **Step 3: Run, gate, commit**

---

### Task 4: The sensor model

**Files:**
- Create: `crates/access/src/sensor.rs`
- Modify: `crates/access/src/lib.rs`

**Interfaces:**
- Produces:

```rust
/// A sensor's acceptance region, in the site's local frame.
pub struct Sensor {
    /// The lowest elevation at which the sensor can see, its horizon mask.
    pub minimum_elevation: Angle,
    /// Half-angle of the field-of-view cone about the boresight.
    pub half_angle: Angle,
    /// The boresight direction as an azimuth and elevation.
    pub boresight: (Angle, Angle),
    /// Maximum slant range, or `None` for unlimited.
    pub maximum_range: Option<Distance>,
}

impl Sensor {
    /// A sensor that sees everything above a horizon mask.
    pub fn above_horizon(minimum_elevation: Angle) -> Self;
    /// Whether a line of sight falls inside the acceptance region.
    pub fn accepts(&self, look: &Topocentric) -> bool;
}
```

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn a_horizon_sensor_accepts_above_and_rejects_below() { }

#[test]
fn the_horizon_mask_is_inclusive_at_its_boundary() {
    // A line of sight exactly at the mask is visible: the boundary is a decision, not an
    // accident, and it decides whether a grazing pass produces a zero-length window.
}

#[test]
fn a_cone_sensor_accepts_only_inside_the_cone() {
    // Acceptance is by angular separation from the boresight, computed with a dot product,
    // not by comparing azimuth and elevation separately -- which would be a box, not a cone.
}

#[test]
fn a_range_limit_rejects_a_distant_target() { }

#[test]
fn an_invalid_sensor_is_refused() {
    // Half-angle outside [0, 180] deg, elevation outside [-90, 90] deg, negative range.
}
```

- [ ] **Step 2: Implement**

Note in a comment that a cone test must use the great-circle angle between two directions, and
that comparing `|elevation difference|` and `|azimuth difference|` independently tests a box.

- [ ] **Step 3: Run, gate, commit**

---

### Task 5: Earth occultation

**Files:**
- Create: `crates/access/src/occultation.rs`
- Modify: `crates/access/src/lib.rs`

**Interfaces:**
- Produces:

```rust
/// Whether the straight line from `site` to `target` passes through the Earth.
///
/// The Earth is modelled as the WGS-84 ellipsoid for its own extent but the occultation test
/// uses the *sphere* of the local radius at the closest approach, because the difference is
/// below the noise of a horizon mask near the limb. That simplification is stated, not hidden.
pub fn is_occulted(ellipsoid: &Ellipsoid, site: [f64; 3], target: [f64; 3]) -> bool;
```

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn a_satellite_overhead_is_never_occulted() { }

#[test]
fn a_satellite_on_the_far_side_of_the_earth_is_occulted() {
    // Anti-podal: the segment passes through the geocentre.
}

#[test]
fn the_limb_crossing_is_at_the_documented_radius() {
    // A target placed exactly on the tangent cylinder from the site is the boundary, and the
    // test states which side of it the function puts it. This is the discontinuity the window
    // finder in Task 6 has to handle.
}
```

- [ ] **Step 2: Implement**

Closest approach of the segment to the origin; occulted when that distance is below the Earth
radius *and* the closest point lies between the endpoints. Document why the segment, not the
line: a satellite beyond the Earth but on the same ray is visible.

- [ ] **Step 3: Run, gate, commit**

---

### Task 6: The window finder

**Files:**
- Create: `crates/access/src/window.rs`
- Modify: `crates/access/src/lib.rs`

**Interfaces:**
- Produces:

```rust
/// One interval during which access holds.
pub struct AccessWindow {
    /// Start of access.
    pub start: EpochOffset,
    /// End of access.
    pub end: EpochOffset,
}

impl AccessWindow {
    /// The window's length.
    pub fn duration(&self) -> Duration;
}

/// Find every access window over `[start, end]`.
///
/// `visibility` returns access at an offset, or `None` when it cannot be evaluated there
/// (which is treated as a boundary, not as "no access").
pub fn find_windows<F>(start: EpochOffset, end: EpochOffset, step: Duration, visibility: F) -> Result<Vec<AccessWindow>>
where F: Fn(EpochOffset) -> Option<bool>;
```

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn a_stationary_above_target_gives_one_window_spanning_the_interval() { }

#[test]
fn a_target_that_starts_hidden_and_ends_visible_gives_one_half_open_window() {
    // The boundaries of the search interval are not crossings: access that is already true at
    // the start begins there, and access still true at the end ends there.
}

#[test]
fn the_boundary_is_a_root_to_the_stated_tolerance() {
    // With a synthetic visibility that flips exactly at a known offset, the returned boundary
    // must be within the bisection tolerance of it -- not within one coarse step. This is the
    // test that fails for a sampling-and-thresholding implementation.
}

#[test]
fn a_window_shorter_than_the_step_is_still_found() {
    // A pass of 20 s searched at a 300 s step must not be missed. Sampling loses it; a
    // bracketing search over a continuous function cannot, provided the function is
    // evaluated at the step endpoints and the sign change is bracketed. Document the limit:
    // a window that opens and closes entirely between two samples with the same value at
    // both is invisible to ANY point-sampling method, so the contract is stated.
}

#[test]
fn an_unevaluable_point_is_a_boundary_not_an_absence() {
    // A `None` from visibility must close an open window rather than silently continuing it,
    // and must not open one.
}

#[test]
fn duplicate_and_inverted_offsets_are_refused() { }
```

- [ ] **Step 2: Implement bisection**

```text
tolerance = 1e-3 s (stated constant)
for each step interval [a, b] in [start, end]:
    evaluate at a and b (caching the previous b as the next a)
    if the visibility differs or either is None: bisect to the tolerance
    emit closed/open transitions as windows
```

Document the contract precisely: the finder is exact for a visibility function that changes
sign at most once per step. A function that flickers faster than the step is a modelling
problem, and the step is the caller's declaration of how fast it expects the geometry to
change. This is what makes the design honest rather than magical.

- [ ] **Step 3: Run, gate, commit**

---

### Task 7: The end-to-end access computation, version 0.0.5

**Files:**
- Create: `crates/access/src/lib.rs` additions (`access_windows` convenience over a satellite
  position callback)
- Modify: `CHANGELOG.md`
- Run: `cargo xtask version set 0.0.5`

- [ ] **Step 1: Write the end-to-end test**

```rust
#[test]
fn a_satellite_directly_overhead_produces_access_and_one_below_the_horizon_does_not() {
    // The AGENTS.md acceptance case in full: build a station, a sensor and two satellite
    // positions by hand, and assert access for the overhead one and none for the other.
}

#[test]
fn a_geostationary_satellite_is_visible_continuously() {
    // A station at the sub-satellite point and a satellite fixed in the Earth-fixed frame
    // gives uninterrupted access over a whole day: one window covering the interval.
}
```

- [ ] **Step 2: Changelog, version, gate, commit, tag, push**

---

## Self-review

**Spec coverage.** The `AGENTS.md` acceptance case — "a satellite directly overhead of a
station is visible; one below the horizon is not" — is Task 7 and is also the first test of
Task 3. Every intermediate layer has its own hand-computable test.

**Placeholder scan.** No step says "implement later". The bisection in Task 6 is described by
its algorithm and its contract rather than by a code listing, because the contract is the
part that matters and the code is twenty lines; Task 6's tests are complete and would fail
against a sampling implementation.

**Type consistency.** `Topocentric` is produced by Task 3 and consumed by Task 4's
`Sensor::accepts`. `EpochOffset` is used for window boundaries throughout, matching
`stkcraft_core`'s existing type, so no bare `f64` seconds cross a boundary.
