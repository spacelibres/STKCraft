<!-- craft-house-rules:start core source=dde09ea31e2e457f5b4653a6e76f7cdf55944bdf sha256=540e6cea75d12d0710c3dca89658328e81e508ecef102446ca5bb95b7c0b92bb -->
## House rules

These apply in every Craft repository. A later rule may be stricter, not weaker. The source is `standards/house-rules-core.md` in `storytold/craft-contrib`. Propose changes there, not in a copy.

1. **A person is accountable.** Whoever runs an agent answers for what it opens and for its review comments.
2. **Check, then claim.** Before writing code, read the issue comments, search pull requests for `#N` and the keywords, and check recent commits on `main`. Comment on the issue first. Release a claim you will not finish.
3. **Verify, don't guess.** Open a definition before using it. Confirm the cause in the code, even when someone names a location.
4. **Keep fixes narrow and tested.** Fix what the issue describes, where the bug is. Add a test that fails without the fix.
5. **Review the diff before opening a pull request.** Identifiers exist as used, the test fails without the fix, nothing unrelated changed, and the description is true.
6. **Look for a better pull request.** If another fixes the same thing, close yours when it is better or equal, and say why.
7. **Report only what ran.** Say what you tested and what you could not run. Without a local build or test, say so and rely on CI before marking ready.
8. **`main` changes through a pull request.** Do not push to it. Branch protection enforces this. There is no exception.
9. **Stop and ask only about** licensing outside the allowlist, publishing, releases, a new remote, secrets, spending money, destructive git, and anything else hard to undo. Otherwise keep working. `plan/` is a gitignored notebook, not rules. Without it, do not ask for it and do not stop.
10. **Keep secrets and personal details out** of the repository, logs and requests.
11. **CI stays shared.** Workflows call `storytold/craft-common-actions`. Do not change CI in a feature or fix pull request. If CI cannot cover a change, open an issue there.
12. **Guards say why.** A check that looks removable says what it prevents. Read that before changing it.
13. **Instructions must be readable by everyone.** No notes about one machine, and no links to files a reader cannot open. `craftrules` is retired. Do not link to it.
14. **Keep pull requests moving.** Concurrent pull requests are fine; unanswered actionable feedback is not. Respond in 7 days, a reminder at 14, closure eligible at 30, from when an actionable request waits on the author. Suggestions and unconfirmed bot findings do not start the clock. A fix, explanation or agreed plan resets it; unrelated activity does not. Waiting on a reviewer or maintainer pauses it and does not count against the author. A maintainer decides closure; it is never automatic. Reuse search results, do not poll, and stop at a rate limit.
15. **No agent attribution.** A pull request does not say an agent wrote it. A repository rule may not require that.
<!-- craft-house-rules:end -->
<!-- craft-house-rules:start rust source=dde09ea31e2e457f5b4653a6e76f7cdf55944bdf sha256=387e0ccb4b49e567f60951d7d6fff7305054c89f43ad812b9993dc7c38528013 -->
## Rust Craft repositories

These apply to native Craft apps and to shared Rust libraries. Select them by repository type. A repository that merely contains some Rust does not take this block. Rules later in a repository's file may be stricter, but not weaker. The source is `standards/house-rules-rust.md` in [storytold/craft-contrib](https://github.com/storytold/craft-contrib).

1. **Never crash.** Non-test code does not panic and has no `unsafe`, except where the repository file names an exception. Return errors, treat input as hostile, and bound recursion and allocations. New production crate roots copy the `deny` attribute from an existing crate: `clippy::unwrap_used`, `clippy::expect_used`, `clippy::panic`, `clippy::unimplemented`, `clippy::todo`, `clippy::unreachable`. Every crash fix comes with a test that panicked before the fix.
2. **Clean room.** Learn from observed behaviour and public specifications. Do not copy another product's code, assets, strings or artwork. Check a dependency's licence before adding it.
3. **Assets and fonts.** Every asset is original or suitably licensed, and is recorded in `ATTRIBUTION.md` in the same change. Font files live in [storytold/craft-fonts](https://github.com/storytold/craft-fonts), not in the app repository.
4. **Rust only for native Craft apps.** No handwritten JavaScript or TypeScript, and no Tauri, Electron or webview shell, in those apps. Shared tooling follows its documented technical scope and maintainer-approved exceptions. Craft PR Review's existing Tauri and TypeScript UI is one such exception. It does not relax this rule for native apps. Preserve any existing web build.
5. **Time goes through `chrono`.** Do not hand-roll calendar conversion. For dates beyond what `chrono` can represent, follow [standards/time.md](https://github.com/storytold/craft-contrib/blob/main/standards/time.md).
6. **Contributor credits.** Add or edit only your own person's entry in `contributors/people.toml`, and only after asking them. Never guess a name or hand-edit `contributors.json`.

## Licence allowlist

Dependencies and assets use SPDX identifiers. An expression is allowed when every identifier is in this list. A repository may name a shorter list. It may not add an identifier here by editing its own copy of these rules.

- `ISC`
- `MIT`
- `BSD-2-Clause`
- `BSD-3-Clause`
- `Apache-2.0`
- `OFL-1.1`
- `Ubuntu-font-1.0`
- `Bitstream-Vera`
- `CC0-1.0`
- `CC-BY-4.0`
- `CC-BY-SA-4.0`
- `LicenseRef-Public-Domain`

`LicenseRef-Storyteller-Trademark` is not in this list. It covers Storyteller brand marks only, where a repository already says so.
<!-- craft-house-rules:end -->

# STKCraft — instructions for agents

STKCraft is a clean-room, open-source, Rust-native space mission analysis and visualisation
application. It targets the workflow of a professional orbital-analysis suite — building a
scenario out of satellites, ground stations, sensors and targets, propagating them over a
timeline, and answering "when can this see that, and for how long" — and aims to be faster,
more open and fully agent-drivable. It runs natively on macOS, Windows, Linux and FreeBSD, and its
analysis core compiles to WebAssembly; a browser shell is not shipped yet.

It is an independent project applying to join the ArtCraft Craft family, and follows the same
conventions as the family's apps: `../gridcraft` (spreadsheet), `../cadcraft` (CAD),
`../texturecraft` (3D texture painting), `../designcraft` (page layout), `../photocraft` (image
editing).

## Start every session here

1. `ROADMAP.md` — what is shipped, what is next, and what "done" means for each step.
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
- **Errors are `Result<T, E>`** through `stkcraft_framework_core::Error` and `?`. An unfinished
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
  commit a large binary fixture. `cargo xtask fetch-data` downloads the ephemeris data — ephemeris
  kernels, IERS Earth orientation, the leap-second kernel, planetary constants and the star
  catalogue — into `data/`, which is `.gitignore`d, one file after another, each as parallel ranged downloads (eight at a time,
  `--jobs` to change it), and verifies each against the SHA-256 recorded in
  `docs/guides/ephemeris-data.md`; a deployment fetches the same files from the same URLs.
- **The documents the knowledge base is measured on are fetched too.** `cargo xtask papers` puts the
  PDF corpus in `data/papers/` the same way, verified against `docs/guides/paper-corpus.md`. Nothing
  is committed, and a row is added to either table only with the checksum of a file that was actually
  downloaded and verified.
- **Everything is a command.** User-visible behaviour = a command in the engine
  (`id`, label, menu path, shortcut, parameter documentation, `enabled`, `run`) plus tests.
  The menus, shortcuts, CLI, control channel and MCP server all dispatch commands by id, so
  an agent can do anything a user can.
- **A parameter declares its kind** (`Number`, `Text`, `Degrees`, `DegreesPair`), not just its name and prose.
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
  English and pass it through `tr!`, `trc!` or `trn!` (`stkcraft_framework_i18n`): `cargo xtask i18n` extracts
  those literals, `cargo xtask ci` checks the catalogs, and `cargo xtask i18n translate --language
  <code>` fills a language from the user's own DeepSeek key. A string built at run time cannot be
  extracted, so the macros take literals. Rules and format: `docs/standards.md` §1,
  `docs/guides/localization.md`.
- **The user's documents come before the model's memory.** Anything factual or numeric starts with
  `knowledge.search` over the person's own library, and the answer cites `path:lines a-b` — or
  `paper.pdf:page 7, lines 3-9`, because a paper's place is a page and a file's is a line. A search
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
  winit, rfd or anything else listed in `GUI_CRATES` (`xtask/src/layers.rs`). Register every new crate in `xtask/src/layers.rs`.
- **The UI is thin and swappable.** Panels read engine state and act by running commands.
  The 3D viewport is a separate crate so it can be replaced without touching the engine.
- **The window uses the family's UI stack, not a local one.** `egui`/`eframe`/`winit` come from the
  pinned `storytold/egui` fork in `[patch.crates-io]`, the shared library is `craft-ui` from
  `storytold/craft-libs` at the one revision every `craft-*` dependency uses, and text is drawn by
  `craft_ui::fonts::UiFonts` — which searches the machine's installed fonts, the only reason a CJK
  interface renders at all. Do not add a second font stack, a private theme token set or a widget
  that exists in `craft-ui`. `STKCRAFT_FONTS=off` renders without installed fonts, for a
  machine-independent screenshot. See `docs/standards.md` §5.
- **Time goes through `stkcraft-framework-time`.** Never hand-roll calendar conversion, leap seconds
  or time-scale offsets; never use a bare `f64` to mean "seconds since some epoch".
- **Units are types.** `Angle`, `Distance`, `Duration` and `EpochOffset` exist so a
  degrees/radians mix-up is a compile error. Do not add `From<f64>` conversions.
- **Rust only** (no handwritten JS/TS). **Never break wasm** (`cargo xtask wasm`).
- **Quality gates** before every commit: `cargo xtask ci` (fmt, clippy `-D warnings`, tests, assets,
  experts, experts-eval, i18n, api-docs, layers, wasm, version, and the lockfile check the workflow ends with). It is the
  same gate `.github/workflows/ci.yml` runs through the shared `gate.yml` on Ubuntu and Windows, so a
  commit that passes it locally is a commit whose CI gate passes on both legs. Two things it cannot
  check locally and CI can: the Windows leg's own environment and the packaging lint. macOS is not
  in the CI matrix yet, so a macOS-only regression is found by hand or in a release run. Commit after
  every arc that builds and passes.
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
cargo run --release -p stkcraft -- passes      # satellite passes over a ground station
cargo run --release -p stkcraft -- planet      # passes over a station on another body
cargo run --release -p stkcraft -- commands    # every command id, parameter and menu
cargo run --release -p stkcraft -- run access.compute --scenario mission.json --json
                                               # any registry command, against a scenario document
cargo run --release -p stkcraft -- model       # show or change the model in use
cargo run --release -p stkcraft -- version     # the version of this build
cargo run --release -p stkcraft -- chat        # ask questions in one conversation
cargo xtask ci                                 # the full gate
cargo xtask version                            # print the workspace version
cargo xtask version bump                       # bump it, smallest component
cargo xtask release                            # tag and push; a GitHub Release only if stable
cargo xtask fetch-data                         # download the kernels, verified
cargo xtask papers                             # download the paper corpus, verified
cargo xtask i18n                               # check the interface catalogs
cargo xtask i18n report                        # what each language still lacks
cargo xtask i18n translate --language de       # fill German from the English source
cargo xtask api-docs                           # regenerate the API reference
cargo xtask version set X.Y.Z                  # set the version to a given number
cargo xtask standards                          # download the CCSDS corpus, verified
cargo xtask naif                               # download the NAIF corpus, verified
cargo xtask mathlib                            # index a Mathlib checkout (MATHLIB_DIR=<checkout>)
cargo xtask verify-formulas                    # re-extract and re-classify formulas; fail on drift
cargo xtask layers                             # check crate layering
cargo xtask wasm                               # check the WebAssembly build
cargo xtask assets                             # check asset attribution
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
5. **Tag every version; publish a Release only for a stable one.** `cargo xtask release` checks that
   the changelog names the version, creates the annotated `vX.Y.Z` tag, pushes it, and — only when
   the version is stable (1.0.0 or later, no `-rc`/`-alpha` suffix) — prints the
   `gh release create` command. The 0.x development records are **tags and nothing else**: a GitHub
   Release says "this is the version to use", which a 0.x version does not promise, and a dev
   version with both a tag and a Release is the duplication this rule removes. Every version in this
   repository carries a tag — the siblings tag every release, and a version without one is a version
   nobody can check out. `cargo xtask release` exists so that step cannot be forgotten; for 0.1.2
   through 0.1.4 it was, and the tags had to be added afterwards.

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

### Driving the window

The window serves the family's control channel, so a launcher can drive it and photograph it:

```sh
cargo run --release -p stkcraft -- --control 7979      # or STKCRAFT_CONTROL_PORT=7979
# control: listening on 127.0.0.1:7979, token in /run/user/1000/stkcraft/control-7979.token
```

`auth` with the token from that file, then `ui.state` (what the window is showing), `ui.screenshot`
(the frame it paints next, written to `screenshots/` under the data directory and returned as PNG
bytes in base64), `engine.execute`, `scenario.read`, and the stateless half — `version`,
`engine.commands`, `knowledge.*` — which is answered by the socket thread's own session.

The split is the design: `engine.execute` through the window changes the scenario a person is looking
at, not a copy. A window-only method that cannot be answered — a capture with no painted frame, a
window that is minimised — returns the reason rather than an empty image, because a launcher told
"ok" with a picture nobody drew is worse than one told why it waited.

`stkcraft control --port 7979` still serves the same protocol with no window at all, which is what a
build machine and a test use.

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

**Known upstream failure.** The shared `Packaging lint` runs `Get-ChildItem
packaging/windows/*.ps1` without an existence guard. On a checkout with no such directory that
glob is an error under the runner's `$ErrorActionPreference = 'Stop'`, so a pull request that
touches `assets/app-icon/` fails a lint about a package that does not exist. Tracked as
[craft-common-actions#145](https://github.com/storytold/craft-common-actions/issues/145). This
repository keeps `packaging/windows/README.md` so the directory exists and the glob returns
nothing; the fix still belongs upstream, and the README can stay as the note on what lands there.

## Contributor credits (About window)

`contributors/contributors.json` is generated at release time by the shared
`storytold/craft-common-actions` `refresh-contributors` workflow; it is not committed and is never
hand-edited. `contributors/people.toml` holds names people chose for themselves. **Never add, edit
or guess anyone else's entry.** If your human's GitHub username has no `[people.<username>]` entry,
ask them once whether they want to be credited by more than their username.
