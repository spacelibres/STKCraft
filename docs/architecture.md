# Architecture

STKCraft answers questions about spacecraft and ground stations over time: where is a
satellite at a given instant, what can it see, and for how long. This document explains how
the crates fit together and why they are split the way they are.

## Crates and layers

```text
L0  core      units (Angle, Distance, Duration, EpochOffset), the Error type, Vector3
L1  time      time scales, the IERS leap-second table, Julian dates and epochs
L2  frames    reference frames and rotations
```

Planned, in the order they will land:

```text
L0  geom      conic sections, coordinate transforms on the sphere, intersection routines
L1  astro     low-precision Sun, Moon and planet positions
L2  sgp4      SGP4/SDP4 propagation, verified against the published test vectors
L2  ephemeris SP3 and CCSDS OEM/OPM/OMM readers
L2  atmos     neutral atmosphere density
L3  access    line of sight, elevation masks, eclipse and interference geometry
L3  coverage  grids and grids-of-figures over time
L4  scenario  the document model: scenario, objects, links, time intervals
L5  engine    Session, the command registry, history, caches
L6  render    the wgpu 3D and 2D viewports (no egui)
L6  ui-egui   the egui shell (thin; every action is a command)
L6  mcp       the MCP server
    apps      stkcraft (desktop), stkcraft-cli, stkcraft-web
```

`cargo xtask layers` enforces that a crate depends only on a strictly lower layer, and that
nothing at or below `LAST_NON_UI_LAYER` touches egui, eframe, winit or rfd. Every new crate
must be registered in `xtask/src/layers.rs`; the task fails for a workspace member that is
missing from the table.

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
