# Expert and formula foundation — implementation plan

**Status:** Phase 0–2 implemented on the working tree, not yet committed. Phase 3 (answer contract)
is next. The durable contract is `docs/design/experts.md`; this file is the execution plan.

## Context

The kernel already has reference-verified propagation, time scales, frames, ephemerides and access
geometry. The knowledge library already extracts document text, passages and formula regions, and
indexes formulas separately. Mathlib supplies a mathematical vocabulary through
`cargo xtask mathlib`.

What does not exist is the layer that turns those pieces into **trusted domain expertise**:

- no expert package: scope, sources, formulas, tools, refusal conditions and evaluations;
- no authoring record that ties a formula to a source page, an implementation and a test;
- no validator that fails when a `verified` formula loses its test or source;
- no answer contract that makes every expert answer carry its formula id and limitations.

The first expert is **Time & Frames**, because every other expert's coordinates and instants depend
on it, and because its kernel tests already exist.

## Scope

In scope:

- one expert package for Time & Frames;
- the formula authoring schema and its validator;
- a small evaluation set for that expert;
- the answer contract for formula citations and limitations.

Out of scope for this foundation:

- new propagation, coverage, RF, thermal or power features;
- formula-recognition models (layer 3 in `docs/design/mathematics.md`);
- a multi-agent scheduler or task graph;
- release packaging, installers and publishing.

## Deliverables

1. `docs/design/experts.md` — the durable contract (written).
2. `share/experts/time-frames/` — `EXPERT.md`, `sources.toml`, eight formula records,
   `evaluations/`, `limitations.md`.
3. A formula authoring and validation layer in `stkcraft-agent-knowledge` plus
   `cargo xtask experts check`.
4. `cargo xtask ci` includes the new check, and the gate stays green.
5. `CommandOutcome` carries limitations; `access.*` and `planet.access` fill them; the text and
   `structuredContent` answer include them.
6. Evaluation cases pass: in-range, boundary, missing-source and refusal cases.

## Phase 0 — freeze the contract

Acceptance:

- `docs/design/experts.md` names the three stores, the source hierarchy, the package layout and the
  formula record.
- The formula status lifecycle is explicit and validator-enforceable.
- No runtime behaviour changes.

## Phase 1 — the Time & Frames package

Create the package with these records. Each record names a standard/paper source, the kernel
function that implements it and the test that proves it.

| Formula id | Content | Source | Implementation | Verification |
|---|---|---|---|---|
| `time.tai-utc.leap` | TAI − UTC from the leap-second table | IERS Bulletin C / SOFA table | `framework/time/src/leaps.rs` | leap-second tests |
| `time.tt-from-tai` | TT = TAI + 32.184 s | IAU 1991 defining constant | `framework/time/src/epoch.rs` | scale round-trip test |
| `time.tdb-minus-tt` | TDB − TT periodic series | IAU 2006 / ERFA `eraDtdb` | `framework/time/src/tdb.rs` | 20 µs ERFA comparison |
| `time.ut1-from-utc` | UT1 = UTC + DUT1 | IERS Conventions 2010 ch. 8 | `framework/time/src/epoch.rs` | refusal without DUT1 |
| `frames.gmst.iau82` | Greenwich mean sidereal time | Vallado 2013 eq. 3-45 / ERFA | `astro/frames/src/sidereal.rs` | published GMST value |
| `frames.precession.iau77` | IAU-77 precession angles and matrix | Vallado 2013 eq. 3-56–3-58 | `astro/frames/src/precession.rs` | 15-digit reference matrix |
| `frames.nutation.iau80` | IAU-80 106-term nutation | IERS 1980 / ERFA `nut80` | `astro/frames/src/nutation.rs` | 106-term table and reference values |
| `frames.teme-to-itrf` | TEME → ITRF with EOP | Vallado 2013 Algorithms 23/26 | `astro/frames/src/teme.rs` | worked example and EOP range tests |

Package rules:

- `sources.toml` records edition, DOI/URL, optional local path and SHA-256. An external standard may
  have an empty path; a source stored locally must have a matching digest. The citation is always
  mandatory.
- Every formula has `limitations`; for this package the first two are **IAU-77/80, not IAU
  2006/2000A** and **EOP default to zero until an IERS loader exists**.
- No formula record is `verified` unless its test exists and asserts a published value or tolerance.

Acceptance:

- `cargo xtask experts check` passes on the package.
- The validator fails on: missing source, missing test for `verified`, missing implementation path,
  mismatched source digest, unknown expert id.

## Phase 2 — validator and schema loader

Add to `stkcraft-agent-knowledge` (or a small `stkcraft-agent-expert` crate if the layer needs it):

- `FormulaRecord` deserialized from the TOML above;
- `ExpertManifest` from `EXPERT.md` metadata plus `sources.toml`;
- status and reference checks;
- duplicate id checks and source-digest checks.

Add `cargo xtask experts check`:

- walks `share/experts/*/formulas/*.toml`;
- validates required fields and the status rules;
- checks `implementation.path` and `verification.test` exist;
- reports coverage per expert and fails on any violation.

Add it to `task_ci` as a tenth step, so the gate cannot silently lose the formula layer.

Acceptance:

- A deliberately broken record makes the check fail with a message naming the file and field.
- The check is deterministic and offline.
- `cargo xtask ci` reports ten steps and is green.

## Phase 3 — answer contract

- Add `CommandOutcome.limitations: Vec<String>` (empty by default).
- `access.compute`, `access.targets` and `planet.access` add:
  - sampling step and the possibility of missing a short window between samples;
  - target access is the union of vertex windows, not polygon interior coverage;
  - geometric line of sight: no refraction, light time or aberration;
  - zero Earth-orientation parameters until an EOP loader exists.
- The text answer and `structuredContent` include the same list.
- A computed answer names the command and the formula ids it used when a formula record exists.

Acceptance:

- Every access/pass answer carries the limitations in both text and JSON.
- A test asserts the limitations survive the answer formatter.
- No answer presents an `extracted` or `classified` formula as verified.

## Phase 4 — SGP4 and Access experts

Only after Time & Frames passes its evaluation:

- SGP4 package: AIAA 2006-6753, 33 vectors, error codes, decay and validity limits.
- Access package: WGS84 geometry, sensor cone, sampling and union limits.
- Each package gets its own evaluation set; reuse the same validator.

## Evaluation set for Time & Frames

Minimum cases:

1. `UTC 2017-01-01` leap second: TAI − UTC = 37 s.
2. `TT` at a known epoch differs from TAI by 32.184 s.
3. `TDB − TT` at J2000 matches ERFA within 20 µs.
4. GMST at a published epoch matches ERFA to the recorded tolerance.
5. IAU-77 precession matrix at J2000 is identity; at a published epoch matches the reference.
6. IAU-80 nutation at J2000 matches the published obliquity and longitude values.
7. TEME → ITRF worked example matches the published state.
8. UT1 without DUT1 is refused, not defaulted.
9. A date before 1972 or beyond the leap table is flagged, not guessed.
10. A missing EOP is reported as a limitation, not hidden.

Each case records: input, expected value or expected refusal, formula id, source, test path.

## Risks and decisions

- **Clean room:** formulas come from public standards/papers; reference implementations are read for
  values and algorithms, not copied. Mathlib is Apache-2.0 and is a vocabulary, not an authority.
- **Source drift:** a checksum pins the document; a changed document makes the record stale and the
  validator fails.
- **Formula vs code drift:** `implementation.path` and `verification.test` are checked; a formula
  whose code moves must be updated in the same change.
- **Overclaiming:** the expert's `limitations.md` is part of the answer contract. A specialist name
  never removes a limitation.
- **Scope creep:** the multi-agent executor is not required to prove the formula layer. Build the
  evidence pack first; add roles only when an evaluation shows they improve an answer.

## Definition of done

- `share/experts/time-frames/` exists with eight formula records and a source table.
- `cargo xtask experts check` passes and is a CI step.
- The Time & Frames evaluation set passes.
- Access answers carry their limitations in text and JSON.
- `cargo xtask ci` is green.
- `docs/design/experts.md` and this plan describe what exists, with no unbuilt capability claimed.
