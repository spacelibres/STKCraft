# Code and document sync review, 2026-11-07

A second review, after `2026-11-07-documentation-remediation.md`, that compares the documents with the
code. Each claim was checked against the code it describes. Where the code is the working system, the
document was changed to describe it. Where the code is missing a stated behaviour, the gap is recorded
below and the document says it is not built. Plans are history: this file is not updated after it lands.

## Documents corrected to match the code

- `docs/design/architecture.md`: the framework list now includes `storage`, and the agent list includes
  `control`. Both crates are in `xtask/src/layers.rs`.
- `docs/standards.md`: the layer ranges are framework 0–3, astro 2–4, agent 3–6, ui 5, and the apps
  above all analysis crates (`APP_LAYER`, one above `LAYER_COUNT`). The earlier "framework ≤ 4, apps 6" was
  wrong. The storage crate is no longer marked "(next)". The agent row includes `control`. The control
  method list names `auth`. The four shipped skills are named.
- `AGENTS.md`: the layering rule refers to `GUI_CRATES`, which is the real list, rather than the four
  crates named in prose. The CLI section now includes `passes`, `planet`, `model` and `version`. The
  `cargo xtask` section includes `version set`, `standards`, `naif`, `mathlib`, `verify-formulas`,
  `layers`, `wasm` and `assets`. The ephemeris fetch is described as one file at a time, each with
  parallel ranged downloads.
- `docs/design/storage.md`: the `sensor` table is removed, because the code has none; sensors and
  targets are not yet a table. The `formulas` row is replaced by the `formula` table the code creates in
  migration V2. The directory tree lists only what the code writes (`sessions/` and `cache/knowledge/`).
  The data root is described by its real resolution order, starting with `STKCRAFT_DATA`, and the
  configuration directory is described per platform.
- `docs/design/knowledge.md`: the sessions location refers to the data directory, not a fixed Linux path.
- `docs/design/mathematics.md`: the SQL block is replaced by the schema in `crates/framework/storage/src/schema.rs`.
  The differences from the first design are stated: no `surroundings` column, one digest pair, and no
  foreign key. The recogniser environment variable and the `unrecognised` value are marked as not read or
  written by any code, and the OCR hook is marked as existing.
- `docs/guides/ephemeris-data.md`: the path separator for `STKCRAFT_EPHEMERIS_KERNEL` is platform-specific,
  `:` on Unix and `;` on Windows, as `std::env::split_paths` implements it.

## Gaps in the code, recorded and not changed

- **Recogniser and classifier layers (3 and 4).** Not built. `STKCRAFT_FORMULA_RECOGNIZER` is read by no
  code, and no code writes the `unrecognised` value. `docs/design/mathematics.md` now says so.
- **Formula store has no production writer.** `record_formula` is called only from tests. The table is
  real; nothing fills it yet.
- **`cargo xtask release` is missing from the help text,** and the help text lists six entries twice
  (`standards`, `naif`, `math-regions`, `formulas`, `mathlib`, `verify-formulas`). This is a code
  change for `xtask/src/main.rs`, not a document change.
- **Layer check depends on direct declarations only.** `xtask/src/main.rs` checks the GUI rule against
  each crate's declared dependencies, not transitive ones. The docs say "may depend", which is accurate
  for the rule as written, but a transitive GUI dependency would not be caught.
- **Ephemeris test skip.** When no kernel is present, the reader tests print a message and return, so
  they report as passing. `docs/guides/ephemeris-data.md` says tests are skipped, which is accurate, but
  a skipped test is indistinguishable from a passed one in the test summary.
- **Assistant prompt numbering.** The knowledge-first prompt in `crates/agent/runtime/src/lib.rs` has two
  rules numbered 2. Harmless to the model, but a reader of the prompt will be confused.
- **Hidden directories.** The library skips hidden files but still descends into hidden directories
  (`crates/agent/knowledge/src/library.rs`). `docs/design/knowledge.md` says "hidden files", which is
  correct; the directory behaviour is not documented.
- **Historical env var.** `DEEPSEEK_TIMEOUT_SECONDS` appears only in `CHANGELOG.md` (0.1.x entry). No code
  reads it and no current document names it. It is left as history.

## Verification

Crate names and layers were compared with `Cargo.toml` and `xtask/src/layers.rs`. CLI commands were
compared with `apps/stkcraft/src/cli.rs` and the `cargo xtask` dispatch in `xtask/src/main.rs`. Constants
were compared with their definitions in `crates/agent/knowledge`, `crates/agent/runtime`,
`crates/framework/storage`, `crates/framework/time` and `crates/framework/core`. The control method list was
compared with `crates/agent/control/src/lib.rs`. The ephemeris checksums were compared with the files in
`data/` in the previous review. No gate was run in this review; the findings come from reading the code.

## Follow-up: what was implemented, and what could not be

Implemented, as code changes not yet compiled or committed:

- `xtask/src/main.rs`: the help text lists `release` and no longer repeats six entries.
- `crates/agent/runtime/src/lib.rs`: the assistant's system prompt numbers its rules 1 to 6 once each.
  Its rule 2 was numbered twice.

Not implemented, with the reason:

- The sandbox has no Rust toolchain and no network access to crates.io or static.rust-lang.org, so
  nothing can be compiled or tested here. The project's gate (`cargo xtask ci`) therefore has not run
  against these changes, and they must not be committed until it has.
- The working tree already holds uncommitted work from other changes (for example the F2c runtime
  files and a substantial edit to `crates/agent/runtime/src/lib.rs`). A commit made from this tree would
  mix that work into this one, which `CONTRIBUTING.md` forbids.
- The version bump and CHANGELOG entry are deferred. Cargo.toml is at 0.1.85, and an uncommitted 0.1.86
  fix is described in `plan/PROGRESS.md`; choosing the next number is the owner's decision.
- Larger "not built" items (formula recogniser, DOCX/XLSX, OCR, embeddings, PostgreSQL, extension store,
  F2 context memory) were left for a later pass, as the user chose the small-items scope.
