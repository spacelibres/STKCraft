# STKCraft — instructions for agents

STKCraft is a clean-room, open-source, Rust-native space mission analysis and visualisation
application. It targets the workflow of a professional orbital-analysis suite — building a
scenario out of satellites, ground stations, sensors and targets, propagating them over a
timeline, and answering "when can this see that, and for how long" — and aims to be faster,
more open and fully agent-drivable. It runs natively on macOS, Windows, Linux and FreeBSD,
and in the browser via WebAssembly.

It is one of the ArtCraft Craft apps and follows the same conventions as its siblings:
`../gridcraft` (spreadsheet), `../cadcraft` (CAD), `../texturecraft` (3D texture painting),
`../designcraft` (page layout), `../photocraft` (image editing).

## Start every session here

1. `ROADMAP.md` (once it exists) — current stage and focus.
2. `docs/architecture.md` — crate map, layers, and how a scenario flows through them.
3. `AGENTS.md` (this file) — the rules below.

`plan/` is gitignored and local only.

## The one rule that differs from the siblings

The other Craft apps recreate what a user can *see*; a screenshot comparison is a valid
acceptance test. STKCraft computes physics, and a plausible-looking wrong answer is worse
than a visible crash because nobody notices it.

**A numerical layer is finished only when it matches a published reference:**

- orbit propagation against the official SGP4 verification vectors, or against an
  independently published ephemeris;
- time scales against the IAU SOFA / ERFA test values and the IERS leap-second table;
- frame rotations against SOFA's transformation checks;
- access and coverage geometry against hand-computable cases (a satellite directly
  overhead of a station is visible; one below the horizon is not).

**Every physical constant and every formula carries its source in a comment** — a standard,
a paper, or a reference implementation with its version. "It looks right" is not evidence.
When a value cannot be verified, return [`Error::Unsupported`] rather than a guess.

## Never crash

People trust STKCraft with mission timelines; a crash loses their work, and a silent NaN
corrupts an analysis without telling anyone. **This outranks feature work.**

- **No panics in non-test code:** no `unwrap()`, `expect()`, `panic!`, `unreachable!`,
  `todo!`, `unimplemented!`; no `unsafe` (`unsafe_code = "forbid"`).
- Every production crate root carries
  `#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]`.
- **Errors are `Result<T, E>`** through `stkcraft_core::Error` and `?`. An unfinished
  feature returns `Error::Unsupported`; it never panics and never returns a placeholder
  number.
- **Input-derived numbers are hostile** (ephemeris files, scenario documents, command and
  MCP parameters): validate ranges before use, use checked or saturating arithmetic, bound
  recursion and cap input-sized allocations.
- **NaN is not an error value.** A geometric function that cannot produce a direction
  returns `Option`, so the caller decides. `Vector3::normalized` is the model: it returns
  `None` for a zero vector rather than a NaN vector.
- **Prove it:** every crash or correctness fix lands with a regression test.

## Non-negotiables

- **Clean-room.** No commercial mission-analysis product is installed here and none of its
  code, data files, help text or artwork may be read or copied. Everything comes from
  published standards (IERS Conventions, IAU resolutions, CCSDS blue books, the SGP4
  specification), public papers and public domain data. Never copy GPL/LGPL/AGPL code.
- **Assets — absolutely essential.** No iconography, artwork, fonts, satellite models or
  star textures from any commercial product, ever. Every asset must be original, public
  domain/CC0, or redistributable open source, and must have a row in `ATTRIBUTION.md`;
  `cargo xtask assets` enforces it. Prefer drawing in code — icons, orbit colours, ground
  tracks and the star field are generated, so they carry no licence.
- **Fonts live in [`storytold/craft-fonts`](https://github.com/storytold/craft-fonts)**, as
  an optional build input (`CRAFT_FONTS_DIR`). No font file is committed here.
- **Bulk data lives outside this repository.** Star catalogues, Earth orientation
  parameters, gravity models, atmospheric tables and ephemeris files are fetched pinned and
  checksum-verified, or bundled as small generated tables with a source comment. Never
  commit a large binary fixture.
- **Everything is a command.** User-visible behaviour = a command in the engine
  (`id`, label, menu path, shortcut, parameter documentation, `enabled`, `run`) plus tests.
  The menus, shortcuts, CLI, control channel and MCP server all dispatch commands by id, so
  an agent can do anything a user can.
- **Programmatic calls never open dialogs.** A command run through the control channel or
  MCP uses defaults; only a menu click starts an interactive sequence.
- **Layering** is enforced by `cargo xtask layers`. A crate may depend only on a strictly
  lower layer, and nothing at or below `LAST_NON_UI_LAYER` may depend on egui, eframe,
  winit or rfd. Register every new crate in `xtask/src/layers.rs`.
- **The UI is thin and swappable.** Panels read engine state and act by running commands.
  The 3D viewport is a separate crate so it can be replaced without touching the engine.
- **Time goes through `stkcraft-time`.** Never hand-roll calendar conversion, leap seconds
  or time-scale offsets; never use a bare `f64` to mean "seconds since some epoch".
- **Units are types.** `Angle`, `Distance`, `Duration` and `EpochOffset` exist so a
  degrees/radians mix-up is a compile error. Do not add `From<f64>` conversions.
- **Rust only** (no handwritten JS/TS). **Never break wasm** (`cargo xtask wasm`).
- **Quality gates** before every commit: `cargo xtask ci` (fmt, clippy `-D warnings`,
  tests, assets, layers, wasm). Commit after every arc that builds and passes.

## Running and looking at the app

```sh
cargo run --release -p stkcraft                # the desktop app
cargo run --release -p stkcraft -- --help      # the command line
cargo xtask ci                                 # the full gate
```

### Looking at the viewport without a window

A build machine has no window system, so the app can render one frame offscreen through the
same `wgpu` pipelines the window will use and write it as a PNG:

```sh
STKCRAFT_SCREENSHOT=/tmp/frame.png cargo run --release -p stkcraft
```

This is a verification path, not a feature. It prints the adapter it chose, so a software
Vulkan fallback is visible rather than hidden. It is how the viewport image in `CHANGELOG.md`
was produced.

**A passing test is not evidence that the drawing is right.** Five defects in the 0.1.0
viewport — an invisible star field, an invisible graticule, a ground track hidden behind the
Earth, a track sampled over the wrong period, and an infinity rendered as black — passed their
unit tests while producing a wrong image. Render, look at the pixels, and only then claim the
viewport works. That is the same standard as the numerical layers, applied to drawing.

### Display environment

On Wayland, `eframe` picks an X11 client through XWayland and there is no portable screenshot
tool: the GNOME screenshot portal prompts for permission and the X11 root window shows nothing.
That is why the offscreen path above exists.

Once the control channel lands, this section gains the `--control <port>` recipe the siblings
use.

## CI workflows

`.github/workflows/` calls the shared workflows in
[`storytold/craft-common-actions`](https://github.com/storytold/craft-common-actions) at
`@main`, with their inputs and secrets. No other jobs. If a shared workflow cannot build or
test a change, link an existing `craft-common-actions` issue or open one; do not add a
local job.

**Known upstream failure.** `Packaging lint` fails at its PowerShell step because that step
runs `Get-ChildItem packaging/windows/*.ps1` without an existence guard and this repository
has no Windows packaging yet. Tracked as
[craft-common-actions#145](https://github.com/storytold/craft-common-actions/issues/145);
the fix belongs there, not here. Until it lands, a red `Packaging lint` on a change that
touches nothing under `packaging/` is that issue and not a regression.

## Contributor credits (About window)

`contributors/contributors.json` is generated and never hand-edited;
`contributors/people.toml` holds names people chose for themselves. **Never add, edit or
guess anyone else's entry.** If your human's GitHub username has no `[people.<username>]`
entry, ask them once whether they want to be credited by more than their username.
