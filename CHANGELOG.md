# Changelog

All notable changes to STKCraft are recorded here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Versions in the `0.0.x` series are early alpha. Each one states what it can and cannot do,
so a reader is never left guessing whether a missing capability is a bug or simply not built
yet.

## [0.1.6] — Every planet, checked against NASA's own numbers

The reader places Mars, Jupiter and Saturn as bodies, not only as barycentres, and the regression
table now holds 24 CSPICE reference states covering the Sun, Mercury, Venus, the Earth, the Moon
and those three giant planets at three epochs each. All 24 agree to better than a metre in position
and a nanometre per second in velocity.

### Fixed

- **Summary records are a linked list, not a range.** Saturn's kernel puts its summary records at
  either end of a 660 MB file with the data between them, so iterating `FWARD..=BWARD` read data
  records as summaries. Following each record's `NEXT` pointer is what the format asks for; the old
  code at least failed loudly on a nonsense count rather than inventing segments.
- **A target can appear more than once.** A kernel that covers a long span writes one segment per
  time range, so taking the first match for a body answered "outside the coverage" for epochs the
  kernel does cover — Jupiter did exactly that at J2000. Segments are now selected by coverage.
- **Type-3 segments**, which store position *and* velocity coefficients; the satellite-system
  kernels use them, and their velocity comes from the stored coefficients rather than from
  differentiating the position series.

### Added

- **`de440s.bsp`, `mar099.bsp`, `jup365.bsp` and `sat441.bsp` read as one ephemeris**, which is what
  puts Mars, Jupiter and Saturn themselves in reach: `Body::Mars` resolves `499 → 4 → SSB` across
  two files without the caller knowing which file holds which step.

## [0.1.5] — Every version is tagged, and data is fetched rather than committed

Releases are tags in this repository and in its siblings, and 0.1.2 through 0.1.4 were committed
without them. That is now a command rather than a habit: `cargo xtask release` refuses to tag unless
the changelog names the current version, creates the annotated `vX.Y.Z` tag, pushes it, and prints
the `gh release create` line that publishes it with generated notes. The three missing tags have
been created and released.

### Added

- **`cargo xtask release`** — tag and push the version the changelog names, and print the release
  command. The step exists because it was forgotten three times in a row and everything else about
  a release here is mechanical.
- **`cargo xtask fetch-data`** and `docs/ephemeris-data.md` — the ephemeris kernels are downloaded
  from NAIF with a recorded SHA-256 and verified, into a `.gitignore`d directory; nothing is
  committed and a deployment fetches the same files from the same URLs. A row is added to the table
  only with the checksum of a file that was actually verified, never in advance.

### Changed

- **`.gitignore` excludes ephemeris data**: `*.bsp`, `*.bpc`, `*.tpc`, `*.tf` and `data/`. A kernel
  in the repository would age silently against the file a deployment actually reads.

## [0.1.4] — Several kernels, one ephemeris, and an asset gate that means what it says

A planetary kernel carries the system barycentres and a satellite-system kernel carries a planet
and its moons about one of them, so `Mars → Mars barycentre → SSB` spans two files. `EphemerisSet`
reads any number of kernels as one ephemeris and walks each chain across them, which is what makes
"ask for any planet" a single call rather than a question about which file to open.

### Added

- **`EphemerisSet`** — several SPK kernels behind the one `Ephemeris` trait, with the chain lookup
  crossing files. A body no loaded kernel holds is still refused by NAIF identifier.
- **`cargo xtask assets` now ignores directories git ignores.** `plan/` is the local scratch area
  `AGENTS.md` describes, and a file parked there was failing the gate as though it were shipping. A
  directory named by a plain `.gitignore` entry no longer counts as an asset of this repository.

## [0.1.3] — The ephemeris reads a trajectory

The SPK reader evaluates type-2 segments, and `stkcraft-ephemeris` now answers the question it was
built for: where is a body at a TDB instant, in barycentric ICRF coordinates, out of a JPL kernel.

### Added

- **Type-2 SPK evaluation.** Records first, then a four-number directory at the end of the segment
  carrying `INIT`, `INTLEN`, `RSIZE` and `N`; `RSIZE` is a whole record in array elements, so the
  coefficient count is `(RSIZE − 2) / 3`; and a segment with `N = 0` is degenerate rather than
  empty, which is how DE440 writes a barycentre that coincides with its planet.
- **Verified against CSPICE**, which is the only claim that matters here: fifteen reference states
  — the Sun, Mercury, Venus, the Earth and the Moon at three epochs each, the Moon's being a
  two-step chain through the Earth-Moon barycentre — agree to better than 1e−3 km in position and
  1e−9 km/s in velocity over `de440s.bsp`. A previous attempt, reading the directory from the front,
  did not come within 2 000 km; the format document settled it, and the difference between the two
  layouts is recorded in the module documentation so it is not guessed at again.

### Not built yet

- **The analytic backend** (VSOP87 and ELP2000 truncations) that would place the planets with no
  data file at all.
- **Mars, Jupiter, Saturn, Uranus and Neptune themselves.** `de440s.bsp` carries only their system
  barycentres; the body-to-barycentre offset needs a satellite-system kernel, and the reader says
  so by name instead of returning the barycentre as if it were the planet.

## [0.1.2] — The ephemeris crate, and the layout it will not guess

`stkcraft-ephemeris` exists: the trait every backend will fill, a Chebyshev evaluator, and a reader
for the NAIF container the JPL ephemerides are published in. What it does *not* do is read a
trajectory out of a segment, because that layout did not check out, and a wrong ephemeris produces
a wrong mission timeline quietly.

### Added

- **`stkcraft-ephemeris` (L3), with an `Ephemeris` trait** — barycentric ICRF position and velocity
  for a `Body` at a TDB instant, and nothing else. Keeping the contract that narrow is what makes a
  second backend a drop-in later. A TDB epoch is required rather than converted silently, because
  the kernel's argument is TDB and the conversion belongs to `stkcraft-time`.
- **A DAF/SPK structure reader** implemented from NAIF's public format documents: the file record,
  the summary records (whose control words are doubles so they read the same on either endianness),
  and every segment's interval, target, centre, frame, type and word addresses. Checked against
  `de440s.bsp`: fourteen segments, the ten barycentres plus the Earth, Moon, Mercury and Venus
  chains, exactly as the file advertises. Little-endian IEEE only — a big-endian file is refused
  rather than byte-swapped into a plausible wrong trajectory.
- **A Chebyshev evaluator** using Clenshaw for the value and a derivative series for the rate,
  tested against the closed forms, the `Tₙ(±1)` identities and a finite difference.

### Not built yet

- **Reading a type-2 trajectory.** The documented layout — initial state, interval length,
  coefficient count, record count, records — does not reproduce CSPICE on `de440s.bsp`: the word a
  summary points at is the first record's midpoint, and no coefficient count, header length or axis
  ordering tried brought the evaluation within 2 000 km of `spkezr`. The evaluator therefore returns
  `Error::Unsupported`, states why, and the plan document records what was tried. The next step is
  to read the SPK Required Reading against a known-good implementation instead of fitting numbers.
- **The analytic backend** (VSOP87 and ELP2000 truncations) that would place the planets with no
  data file at all.

## [0.1.1] — The kernel learns more than the Earth

The window draws through the same `wgpu` pipelines as the offscreen verification path, time
reaches TDB, and a new crate carries the solar-system bodies with the IAU's own rotation models.

### Fixed

- **The window draws the Earth.** It draws the globe, the ground tracks, the graticule, the
  markers and the star field through the same `wgpu` pipelines the offscreen verification path
  uses, instead of projecting vertices on the CPU and handing `egui`'s painter line segments.
  Two consequences a user sees on the first frame: the Earth is a lit sphere rather than a
  starfield with an outline on it, and the stars are drawn at all — the painter path never
  added them.
- **A ground track on the far side is hidden by the Earth.** The painter had no depth buffer,
  so the half of every track behind the globe was drawn over the disc. The window's pass now
  carries a depth attachment (`NativeOptions::depth_buffer`), the globe writes depth and the
  lines test against it. This is checked numerically rather than by eye: both verification
  paths sample every track vertex, skip the ones within 2 % of the limb, and require that no
  vertex geometrically behind the globe shows its own colour on screen — with the vertices in
  front required to be drawn, so a check that saw nothing could not pass. With the depth
  compare on the line pipeline changed to `Always`, that check reports 100 of 100 far-side
  samples drawn over the globe, which is the defect it exists to catch.

### Changed

- **The pipelines, the shader and the buffers moved into `stkcraft-viewport`.** `SceneRenderer`
  owns the two pipelines, the uniform and one growable buffer per drawable, so the window and
  the screenshot path share them by construction rather than by both happening to create the
  same descriptors. Buffer writes go through `queue.write_buffer` into a buffer that is
  reallocated only when it has to be, instead of a fresh allocation per drawable per frame.
- **`SceneCallback` is the window's drawing path.** It is an `egui-wgpu` paint callback — the
  way to draw into the pass `egui` already records — so the scene lands in the pass that has
  the depth attachment and nothing has to be blitted in from a second swapchain. The surface
  format is read at callback time, and a surface format that changes (dragging the window to a
  monitor driven by another GPU) rebuilds the pipelines instead of drawing with the wrong ones.
- **`STKCRAFT_SCREENSHOT_EGUI` renders the window's own path offscreen**, and with the central
  panel unframed its image is byte-identical to `STKCRAFT_SCREENSHOT`'s. `AGENTS.md` documents
  both.
- **The default branch is `main`**, matching the sibling Craft apps. The two workflows that
  filtered on `master` moved with it in the same commit, because a rename without them would
  have left every push to the default branch unchecked.

### Added

- **`TDB` is a real time scale.** `stkcraft-time` used to treat it as TT; it now carries the
  seven largest terms of the Fairhead & Bretagnon (1990) series in the truncated form of Kaplan
  (2005), USNO Circular 179. Tests compare it with ERFA's full series at eight epochs from 1985
  to 2025: worst disagreement 5.2 µs. `TDB → TT` is a fixed-point inversion, because the series
  argument is a TT quantity that is not known until the correction is.
- **`stkcraft-body`, a new crate beside `stkcraft-frames`,** answers three questions about a
  solar-system body and no others: how massive and how large it is (DE440 masses, IAU 2015
  radii), how it is oriented at a TDB instant (IAU 2015 pole and prime-meridian models, with the
  periodic terms the kernel carries), and where a point on its surface is (planetocentric and
  planetodetic latitudes against that body's own ellipsoid).
  - Ten bodies: the Sun, Mercury, Venus, the Earth, the Moon, Mars, Jupiter, Saturn, Uranus and
    Neptune, keyed by NAIF identifier as well as name.
  - The rotation matrices agree with CSPICE's `pxform` over the same kernel to 1e−8 or better for
    nine bodies at three epochs. That comparison is what established the convention — the pole's
    right ascension uses a sine where the declination uses a cosine — and a structural test
    would have accepted three of the wrong answers.
  - **Mars is incomplete and says so.** Its coefficient arrays in `pck00011.tpc` do not map
    positionally onto its angle list, and no offset reproduces CSPICE, so the periodic terms are
    not guessed at: the body ships with its polynomial terms, and a test records the difference
    and what to do when it is resolved.

### Changed

- **Version control rule, now enforced by `cargo xtask ci`.** Every commit bumps the smallest
  version component and writes the `CHANGELOG.md` section that names it; `cargo xtask version
  bump` does the first half, and the gate fails if the newest section and the workspace version
  disagree, or if an `[Unreleased]` heading is left behind.

### Not built yet

- **Lines are one pixel wide.** `wgpu` has no portable line width, so a thick line needs
  screen-space quad expansion — a geometry change, not a parameter.
- **No anti-aliasing on the geometry.** The window runs one sample; `eframe` supports MSAA, but
  enabling it means building the pipelines for the sample count, so it is a decision rather
  than a flag.
- **The ephemeris, the engine's central body and the moons** are the next three steps of the
  solar-system arc; `docs/plans/2026-11-05-solar-system-kernel.md` records their acceptance
  criteria.

## [0.1.0] — The 3D viewport: the analysis, seen

The first release whose minor version changes, because the product now has a user interface.
`cargo run --release -p stkcraft` opens a window showing the Earth, the satellites in a
scenario and their ground tracks, with a clock, a camera and the access table in a side panel.
The command line is unchanged; a subcommand still goes to the CLI, and an agent still gets no
window.

**The rule from `AGENTS.md` is what shaped this release.** The viewport draws; it never
computes. Everything it shows arrives in a `Frame` the app has already built from propagated
positions, so replacing the viewport cannot change an answer and a number on screen is always
the engine's number.

![The 3D viewport at 0.1.0](docs/images/viewport-0.1.0.png)

### Added

- **`stkcraft-viewport`, a new crate at the UI layer** (`APP_LAYER - 1`), depending only on
  `stkcraft-core`, `wgpu`, `egui` and `bytemuck`:
  - `camera` — a turntable camera and the four matrices the viewport needs, written out rather
    than pulled from a maths crate. `Mat4` is column-major, which is what WGSL expects.
  - `frame` — `Frame`, the whole interface between the engine and the viewport: markers, ground
    tracks, colours and the Earth radius. A plain data structure with no reference to the
    scenario, which is what makes the viewport swappable.
  - `geometry` — the sphere, the graticule, the star field, polylines and markers, all drawn in
    code. Generated geometry carries no licence and needs no `ATTRIBUTION.md` row.
  - `renderer` — the `Globals` uniform, the vertex layout and the WGSL shader, as a string in
    the source so it cannot go missing at runtime.
  - `scene` — the pure `(Frame, Camera) -> SceneData` assembly, which is where the drawing
    decisions live and which is testable without a GPU.
- **The desktop application**, in `apps/stkcraft`:
  - `app` — the `eframe` shell: a window, the camera input (drag to orbit, wheel to zoom at a
    constant ratio per notch), a simulated clock with a rate multiplier, pause/reset/step, and
    a side panel listing the scenario, the access table and any problems. The access panel runs
    the engine's `access.compute` command rather than computing anything itself.
  - `frame_builder` — `Scenario` plus an instant into a `Frame`: positions through
    `Propagator::itrf`, ground tracks sampled over one orbit period, every failure reported
    rather than swallowed.
  - `screenshot` — an offscreen `wgpu` render of the same scene to a PNG, switched on by
    `STKCRAFT_SCREENSHOT`. This is how the viewport is verified on a machine with no window
    system, and it is what produced the image above.
- **`docs/images/viewport-0.1.0.png`**, generated by this repository and attributed.
- **`docs/plans/2026-11-05-viewport.md`** records the arc.

### Fixed

Five defects, every one found by rendering and *looking* rather than by a test passing. They
are listed because each cost real time and each has a lesson:

- **The star field was invisible.** The stars were placed at 60 camera distances while the far
  plane was 100 camera distances — inside the frustum — and they still did not draw. The cause
  was the *ratio* between the near and far planes: a 300 km near plane against a 3 000 000 km
  far plane leaves 24-bit depth unusable at the star radius, so the depth test discarded them.
  The camera now carries explicit `near_ratio` and `far_ratio`, the planets are `0.05` and `4.0`
  of the camera distance, and the star sphere is at `0.9` of the far plane *by construction*.
  A test now sweeps the zoom range and asserts the stars stay inside the far plane.
- **The graticule and the ground track were invisible.** Both sat at `r × 1.001` above the
  surface, which is a quarter of a pixel at any useful zoom, and a coplanar line loses the
  depth test to the surface it hugs about half the time. The obvious fix — a `DepthBiasState`
  on the line pipeline — is rejected by `wgpu` for a line list ("depth bias is not compatible
  with non-triangle topology"). The graticule is therefore lifted by `0.4 %` (25 km, about one
  line width) and ground tracks are lifted to at least the same radius.
- **The ground track was hidden behind the globe.** With the camera looking at the geocentre,
  the satellite passes between the camera and the Earth, so a polar-orbit track was only
  visible where it crossed the limb. The camera now has a `globe_lift`, and the default view
  is lifted a third of an Earth radius so the track shows on the near face. The framing is
  arithmetic, not taste: `asin(6378 / 23000) = 0.28 rad`, about 70 % of the 0.8 rad field of
  view.
- **The ground-track sampler used the wrong unit.** `Elements::mean_motion` is radians per
  minute, not revolutions per day; the sampler treated it as the latter and produced a track
  spanning a day, whose nearest point was 10 800 km from the satellite. The test that caught
  it now asserts the marker lies within one sample of its own track.
- **The app's colour conversion turned infinity into black.** Rust's `clamp` propagates NaN, so
  the `is_finite` guard was mapping `+inf` to zero; NaN and infinity are now handled separately.

### Changed

- **`eframe` 0.37 changed the application trait.** `App::update(ctx, frame)` is now
  `App::ui(ui, frame)` and panels are shown inside a `&mut Ui`; `SidePanel` is `Panel::left`,
  `default_width` is `default_size`, and `raw_scroll_delta` is `smooth_scroll_delta`. Recorded
  here because a future upgrade will move again.
- **The `wgpu` in use is 30.0, via `eframe`,** so the two cannot drift: `wgpu` 30 rejects a
  depth bias on a line list, requires `Option<bool>` for `depth_write_enabled`, takes
  `&[Option<&BindGroupLayout>]` for a pipeline layout, and spells `multiview` as
  `multiview_mask`.
- **`xtask/src/layers.rs`** registers the viewport and adds `wgpu` to `GUI_CRATES`, which is
  what the layering rule is about even though no analysis crate may name it.

### Not built yet

- **The window draws through egui's painter, not through the `wgpu` pipelines.** The offscreen
  path in `screenshot` uses the real pipelines, and the two share `SceneData`, so the geometry
  is identical; the window pass is the next arc. Dragging, zooming and the clock all work.
- **Lines are one pixel wide.** `wgpu` has no portable line width, so a thick line needs
  screen-space quad expansion — a geometry change, not a parameter.
- **Lines ignore the depth of the globe** in the painter path, so the far side of a ground
  track is visible through the Earth. The `wgpu` path does not have this problem.
- **No coastlines, no night-side lighting, no satellite models.** All three want a data file or
  an asset with a source and a checksum, which is a separate decision.
- **The Earth orientation parameters are still `negligible()`**, so ground positions carry up to
  about a kilometre of polar-motion and UT1 error. Unchanged from 0.0.7 and still stated.

## [0.0.7] — A command line that does something

### Added

- **A command line that does something.** `stkcraft passes --tle <L1> <L2> --lat --lon --mask
  --hours --step --name` prints a pass table with absolute UTC times, durations and the total
  as a percentage of the window. Every subcommand dispatches through the engine's command
  registry by id, so the CLI is a transport rather than a second implementation.

### Fixed

- **Times were displayed 37 seconds early.** `Epoch::from_utc` stores the instant on the TAI
  scale, and the display path asked for a UTC conversion that `to_scale` deliberately refuses
  (a TAI Julian Date does not carry the leap offset). The refusal was swallowed, so the
  unconverted value was printed as UTC — exactly the TAI−UTC offset. The new `Epoch::to_utc()`
  reads the leap table instead of guessing.

## [0.0.6] — The engine: from a TLE to an access report

The four numerical layers are now joined into something that runs. A new crate,
`stkcraft-engine`, holds a scenario document, turns each satellite's two-line element set into
an Earth-fixed position through the frame chain, runs access over the timeline, and exposes
every user-visible action as a command.

**The product can now answer its central question end to end.** Give it a TLE and a station;
it reports when the satellite is visible and for how long.

### Added

- **`stkcraft-engine` at L4**, above `stkcraft-access`:
  - `scenario` — the document: satellites with their element sets, stations with their sites
    and sensors, a start epoch and a duration. Data, not behaviour, so a saved scenario never
    changes meaning when the physics improves.
  - `propagate` — the join that had no home: TLE → SGP4 → TEME → ITRF/GCRF. It belongs here
    because `stkcraft-sgp4` and `stkcraft-frames` are both L2 and cannot depend on each other.
  - `analysis` — access over a scenario, one satellite-to-station pair at a time or all pairs.
  - `commands` — the command table: seven commands, each with an id, label, menu path,
    documented parameters, an `enabled` predicate and a `run` function.
- **`Epoch::now()` and `Epoch::j2000()`** in `stkcraft-time`.
- **`docs/plans/2026-11-05-engine.md`** records the arc.

### Fixed

- **A satellite above a mid- or high-latitude station was never visible.** The occultation
test asked whether the line of sight's closest approach to the geocentre was inside a sphere of
volumetric radius, `6371.0 km`. That is right for an equatorial station, whose own radius is
`6378.1 km`. But WGS-84 puts a station at 45° at `6367.5 km`, at 60° at `6362.1` and at the
pole at `6356.8` — every station above about 40° latitude sits *inside* that sphere, so the
closest approach of any line of sight from it was its own radius, every line was reported
occulted, and those stations could see nothing at all.

  Every existing occultation test used an equatorial station, so all of them passed. The bug
  surfaced only when a low-Earth satellite over a 45° station found zero passes in a day. The
  test now covers 45°, 60°, 75° and 89° for both an overhead satellite and the antipode.
- **`Epoch` did not survive JSON.** `serde_json`'s shortest-decimal formatting returned some
  values one unit in the last place away — a 20-nanosecond error that only an equality
  assertion notices. `Epoch` now serialises as `"<scale>:<day>+<fraction>"`, two exact decimal
  numbers, which also makes a scenario file legible to a human.
- **Satellite and station ids could be reused** after removal, because the next id was
  `max(existing) + 1`. Removing the last object freed its id and the next object took it,
  silently rebinding any reference still held. Ids now come from a monotonic counter.

### Correctness

Every claim names the test that holds it.

- **A high-latitude station can see its satellites.** Checked at four latitudes for an
  overhead satellite and its antipode. (`a_high_latitude_station_can_still_see_its_satellites`)
- **The published case 00005 reproduces its TEME state through the engine join,** so the join
  is verified before any frame conversion is involved.
  (`a_near_earth_element_set_reproduces_its_published_teme_state`)
- **A geostationary element set stays within 100 km of one Earth-fixed point over a day,** which
  is the end-to-end check that SGP4, precession, nutation and sidereal time agree.
  (`a_geostationary_satellite_stays_near_one_earth_fixed_point`)
- **A low-Earth satellite gives 3 to 8 passes a day** over a station, each lasting 1 to 30
  minutes. Bounded rather than pinned, so the test means something without failing on an
  element-set edit. (`a_low_earth_satellite_has_several_passes_a_day`)
- **Raising the horizon mask strictly reduces total access.**
  (`raising_the_horizon_mask_reduces_the_total_access`)
- **Commands dispatch by id,** and a missing or wrongly typed argument is refused by name.
  (`commands_dispatch_by_id`, `a_missing_required_argument_is_named`,
  `an_argument_of_the_wrong_type_is_refused`)
- **A broken two-line element set never reaches the scenario.** (`an_invalid_element_set_is_refused_by_the_command`)

### Verification

- `cargo xtask ci --offline` passes all six gates: fmt, clippy `-D warnings`, **208 tests**
  workspace-wide, assets, layers, and the wasm build of all six portable crates.

### Known limitations

- **No transports.** Nothing yet calls the command registry from a menu, a shortcut, a CLI or
  a socket. The table is built and tested through the same dispatch path they will use.
- **No persistence beyond serde.** There is no project file format, no undo and no recent-file
  list.
- **Earth orientation parameters still default to negligible.** They are validated and can be
  supplied, but nothing loads them from an IERS file.
- **No inter-satellite access and no coverage grid** — only satellite-to-ground.
- **Still no viewport and no UI.** The `stkcraft` binary prints its version and exits.

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

[Unreleased]: https://github.com/spacelibres/STKCraft/compare/v0.0.6...HEAD
[0.0.6]: https://github.com/spacelibres/STKCraft/compare/v0.0.5...v0.0.6
[0.0.5]: https://github.com/spacelibres/STKCraft/compare/v0.0.4...v0.0.5
[0.0.4]: https://github.com/spacelibres/STKCraft/compare/v0.0.3...v0.0.4
[0.0.3]: https://github.com/spacelibres/STKCraft/compare/v0.0.2...v0.0.3
[0.0.2]: https://github.com/spacelibres/STKCraft/compare/v0.0.1...v0.0.2
[0.0.1]: https://github.com/spacelibres/STKCraft/releases/tag/v0.0.1
