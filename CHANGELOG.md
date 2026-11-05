# Changelog

All notable changes to STKCraft are recorded here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Versions in the `0.0.x` series are early alpha. Each one states what it can and cannot do,
so a reader is never left guessing whether a missing capability is a bug or simply not built
yet.

## [Unreleased]

Nothing yet.

## [0.0.5] — When can this satellite see that station

The product's central question is now answerable. A new crate, `stkcraft-access`, turns a
station, a sensor and a target position over time into **access windows with start and end
times**, and every piece of it is held to a case that can be checked by hand.

It does not yet connect to a two-line element set: the target position is supplied by the
caller. Joining SGP4 to this is the engine's job and is the next arc.

### Added

- **`stkcraft-access` at L3**, above `stkcraft-frames`. Six modules, each independently tested:
  - `ellipsoid` — the WGS-84 Earth, so a station is where it really is rather than where a
    sphere would put it. The two defining constants are written down and everything else is
    derived, because a derived constant typed from memory is a transcription error waiting to
    happen.
  - `site` — a geodetic station and its local east/north/up axes, with `up` along the
    **ellipsoid normal** rather than the geocentric radius.
  - `geometry` — elevation, azimuth and range from a station to a target, azimuth clockwise
    from north.
  - `sensor` — a horizon mask, a field-of-view cone and an optional range limit.
  - `occultation` — whether the Earth is in the way.
  - `window` — the window finder, and `problem` — the end-to-end computation.
- **`find_windows`** brackets visibility changes and bisects them to a millisecond. It does
  not sample and threshold: sampling snaps every boundary to the sample grid, so the reported
  start time changes when the step changes, and it loses any pass shorter than the step.
- **`docs/plans/2026-11-05-access-geometry.md`** records the arc, its reference values and the
  reasoning behind each layer.

### Correctness

Every claim names the test that holds it.

- **The acceptance case from `AGENTS.md` holds end to end.** A satellite 500 km directly
  overhead is visible; one below the surface is not. (`a_satellite_directly_overhead_is_visible_and_one_below_the_horizon_is_not`)
- **A sensor cannot see through the Earth.** Even a mask of `−90°` with a `180°` cone reports
a satellite on the far side as not visible, because occultation is checked separately from the
  sensor. (`a_satellite_on_the_far_side_is_rejected_even_at_high_elevation`)
- **A window boundary is a root, not a sample.** The boundary is placed at `1234.5678 s`,
  nowhere near a `60 s` sample, and must be recovered to `1 ms`. This is the test that fails for
  a sampling implementation. (`the_boundary_is_a_root_not_a_sample`)
- **A pass shorter than the step is still found** when it straddles a sample.
  (`a_short_window_between_two_samples_is_found_when_it_straddles_one`)
- **The cone is round, not square.** With the boresight north at 45°, a point 4° off in
  azimuth and 4° off in elevation is 4.84° away on the sky — outside a 4.5° cone, inside a
  4.5° box. (`the_cone_is_round_not_square`)
- **The local frame is the ellipsoid normal, not the radius.** A target along the geocentric
  radius at 45° N sits at `90° − 0.1924°` elevation, not at 90°.
  (`the_up_axis_being_the_normal_matters`)
- **A station at the North Pole sits on the z axis at the polar radius**, which is the test
  that fails if the `(1 − e²)` factor is dropped from the z term.
  (`a_site_at_the_north_pole_is_on_the_z_axis_at_the_polar_radius`)
- **The sidereal, occultation and geometry layers are each checked against computed values**,
  not recalled ones — see the commit messages for the cases where a recalled figure was wrong.

### Verification

- `cargo xtask ci --offline` passes all six gates: fmt, clippy `-D warnings`, **170 tests**
  workspace-wide, assets, layers, and the wasm build of all five portable crates.
- 49 of those tests are in `stkcraft-access`, a new crate with no test debt.

### Known limitations

- **No propagation in this crate.** The target position arrives as a closure. Connecting a
  two-line element set through SGP4 and the frame chain is the engine's job.
- **The line of sight is geometric.** No atmospheric refraction, no light time, no aberration.
  Refraction is zero above about `5°` and half a degree at the horizon, so a mask near `0°` is a
  geometric horizon, not an optical one. Both omissions are named in the crate docs.
- **No antenna gain pattern.** The cone is a hard cutoff; a real dish has a main lobe and
  sidelobes.
- **The Earth occultation test uses a sphere of volumetric radius** (`6371.0008 km`) rather
  than the ellipsoid, because the difference moves a grazing elevation by `0.06°`. The
  elevation test itself uses the exact ellipsoid.
- **No ground coverage grids, no chains, no access between two satellites.** Only
  satellite-to-ground access is implemented.
- **Still no engine, no scenario document, no viewport and no UI.**

## [0.0.4] — From a TEME vector to a usable position

0.0.3 could compute where a satellite is but nothing could use the answer: SGP4 is defined in
TEME, a frame tied to the mean equinox of date, so the position was not comparable with a
ground station, a star or a 3D view. `stkcraft-frames` now carries the whole conversion
chain, and both of its endpoints reproduce the published worked example to `1e-6 km`.

### Added

- **`stkcraft-frames` gains the frame chain, in five tested layers.** Each is a separate module
  with its own reference, so a mistake is caught where it is made rather than at the end:
  - `arguments` — the five IAU-80 Delaunay arguments, checked against ERFA and Vallado to
    `1e-13 rad`.
  - `nutation` — the 106-term IAU-80 series and the mean obliquity. The two references agree
    with each other to `3e-15 rad`, and this crate agrees with both to `1e-14`.
  - `precession` — the IAU-77 angles and matrix, matching the reference matrix to fifteen
    digits.
  - `sidereal` — Greenwich mean sidereal time, matching the reference to `1e-12 rad`.
  - `teme` — the composite conversions, reproducing the worked example to `1e-6 km`.
- **`teme_to_gcrf`.** TEME to the inertial frame as `P · N · E`, where `E` is the geometric
  part of the equation of the equinoxes. This is the frame an ephemeris, a star field or a
  coverage grid should be expressed in.
- **`teme_to_itrf`.** TEME to the Earth-fixed frame as `W · R3(gmst)`, including the
  `ω × r` term that distinguishes a rotating-frame velocity from a merely rotated one. That
  term is about `0.5 km/s` at low Earth orbit, so a conversion that omitted it would look
  right and be badly wrong.
- **`CartesianState`, `EarthOrientation` and `EquinoxTerms`.** `EarthOrientation` never
  defaults silently: `negligible()` is named so that ignoring `UT1 − UTC` and polar motion is
  a visible decision, because the omission costs about a kilometre of ground position.
  `EquinoxTerms` is an explicit argument because the standard changed in 1997 and the wrong
  branch costs about 15 m.
- **`docs/plans/2026-11-05-frames-teme-conversion.md`** records the arc, the reference values
  and the reasoning for each layer.

### Correctness

Every claim names the test that holds it.

- **The GCRF conversion reproduces the published example** to `1e-6 km` in position and
  velocity. (`teme_to_gcrf_reproduces_the_worked_example`)
- **The Earth-fixed conversion reproduces both of its published references** — the kinematic
  terms that apply after 1997 and the geometric ones that applied before — to `1e-6 km`.
  (`teme_to_itrf_with_kinematic_terms_reproduces_the_worked_example`,
  `teme_to_itrf_with_geometric_terms_reproduces_its_own_reference`)
- **The Earth-fixed velocity really is Earth-fixed.** A test compares it against a plain
  rotation and fails if the `ω × r` term is missing. (`the_earth_fixed_velocity_carries_the_rotation_term`)
- **A frame change preserves the radius,** and an inertial one preserves the speed. This is
  the only check that catches a transposed rotation without a reference table at hand.
  (`a_frame_change_preserves_radius_and_speed`)
- **The 106-term nutation table is the published one.** The transcription was diffed field by
  field against its source: 106 rows, zero mismatches. The test also asserts the length and
  the first and last rows, so a truncated paste fails. (`the_table_has_all_published_terms`)
- **Nutation stays inside its published envelope** of 20″ in longitude and 10″ in obliquity
  for every year from 1900 to 2100. (`nutation_stays_inside_its_published_envelope`)
- **Precession is exactly the identity at J2000,** and the matrix agrees with the angles it
  was built from. (`precession_is_the_identity_at_j2000`,
  `precession_accumulates_forty_six_arcseconds_a_century`)
- **Impossible Earth orientation is refused rather than applied.** A pole coordinate of a
  radian is a degrees-where-radians-belong mistake and would tilt the Earth by thousands of
  kilometres. (`impossible_earth_orientation_is_refused`)

### Verification

- `cargo xtask ci --offline` passes all six gates: fmt, clippy `-D warnings`, **121 tests**
  workspace-wide, assets, layers, and the wasm build of all four portable crates.
- 41 of those tests are in `stkcraft-frames`, up from 17.

### Known limitations

- **No Earth orientation file loader.** `UT1 − UTC` and the polar motion coordinates are
  caller inputs until an IERS `finals2000A` reader exists. They are validated against the
  IERS ranges, never guessed.
- **The ground-station local horizon is not here yet.** It needs an ellipsoid and a site,
  which belong with the access geometry rather than the frame chain.
- **No single call from TLE to GCRF.** The conversion takes a `CartesianState`; joining it to
  `stkcraft_sgp4`'s output is the engine's job, and there is no engine yet.
- **TEME only in.** There is no inverse (`gcrf_to_teme`) because nothing needs one yet.

## [0.0.3] — Deep-space propagation made exact

Deep-space (SDP4) propagation was the one known-wrong numerical layer in 0.0.2: worst case
about 85 km over the published verification suite. It is now exact — every one of the 33
verification cases reproduces the reference output to within `4e-9 km`, the limit of the eight
decimals that table prints. The tolerance in the test was tightened from the measured 100 km
to 1 mm, so the old error could not survive unnoticed a second time.

### Fixed

Each item names the defect, because the diagnostics that found them are reusable.

- **The epoch passed to `dscom` was the fraction of a day, not the day count.**
  `Elements::sgp4_epoch` computes the whole days since 1950 January 0.0 and the fraction of
the day, but `sgp4init` read only the first return value and used it as `epoch`. The lunar and
solar arguments `day` therefore had no multi-decade ramp: `zmol`/`zmos` were wrong in their
first digit, and every deep-space secular rate built from them drifted. This was the root cause
of the 85 km error. Regression test:
`the_sgp4_epoch_counts_whole_days_from_the_1950_zero`.
- **`JulianDate` held a half-day inside `day`.** `day` was `X.5` for a UTC midnight, so
  subtracting the SGP4 epoch zero from it could not be exact and the whole-day count came out
  half-integral. `day` is now always an integral day count from JD 0, with all fractional and
  half-day content in `fraction`. That invariant is what makes the fix above possible, and it is
  now documented on the type and asserted in `the_epoch_lands_on_the_stated_day`.
- **`dsinit` derived `inclm` as `asin(sin(i))` instead of using the perturbation.** The
  reference assigns the epoch inclination before calling `dsinit`; recomputing it from the
  sine folds every orbit above 90° of inclination into the wrong quadrant, which silently
  disables or mis-applies the 180° guard that drops the node terms.
- **`dspace` negated the Euler–Maclaurin `step2` term for negative time.** The reference uses
  `+259200` in both directions; only the first-order `delt` changes sign. `xni` was therefore
  wrong by twice the second-order term when stepping backwards.
- **`xfact` used the Kozai mean motion instead of the un-Kozai'd one,** for both resonances.
- **The Kepler iteration clamped at 1.0 rather than 0.95,** and used `signum()` where the
  reference chooses `+0.95`/`-0.95` with an explicit comparison. The published case 23333
  exists to exercise this clamp.
- **The lunar secular terms read the wrong `z` set.** `didt`, `dmdt`, `sghl` and `shll` must
  use the lunar `z` values that `dscom` leaves after its second pass, while `sis` uses the
  solar `sz` capture from the first. The reference implementation's parameter names are
  misleading here, so each term is now checked against the reference's own printed values
  rather than against its naming.

### Changed

- **`DEEP_SPACE_TOLERANCE_KM` is `1e-6 km`,** matching the near-Earth bound. A single named
exception (`NEAR_PARABOLIC_TOLERANCE_KM`, `1e-5 km`) covers case 23333, whose `e = 0.97`
multiplies the round-off by a near-unity `1/(1 - e cos E)` while converting to position.
- **The verification test is renamed** from `deep_space_cases_stay_within_the_documented_tolerance`
to `deep_space_cases_reproduce_the_published_output`, since it now asserts accuracy rather
than a bound.

### Verification

- All 33 published cases, near-Earth and deep-space, match `tforverf.out` to `4e-9 km`.
- `cargo xtask ci --offline` passes all six gates: fmt, clippy `-D warnings`, 97 tests,
  assets, layers, and the wasm build of all four portable crates.

The defect localisation used a small C++ harness compiled against the published reference
source to print intermediate `dscom`/`dsinit`/`dspace` values, and compared them term by term
with the Rust implementation. That harness was scratch work and is not committed.

### Known limitations

- **TEME only.** The output frame is still the one SGP4 is defined in; conversion to ICRF and
to an Earth-fixed frame is the next arc.
- **Two error codes differ from the published file, deliberately** — unchanged from 0.0.2 and
documented in that section.

## [0.0.2] — SGP4 and SDP4 propagation

The first version that can compute where a satellite is. It reads a two-line element set and
turns it into an inertial state vector, using the same theory the public element sets are
generated for.

**It is verified against the published reference for near-Earth orbits and is not yet exact
for deep-space orbits** — see *Known limitations* before trusting a high-orbit result.

### Added

- **`stkcraft-sgp4` (L2).** A from-scratch implementation of SGP4 and SDP4, written against
  the published theory: Vallado, Crawford, Hujsak and Kelso, *Revisiting Spacetrack Report 3*,
  AIAA 2006-6753, together with its reference source code, courtesy of CelesTrak, and the
  original Hoots and Roehrich Spacetrack Report 3 (1980). Every constant and every
  intermediate quantity carries the routine it comes from in a comment.
- **A strict TLE parser.** Fixed-column parsing with the checksum enforced, the implied
  decimal and exponent notation (`28098-4` is `0.28098e-4`) decoded, the two-digit epoch year
  resolved with the published 1957–2056 window, and Alpha-5 catalog identifiers carried
  through as text. A malformed line is refused with the line and column named; nothing is
  guessed and nothing is silently defaulted.
- **Typed elements and states.** `Elements` converts once, at one boundary, from the TLE's
  degrees and revolutions per day into the radians and minutes SGP4 is written in.
  `StateVector` carries `Distance` and `Velocity`, not bare `f64`, and is documented as TEME —
  the frame SGP4 is defined in.
- **The reference error codes as a type.** `PropagationError` keeps the reference
  implementation's numbering (1–6) so published logs and test files still read correctly, but
  as an enum so the meaning travels with the value. A decayed, sub-orbital or
  out-of-family element set returns an error; it never returns a plausible-looking number.
- **The full verification suite.** All 33 element sets and all 667 expected rows from
  `tforverf.out` are transcribed into `crates/sgp4/src/tests/vectors/data.rs`, byte for byte,
  with their source named.
- **`cargo xtask`** now checks `stkcraft-sgp4` builds for `wasm32-unknown-unknown`, and the
  crate is registered in the layering table at L2.

### Correctness

Each claim names the test that holds it.

- **Near-Earth propagation matches the published output exactly.** All eight near-Earth
  verification cases reproduce `tforverf.out` to within `4e-9 km` — the limit of what a table
  printed to eight decimals can express. (`near_earth_cases_reproduce_the_published_output`)
- **Deep-space propagation runs the right branches.** Every deep-space case that initializes
  takes the `dspace` path and stays within the documented tolerance; the one-day and half-day
  resonances are both exercised.
  (`deep_space_cases_stay_within_the_documented_tolerance`)
- **Both halves of the theory are covered.** 29 of the 33 cases initialize and both the
  near-Earth and the deep-space branches are exercised; the other four are refusals.
  (`every_case_initializes_or_is_one_of_the_three_broken_sets`)
- **A decayed element set fails rather than lying.** Case 29141 propagates, then reports an
  error before its published run ends. (`a_steadily_decaying_element_set_eventually_fails`)
- **A sub-orbital element set is refused.** Case 28872 has a perigee inside the Earth, so no
  state vector is produced at all. (`a_sub_orbital_element_set_is_refused_at_initialization`)
- **The checksum catches a single transposed digit,** and a mismatch between the two lines'
  catalog numbers is rejected. (`a_flipped_digit_is_rejected_by_the_checksum`,
  `a_mismatched_catalog_number_is_rejected`)
- **A degree/radian mix-up cannot compile,** because `Angle` has no `From<f64>`.

### Known limitations

- **Deep-space propagation is not yet exact.** The near-Earth half of the theory reproduces
  the published output to the last printed digit; the deep-space half is currently within
  about 85 km worst case over the verification suite, and several resonant cases (the
  Molniya-like 12-hour orbits and some one-day ones) are off by tens of kilometres. The
  cause is in the deep-space secular and periodic terms, not in the parser, the epoch
  handling or the near-Earth path, all of which are exact. **Treat a high-orbit position as
  indicative until this is resolved.** The tolerance in the test is set to the current
  accuracy rather than to the intended one, so a regression cannot pass unnoticed.
- **Two error codes differ from the published file, deliberately.** Case 33333 is refused
  with code 5 (sub-orbital) rather than 4, because its perigee is inside the Earth and an
  orbit that does not exist gets no state vector. Case 33335 is documented in the published
  file as exercising code 3, but it propagates cleanly here and in an independent
  transcription of the reference, so it is asserted to succeed.
- **TEME only.** The output frame is the one SGP4 is defined in. Converting to ICRF or to an
  Earth-fixed frame needs precession, nutation and Earth rotation, which belong to
  `stkcraft-frames` and are not wired up yet.
- **WGS-72 only in practice.** `GravityField::Wgs84` exists for reproducing comparison runs,
  but an operational element set propagated with WGS-84 is subtly wrong and the type says so.
- **No access, coverage, scenario document, viewport or UI.** The `stkcraft` binary still
  prints its version and exits.

## [0.0.1] — 2026-11-05

The first version. It establishes the engineering standards and the numerical foundation the
rest of the product is built on. **It cannot yet propagate an orbit, find an access or draw a
3D view**; those are the next three versions.

### Added

- **Workspace and quality gate.** A Cargo workspace (edition 2024, resolver 3) with a single
  version source in `[workspace.package]`, and `cargo xtask` tasks matching the sibling Craft
  apps: `ci`, `fmt`, `layers`, `assets`, `wasm` and `version`.
- **Enforced layering** (`cargo xtask layers`). A crate may depend only on a strictly lower
  layer, and nothing at or below `LAST_NON_UI_LAYER` may depend on egui, eframe, winit or rfd.
  Every workspace member must be registered in `xtask/src/layers.rs`; an unregistered crate
  fails the task.
- **Enforced asset attribution** (`cargo xtask assets`). A file with an asset extension must
  have a row in `ATTRIBUTION.md`, and a row must name a file that exists, so the table cannot
  rot in either direction.
- **`stkcraft-core` (L0).** Typed quantities — `Angle`, `Distance`, `Velocity`, `Mass`,
  `Duration`, `EpochOffset` — with named conversions and no `From<f64>`, so a degrees/radians
  mix-up is a compile error. A single `Error` type whose `Unsupported` variant is the only
  sanctioned way to represent an unfinished feature. A `Vector3` whose `normalized` returns
  `None` for a zero or non-finite vector rather than producing NaN.
- **`stkcraft-time` (L1).** The reference time scales (UTC, TAI, TT, TDB, TCB, TCG, UT1, GPS)
  as a type, the IAU SOFA / ERFA constants with their sources cited, and the published
  `TAI - UTC` leap-second table from the IERS `tai-utc.dat` file.
- **`stkcraft-frames` (L2).** `Rotation3`, which refuses to exist unless its rows are
  orthonormal and its determinant is +1, so a reflection or a scaled matrix cannot be mistaken
  for a rotation.
- **CI wiring.** `.github/workflows/` calls the shared `storytold/craft-common-actions`
  workflows at `@main`; there are no app-local jobs.
- **`AGENTS.md`**, **`docs/architecture.md`**, **`NOTICE`**, **`ATTRIBUTION.md`** and the
  dual MIT / Apache-2.0 licences.

### Correctness

These are the properties the numerical layers guarantee, each with a regression test:

- **Time-scale offsets are exact.** `TT - TAI` is 32.184 s and `TAI - GPS` is 19 s to the
  limit of an `f64`. An earlier implementation subtracted two summed Julian Dates and was
  wrong by about 13 µs; intervals are now computed part by part on the split representation.
- **Sub-microsecond resolution survives.** A one-microsecond step at J2000 is preserved
  through a round trip, which a single-`f64` Julian Date cannot do.
- **Leap-second lookups report their confidence.** A date before 1960 or more than five years
  past the table's revision is flagged rather than presented as a fact.
- **A scenario can carry a newer leap second** through `LeapSecondTable`, without waiting for
  a release, and the built-in table is never mutated by a caller's extension.
- **UT1 conversion refuses to guess.** `DUT1` is published only by IERS, so converting to UT1
  without it is an error, not a defaulted zero.
- **Degree-minute-second formatting carries correctly.** `0°59'59.99999999"` renders as
  `1°00'00"`, never `0°60'00"`.
- **No panics in production code.** Every crate root denies `unwrap`, `expect`, `panic`,
  `unimplemented`, `todo` and `unreachable`, and the workspace forbids `unsafe`.

### Known limitations

- **No propagation yet.** `stkcraft-sgp4` does not exist, so no orbit can be computed.
- **No access or coverage.** The geometry that answers "when can this see that" is not built.
- **No scenario document, viewport or UI.** The `stkcraft` binary prints its version and exits.
- **TDB, TCG and TCB are approximated by TT.** The periodic TDB term (under 2 ms) and the
  TCG/TCB rates are not applied; `Epoch::to_scale` documents this where the offset is added.
- **Converting a uniform-scale epoch back to UTC is not implemented** and returns
  `Error::Unsupported`, because it needs the leap offset at that instant.

[Unreleased]: https://github.com/spacelibres/STKCraft/compare/v0.0.5...HEAD
[0.0.5]: https://github.com/spacelibres/STKCraft/compare/v0.0.4...v0.0.5
[0.0.4]: https://github.com/spacelibres/STKCraft/compare/v0.0.3...v0.0.4
[0.0.3]: https://github.com/spacelibres/STKCraft/compare/v0.0.2...v0.0.3
[0.0.2]: https://github.com/spacelibres/STKCraft/compare/v0.0.1...v0.0.2
[0.0.1]: https://github.com/spacelibres/STKCraft/releases/tag/v0.0.1
