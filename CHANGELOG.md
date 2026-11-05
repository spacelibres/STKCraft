# Changelog

All notable changes to STKCraft are recorded here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Versions in the `0.0.x` series are early alpha. Each one states what it can and cannot do,
so a reader is never left guessing whether a missing capability is a bug or simply not built
yet.

## [Unreleased]

Nothing yet.

## [0.0.2] — SGP4 propagation

_See the entries under this heading once the work lands._

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

[Unreleased]: https://github.com/spacelibres/STKCraft/compare/v0.0.1...HEAD
[0.0.2]: https://github.com/spacelibres/STKCraft/compare/v0.0.1...v0.0.2
[0.0.1]: https://github.com/spacelibres/STKCraft/releases/tag/v0.0.1
