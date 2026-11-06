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
2. `docs/standards.md` — **the rule book**: i18n, agent/skills/MCP, extensions, API docs, crate
   taxonomy, storage.
3. `docs/design/architecture.md` — crate map, layers, and how a scenario flows through them.
4. `AGENTS.md` (this file) — the rules below.

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
  commit a large binary fixture. `cargo xtask fetch-data` downloads all of it — ephemeris kernels,
  IERS Earth orientation, the leap-second kernel, planetary constants and the star catalogue — into
  `data/`, which is `.gitignore`d, as parallel ranged downloads (eight at a time, `--jobs` to
  change it), and verifies each against the SHA-256 recorded in `docs/guides/ephemeris-data.md`; a
  deployment fetches the same files from the same URLs. Nothing is committed, and a row is added to
  that table only with the checksum of a file that was actually verified.
- **Everything is a command.** User-visible behaviour = a command in the engine
  (`id`, label, menu path, shortcut, parameter documentation, `enabled`, `run`) plus tests.
  The menus, shortcuts, CLI, control channel and MCP server all dispatch commands by id, so
  an agent can do anything a user can.
- **A parameter declares its kind** (`Number`, `Text`, `Degrees`), not just its name and prose.
  Every transport converts a caller's value through that declaration, so the schema a model
  reads and the typed value the command receives cannot disagree — the alternative, inferring
  the type from the parameter's name, sent a latitude to a command as a plain number and the
  command answered "expected an angle" to a caller who had sent exactly the right value.
- **A tool returns the answer, not a count.** A command that computes times returns them: the
  text a model reads lists the pass instants, and the same list is in `structuredContent` for a
  program. "4 passes" is a summary, not an analysis, and anything the caller has to recompute is
  work the tool was supposed to have done.
- **Programmatic calls never open dialogs.** A command run through the control channel or
  MCP uses defaults; only a menu click starts an interactive sequence.
- **The model stays outside the kernel.** `stkcraft-astro-engine` makes no network call and links no
  HTTP client; it exposes commands as MCP tools, its state as resources, and procedures as
  skills. `stkcraft-agent-mcp` (a transport, L5) and `stkcraft-agent-runtime` (a client of that protocol, L6)
  sit above it. The agent reaches nothing the protocol does not expose, so a capability a user
  has is a capability an agent has, and adding one means adding a command — never teaching the
  model a number. Do not put a prompt, a model name or a network call below the app layer.
- **Every user-visible string is translatable, and English is the source.** Write interface text in
  English and pass it through `tr!`, `trc!` or `trn!` (`stkcraft_i18n`): `cargo xtask i18n` extracts
  those literals, `cargo xtask ci` checks the catalogs, and `cargo xtask i18n translate --language
  <code>` fills a language from the user's own DeepSeek key. A string built at run time cannot be
  extracted, so the macros take literals. Rules and format: `docs/standards.md` §1,
  `docs/guides/localization.md`.
- **The user's documents come before the model's memory.** Anything factual or numeric starts with
  `knowledge.search` over the person's own library, and the answer cites `path:lines a-b`. A search
  that finds nothing is reported as nothing: the model says the library does not cover the question
  before it offers general knowledge, and never dresses memory up as a document. Rules:
  `docs/standards.md` §2a, `docs/design/knowledge.md`.
- **Skills are documents, not code.** A `share/skills/<name>/SKILL.md` starts with one `#` heading
  whose text is its description, then the procedure: the order of commands, what the answer
  means, and the traps. They are loaded at run time, so a new skill needs no rebuild; they are
  listed by name in the system prompt and read only when a job needs one.
- **A crate's place in the tree says what it is.** `crates/framework/` is the platform (nothing there
  knows what an orbit is), `crates/astro/` is the orbital-analysis domain, `crates/agent/` is the
  model-facing surface, `crates/ui/` is presentation. A new crate goes into the directory that matches
  its subject; the taxonomy is `docs/standards.md` §7 and `cargo xtask layers` is what enforces the
  membership. A `body` or an `access` crate under `framework/` is a bug.
- **State is not scattered.** Every stored thing is relational (the record), key-value (a derived,
  disposable cache) or vector (recall candidates), and one crate owns persistence.
  `docs/design/storage.md` is the decision record; a stray JSON file beside the executable is the failure it
  exists to prevent.
- **Layering** is enforced by `cargo xtask layers`. A crate may depend only on a strictly
  lower layer, and nothing at or below `LAST_NON_UI_LAYER` may depend on egui, eframe,
  winit or rfd. Register every new crate in `xtask/src/layers.rs`.
- **The UI is thin and swappable.** Panels read engine state and act by running commands.
  The 3D viewport is a separate crate so it can be replaced without touching the engine.
- **The window uses the family's UI stack, not a local one.** `egui`/`eframe`/`winit` come from the
  pinned `storytold/egui` fork in `[patch.crates-io]`, the shared library is `craft-ui` from
  `storytold/craft-libs` at the one revision every `craft-*` dependency uses, and text is drawn by
  `craft_ui::fonts::UiFonts` — which searches the machine's installed fonts, the only reason a CJK
  interface renders at all. Do not add a second font stack, a private theme token set or a widget
  that exists in `craft-ui`. `STKCRAFT_FONTS=off` renders without installed fonts, for a
  machine-independent screenshot. See `docs/standards.md` §5.
- **Time goes through `stkcraft-time`.** Never hand-roll calendar conversion, leap seconds
  or time-scale offsets; never use a bare `f64` to mean "seconds since some epoch".
- **Units are types.** `Angle`, `Distance`, `Duration` and `EpochOffset` exist so a
  degrees/radians mix-up is a compile error. Do not add `From<f64>` conversions.
- **Rust only** (no handwritten JS/TS). **Never break wasm** (`cargo xtask wasm`).
- **Quality gates** before every commit: `cargo xtask ci` (fmt, clippy `-D warnings`, tests, assets,
  i18n, api-docs, layers, wasm, version, and the lockfile check the workflow ends with). It is the
  same gate `.github/workflows/ci.yml` runs through the shared `gate.yml`, so a commit that passes it
  locally is a commit whose CI gate passes on Linux. Two things it cannot check locally and CI can:
  the other matrix legs (macOS, Windows) and the packaging lint. Commit after every arc that builds
  and passes.
- **A device file is never read to the end.** `std::fs::read` on `/dev/urandom` (or any device)
  allocates until the machine dies — it did exactly that on 2026-11-06, taking a machine's memory to
  130 GB and getting the process OOM-killed along with the desktop. Read a stated number of bytes with
  `read_exact`, and bound every read whose length comes from outside the repository.
- **Every commit is a version.** See "Version control and the changelog" below: the smallest
  version component goes up by one and `CHANGELOG.md` gains the section that names it, in the
  same commit. `cargo xtask ci` fails if the two disagree.

## Running and looking at the app

```sh
cargo run --release -p stkcraft                # the desktop app
cargo run --release -p stkcraft -- --help      # the command line
cargo run --release -p stkcraft -- mcp         # serve the engine to a model over stdio
cargo run --release -p stkcraft -- ask "..."   # ask the model, with the engine as its tools
cargo run --release -p stkcraft -- chat        # ask questions in one conversation
cargo xtask ci                                 # the full gate
cargo xtask version                            # print the workspace version
cargo xtask version bump                       # bump it, smallest component
cargo xtask release                            # tag the version and push the tag
cargo xtask fetch-data                         # download the kernels, verified
cargo xtask i18n                               # check the interface catalogs
cargo xtask i18n report                        # what each language still lacks
cargo xtask i18n translate --language de       # fill German from the English source
cargo xtask api-docs                           # regenerate the API reference
cargo run --release -p stkcraft -- extensions  # what extension packages are installed
cargo run --release -p stkcraft -- knowledge   # the documents a model answers from
cargo run --release -p stkcraft -- knowledge "maximum off-nadir angle"
cargo run --release -p stkcraft -- history     # what this installation has computed
cargo run --release -p stkcraft -- chat --sessions          # stored conversations
cargo run --release -p stkcraft -- chat --session <id>      # resume one
cargo run --release -p stkcraft -- control --port 7979      # the launcher-facing socket
cargo xtask lockfile                           # Cargo.lock matches what is committed
```

### Version control and the changelog

Every commit that changes the product is a release-shaped step, and the history says so:

1. **Bump the smallest version component.** `0.1.7` becomes `0.1.8`. `cargo xtask version bump`
   edits `[workspace.package]` in `Cargo.toml` and regenerates `Cargo.lock`; that manifest entry
   is the single source of the number, because every crate inherits it with
   `version.workspace = true`.
2. **Write the changelog section that names it**, in the same commit: `## [0.1.8] — <title>`,
   then `### Added` / `### Changed` / `### Fixed` / `### Not built yet` as they apply. Newest
   section at the top.
3. **Write it for a reader deciding whether to upgrade**, not for a diff. What changed, why it
   mattered, and what it means for them — including what is still missing. A changelog line that
   only repeats the commit subject has said nothing.
4. **No `[Unreleased]` section.** An unreleased section is where entries go to be forgotten, and
   the whole point of this rule is that no commit is a draft. Each commit writes its own version.
5. **Tag the release**, and make it a release rather than only a tag: `cargo xtask release` checks
   that the changelog names the version, creates the annotated `vX.Y.Z` tag, pushes it, and prints
   the one `gh release create` command that publishes it with generated notes. Every version in
   this repository carries a tag — the siblings tag every release, and a version without one is a
   version nobody can check out. `cargo xtask release` exists so that step cannot be forgotten;
   for 0.1.2 through 0.1.4 it was, and the tags had to be added afterwards.

`cargo xtask ci` enforces the mechanical half: it fails when the newest `## [x.y.z]` heading and
the workspace version disagree, and when an `[Unreleased]` heading is present. The judgement half
— whether the entry is worth reading — is not mechanical, and that half is yours.

### Looking at the viewport without a window

A build machine has no window system, so the app can render one frame offscreen and write it as a
PNG. There are three paths, because the window has three layers and any of them can be the broken
one:

```sh
STKCRAFT_SCREENSHOT=/tmp/frame.png cargo run --release -p stkcraft        # the scene renderer
STKCRAFT_SCREENSHOT_EGUI=/tmp/window.png cargo run --release -p stkcraft  # the window's own path
STKCRAFT_SCREENSHOT_PANELS=/tmp/panels.png cargo run --release -p stkcraft # the panels around it
```

The first opens its own `wgpu` pass and asks whether the pipelines and the geometry are right. The
second builds an `egui` frame the way the window does, draws the scene as a paint callback inside
`egui-wgpu`'s pass with the depth attachment the window configures, and asks whether the window's
own path draws that scene — a callback that never runs, a missing depth buffer or a projection
built from the wrong aspect ratio all pass the first path and leave the window blank. With no
margin on the panel the two images are byte-identical, which is the check that says the window and
the verified render agree.

The third path draws the panels and leaves the middle of the image as space, because the scene over
it needs a render pass it does not open. It opens the assistant panel first — the panel whose job is
to run the engine — and then checks the image numerically: each side must have content that is
neither space nor the panel's flat fill, and enough label pixels to prove its text landed, with a
blank image as the control that the check can fail at all. A panel that laid out at zero height or
clipped its labels still exits successfully and still writes a PNG, which is exactly the failure an
unexamined image hides.

All three print the adapter they chose, so a software Vulkan fallback is visible rather than hidden.
The first two check the depth buffer numerically: every ground-track sample that is geometrically
behind the globe must be hidden, with the samples in front as the control that proves the check can
see a line at all. `STKCRAFT_SCREENSHOT` is how the viewport image in `CHANGELOG.md` was produced, and
`STKCRAFT_SCREENSHOT_EGUI` must stay byte-identical to it when no panel is open — that equality is the
check that the window and the verified render agree, and the panel path is deliberately outside it.

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

## Licensing

`MIT OR Apache-2.0`, as the family uses: `LICENSE-MIT` and `LICENSE-APACHE` are both present and
`NOTICE` names them, and `Cargo.toml`'s `license.workspace` says the same. A dependency whose licence
is incompatible with that pair does not enter the build. The ArtCraft name and marks are not covered by
the licence. See `docs/standards.md` §6.

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
