# Documentation remediation, 2026-11-07

A record of a review of the rule books and design documents, and of the corrections made from it.
Each item names the source it was checked against. Plans are history: this file is not updated after
it lands, and a reader who wants the current truth reads `docs/standards.md`, `AGENTS.md`,
`CONTRIBUTING.md` and `docs/design/`.

## Method

Every conflict was checked against the code or the file it concerns before it was changed. Where the
code was the authority, the document was changed to match it. Where the rule book was the authority,
the lagging document was changed to match the rule book. Nothing was changed on the strength of an
unverified summary.

## Corrections made

**Root rule files**

- `CONTRIBUTING.md`: the `cargo xtask ci` step list now names every step the task runs (`fmt`,
  `clippy`, `test`, `assets`, `i18n`, `api-docs`, `layers`, `wasm`, `version`, and the `Cargo.lock`
  check). The version example uses `cargo xtask version bump` and `set X.Y.Z`, matching `AGENTS.md`.
  The changelog subsections now match `AGENTS.md`: Added, Changed, Fixed, Not built yet.
- `.gitignore`: the comment names `docs/guides/ephemeris-data.md`, the file's actual path.

**Standards and design**

- `docs/standards.md` §1: the catalog path is `share/locales/<locale>/stkcraft.tsv`, as §9 and the
  shipped tree require. §8 states that the knowledge index is cached and that the corpus fetchers are
  built, which the code shows.
- `docs/design/architecture.md`: `crates/framework/storage` is no longer listed as planned, because it
  exists and is in the layer table. The PDF-ingestion crate is not a separate planned crate: PDF reading
  lives in `crates/agent/knowledge`.
- `docs/design/storage.md` §6: the corpus fetchers are built (`cargo xtask papers`,
  `cargo xtask standards`). The `formulas` table is listed in §1 as planned before any code writes it,
  as §5 requires.
- `docs/design/knowledge.md`: page-level PDF citations and the index cache are built, so they are
  removed from "Where this is going". `.docx` and `.xlsx` extraction remains.
- `docs/design/mathematics.md`: the `formulas` table had a duplicate `digest` column, a missing comma
  after `problem_lines`, and a foreign key to a table that does not exist. It now references
  `knowledge_document(path)`, the table storage.md defines. The Mathlib command reads a checkout the user
  supplies through `MATHLIB_DIR`; the task does not fetch or pin one, so the text no longer says it does.
  The provenance example is marked as illustrative.
- `docs/design/agents.md`: two clarifications. Model transport is an explicit exception to the retry
  rule and is never retried automatically. Maintenance commands such as `cargo xtask i18n translate` are
  not development workers and run under the maintainer's own key.

**Guides**

- `docs/guides/ephemeris-data.md`: the table lists all twelve fetched files, including the Earth
  orientation, leap-second, planetary-constant, mass and star-catalogue files that were missing. Each
  checksum is the one in `xtask/src/main.rs`, and each matches the local file in `data/`. The text no
  longer claims each checksum was verified against CSPICE, which is true only for the kernels.
- `docs/guides/paper-corpus.md`: the CCSDS formats are described as planned, matching `sources.md`.
- `docs/guides/localization.md`: the in-repository catalog paths are `share/locales/<locale>/...`. The
  `locales/<locale>.tsv` paths that remain refer to the user's configuration directory and are correct.

**Roadmap**

- `ROADMAP.md`: the compaction row is open, not done, because F2.1 is unfinished. The "Then" line no
  longer lists the withdrawn role-contract and executor work as scheduled. The duplicate F5 heading is
  renamed to F5a for the shipped part.

**Local plan notes** (`plan/`, gitignored, changed for consistency only)

- `plan/05-provider-protocol.md`: the header states that F2b shipped by 0.1.85.
- `plan/01-data-foundation.md`: the stack line no longer names the Rust version or an external SHA-256
  tool.

## Deliberately not changed

- **Historical changelog headings.** `CHANGELOG.md` contains sections headed Verified, Measured and
  others. The published history is not rewritten. New sections follow `AGENTS.md`.
- **Version 0.1.86.** `plan/PROGRESS.md` records a verified 0.1.86 crash fix, but `Cargo.toml` and the
  changelog stop at 0.1.85, and the fix is uncommitted. Committing it is the owner's decision, because a
  commit under this rule also bumps the version and writes its changelog section.
- **Task graph and family boundary.** Whether a task graph is built, and how it fits with the family's
  no-parallel-job-system rule, is an open architectural decision. It is recorded here rather than decided
  in a documentation pass.
- **ECSS.** It remains unfetched, as `docs/guides/sources.md` states.

## Verification

- Every checksum in `docs/guides/ephemeris-data.md` was compared with the sha256 of the matching file in
  `data/`; the two sets are identical.
- The `cargo xtask ci` step list in `CONTRIBUTING.md` was compared with `task_ci` in `xtask/src/main.rs`.
- The repository root was checked after the change: no document is in the root except the rule files,
  the manifests, the licence texts and the directories the layout in `docs/standards.md` §9 names.
