# Architecture

STKCraft answers questions about spacecraft and ground stations over time: where is a
satellite at a given instant, what can it see, and for how long. This document explains how
the crates fit together and why they are split the way they are.

## Crates and layers

```text
L0  core      units (Angle, Distance, Duration, EpochOffset), the Error type, Vector3
L1  time      time scales, the IERS leap-second table, Julian dates and epochs
L2  frames    reference frames and rotations
L2  sgp4      SGP4/SDP4 propagation from a two-line element set
L3  access    line of sight, elevation masks, occultation, access windows
L4  engine    scenario, propagation, analysis, the command registry
L4  viewport  the 3D viewport: GPU rendering and generated geometry
    apps      stkcraft (window and CLI)
```

Planned, in the order they will land:

```text
L0  geom      conic sections, coordinate transforms on the sphere, intersection routines
L1  astro     low-precision Sun, Moon and planet positions
L2  ephemeris SP3 and CCSDS OEM/OPM/OMM readers
L2  atmos     neutral atmosphere density
L3  coverage  grids and grids-of-figures over time
    mcp       the MCP server, dispatching the same command ids
```

`cargo xtask layers` enforces that a crate depends only on a strictly lower layer, and that
nothing at or below `LAST_NON_UI_LAYER` touches egui, eframe, wgpu, winit or rfd. Every new
crate must be registered in `xtask/src/layers.rs`; the task fails for a workspace member that
is missing from the table.

The viewport sits above `LAST_NON_UI_LAYER` because it *is* the user interface's drawing code,
but it depends only on `stkcraft-core`: it takes a `Frame` of already-computed positions and
knows nothing about satellites, elements or access. That is the boundary that makes it
replaceable — see below.

## Why the viewport never computes

The other Craft apps recreate what a user can see, and a screenshot is a valid acceptance test.
STKCraft computes physics, where a plausible-looking wrong answer is worse than a crash because
nobody notices it. If the viewport derived an orbit from the current elements — an osculating
ellipse, for instance — it would disagree with the engine the moment drag or J2 mattered, and
the disagreement would be invisible.

So the rule is structural rather than a convention: the viewport's whole input is a `Frame` of
positions the engine produced, and it has no access to an element set. Replacing it cannot
change an answer.

## Why units and time are separate crates

Two classes of mistake in mission analysis produce answers that look right and are wrong:

1. **A unit mix-up.** Passing degrees where radians are expected is a silent factor of 57.
2. **A time-scale mix-up.** Propagating on a uniform scale but stamping the result with a
   UTC time that contains leap seconds shifts everything by a whole second, and a
   conjunction screen that was meant to show a miss now shows a hit — or the reverse.

Both are prevented by types rather than by discipline. `Angle`, `Distance` and `Duration`
carry their unit, and there is deliberately no `From<f64>` so a bare number cannot slip
into a typed position. `Epoch` carries the scale it is expressed in and refuses to take an
interval between two different scales.

## Why Julian dates are stored in two parts

A Julian Date near the present is about 2.46 × 10⁶. An `f64` has about 16 significant
digits, so a single Julian Date resolves only about 40 microseconds. That is coarser than
the 1-second timestep a typical scenario uses, and far coarser than an ephemeris
interpolation wants.

`JulianDate` therefore keeps a whole-day part and a fractional part, following the IAU SOFA
two-part convention. Every interval and every difference is computed part by part — never
by subtracting two summed Julian Dates, which would throw away exactly the precision the
split exists to preserve. A regression test locks this in: a one-microsecond step must
survive a round trip.

## The leap-second table

UTC is steered by inserting whole seconds, so `TAI - UTC` changes. The table in
`stkcraft-time::leaps` is the one in the IAU SOFA / ERFA reference implementation, which in
turn comes from the IERS `tai-utc.dat` file.

Two consequences follow, and both are visible in the API rather than buried:

- A lookup reports its **confidence**. A date before 1960 or more than five years past the
  table's revision is flagged, so a caller can warn instead of pretending the offset is a
  fact.
- A scenario dated past the revision can supply its own `LeapSecondTable` rather than
  waiting for a release.

A day that ends with an inserted second uses the offset in force *before* that leap, which
is SOFA's convention and the one ephemeris files assume.

## Verification

Every numerical layer is finished only when it matches a published reference. The test
vectors live next to the code that uses them, and the constant or formula they verify is
cited in a comment beside it. See `AGENTS.md` for the standard a layer must meet before it
is called done.

`stkcraft-sgp4` is the first layer to meet that standard, and it is worth looking at as the
pattern:

- The verification data is a transcription of the AIAA 2006-6753 archive — the 33 element
  sets and the reference implementation's own output for them — kept in
  `crates/sgp4/src/tests/vectors/data.rs` with its source named.
- The tests compare against that data in two groups, because the two halves of the theory are
  in different states. The near-Earth half is held to `4e-9 km`, which is the limit of what
  a table printed to eight decimals can express; the deep-space half is held to the accuracy
  it actually has today, and `CHANGELOG.md` says plainly that it is not yet exact.
- The tolerances are set to the measured accuracy, never to the intended accuracy, so a
  regression fails the test instead of being absorbed by a loose bound.

A tolerance that a wrong answer can pass is not a test. That is why the deep-space bound is
documented as a limitation rather than presented as a result.
