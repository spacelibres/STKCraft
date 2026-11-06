<h1 align="center">STKCraft</h1>

<!-- The app icon is drawn in code and committed under assets/app-icon/ with a row in
     ATTRIBUTION.md before the first release; it is not referenced here until it exists. -->

<h1 align="center">STKCraft</h1>

<p align="center">
  <b>Space mission analysis and visualisation; an open-source, pure-Rust platform.</b>
</p>

<p align="center">
  Build a scenario from satellites, ground stations, sensors and targets; propagate them over a
  timeline; and answer when something can see something else, and for how long. It runs
  natively on macOS, Windows, Linux and FreeBSD, and in the browser via WebAssembly.<br>
  <i>By the ArtCraft team.</i>
</p>

<p align="center">
  <img alt="Written in Rust" src="https://img.shields.io/badge/written%20in-Rust-4a5568?style=flat-square&logo=rust&logoColor=white">
  <img alt="Runs on macOS, Windows, Linux, FreeBSD and the web" src="https://img.shields.io/badge/runs%20on-macOS%20%C2%B7%20Windows%20%C2%B7%20Linux%20%C2%B7%20FreeBSD%20%C2%B7%20Web-2b6cb0?style=flat-square">
  <img alt="License: MIT OR Apache-2.0" src="https://img.shields.io/badge/license-MIT%20%2F%20Apache--2.0-2b6cb0?style=flat-square">
  <img alt="Status: early alpha" src="https://img.shields.io/badge/status-early%20alpha-c05621?style=flat-square">
</p>

<br>

> [!NOTE]
> **This is an early skeleton.** The workspace, the quality gate, the layering rule and the
> first three numerical crates are in place. Orbit propagation, access and coverage — the
> parts that make it useful — are being built next. See [`docs/design/architecture.md`](docs/design/architecture.md)
> for the crate map and what lands in which order.

## Why STKCraft

- **Correct, not just plausible.** Mission analysis that is subtly wrong is worse than
  analysis that crashes, because nobody notices. Every numerical layer is verified against a
  published reference — the SGP4 verification vectors, the IAU SOFA test values, the IERS
  leap-second table — and every constant carries its source in a comment.
- **Types instead of discipline.** An angle is an `Angle`, not an `f64` that might be degrees.
  An instant carries the time scale it is in. A unit mix-up and a time-scale mix-up are both
  compile errors or explicit, named conversions.
- **Agent-native.** Every user-visible action is a command with an id and a documented
  parameter list. The menus, the keyboard, the CLI, the JSON control channel and the MCP
  server all dispatch the same commands, so an agent can do anything a user can.
- **Everywhere.** One Rust codebase for desktop and the web. No subscription, no licence
  server, no telemetry.
- **Never crashes.** No panics in production code, no `unsafe`, and no NaN where an error
  belongs. An unfinished feature says so instead of returning a plausible number.

## Quick start

```sh
cargo run --release -p stkcraft     # the desktop app
cargo xtask ci                      # fmt, clippy, tests, assets, layers, wasm
```

`cargo xtask` tasks:

| Task | What it does |
|---|---|
| `ci` | The full gate: `fmt --check`, clippy with `-D warnings`, tests, assets, layers, wasm |
| `fmt` | `cargo fmt --check` over the workspace |
| `layers` | Enforces that a crate depends only on lower layers, and that analysis crates stay free of every GUI crate |
| `assets` | Every asset has a row in `ATTRIBUTION.md`, and every row names a file that exists |
| `wasm` | `cargo check --target wasm32-unknown-unknown` for the portable crates |
| `version` | Prints, or with `version set 0.2.0` updates, the single workspace version |

## Architecture

STKCraft is an engine-first Cargo workspace with enforced layering. The analysis crates never
see a window, an input event or a GPU handle, so the same code runs in the desktop app, the
CLI, the MCP server and the browser.

| Layer | Crates |
|---|---|
| L0 | `core` (units, errors, vectors) |
| L1 | `time` (time scales, leap seconds, epochs) |
| L2 | `frames` (reference frames and rotations) |
| next | `sgp4`, `ephemeris`, `access`, `coverage`, `scenario`, `engine` |
| UI | `render` (wgpu viewports), `ui-egui` (the shell), `mcp` |
| apps | `apps/stkcraft`, `stkcraft-cli`, `stkcraft-web` |

- **Contributor and agent rules** (clean-room, asset policy, never-crash, the verification
  standard): [`AGENTS.md`](AGENTS.md)
- **Crate map and the reasoning behind it**: [`docs/design/architecture.md`](docs/design/architecture.md)
- **Bundled assets**: every one is listed with its licence in [`ATTRIBUTION.md`](ATTRIBUTION.md)

## Verifying the numbers

These are the references the numerical layers are held to, in the order they land:

| Layer | Reference |
|---|---|
| Time scales | IAU SOFA / ERFA test values; the IERS `tai-utc.dat` leap-second table |
| Frame rotations | IAU SOFA transformation checks |
| Orbit propagation | Official SGP4 verification vectors; independently published ephemerides |
| Access and coverage | Hand-computable geometry: a satellite directly overhead is visible, one below the horizon is not |

A layer that cannot meet its reference returns an error rather than an approximation it
cannot justify.

## Status

STKCraft is an early skeleton. What exists today:

- The workspace, the `cargo xtask` gate, the enforced layering and the shared CI wiring.
- `stkcraft-framework-core`: typed units, the error type, and a 3D vector that returns `Option` rather
  than NaN when a direction is undefined.
- `stkcraft-framework-time`: the IAU time scales, the published leap-second table with a confidence
  report, and two-part Julian dates precise to well under a microsecond.
- `stkcraft-astro-frames`: rotations that refuse to exist unless they are proper rotations.

What comes next, in order: SGP4 and its verification vectors; the scenario document; the 3D
viewport; then access and coverage, which is the reason the product exists.

## License and credits

STKCraft is dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your
option. Copyright (c) 2026 ArtCraft Team and the STKCraft contributors. Required notices are
in [NOTICE](NOTICE).

Bundled assets, if any, keep their own open licenses; each one is listed with its author,
source and license in [ATTRIBUTION.md](ATTRIBUTION.md).

The ArtCraft name, wordmark and logos are trademarks of the ArtCraft Team and are not covered
by this license. Forks and modified versions must remove them.

STKCraft is a clean-room implementation. It is built from published standards and public
papers; no commercial mission-analysis product's code, data files or artwork is used.

## Repository layout

```
crates/framework/   the platform — units, time, translations, extensions (nothing here knows what an orbit is)
crates/astro/       the orbital-analysis domain — bodies, frames, ephemerides, propagation, access, engine
crates/agent/       the model-facing surface — the knowledge base, the MCP server, the agent runtime
crates/ui/          presentation — the 3D viewport
apps/stkcraft/      the window, the CLI, the MCP server and the assistant
share/              content the app loads — interface catalogs and skill documents
docs/               the engineering record, indexed by docs/README.md
packaging/          installers;  contributors/  credits;  xtask/  the build tool
data/  plan/        fetched bulk data and local notes, both gitignored
```

The map, and the rules that keep it honest, are in [docs/standards.md](docs/standards.md) §9.
