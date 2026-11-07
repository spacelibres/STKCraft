# Domain experts and the formula library

A specialist in STKCraft is a **versioned evidence package**, not a persona prompt. The package
says what the expert covers, which sources and formulas it may use, which deterministic code
executes them, which evaluation cases prove the answers, and when it must refuse.

A role name is never evidence. An answer that says "the orbit expert says so" without a formula
record, a source citation and a verified computation is not an answer.

## Three stores

| Store | Holds | Authority |
|---|---|---|
| Knowledge | documents, pages, passages, citations | the document itself |
| Formulas | authoring records: expression, symbols, validity, source, implementation, verification | the standard or paper, plus the test that proves the implementation |
| Experts | scope, non-goals, allowed tools, required evidence, refusal conditions, evaluations | the package, versioned with the repository |

The existing knowledge library extracts formula regions from PDFs (`Formula`: page, box, Unicode,
introducing sentence). That is an **extraction** record. The formula library below is an
**authoring** record and is the one an expert may cite. A later step links the two.

## Source hierarchy

Every formula names the highest applicable layer:

1. **Standard or specification** — IERS Conventions, IAU resolutions, CCSDS blue books, ISO,
   ITU-R, ECSS. The text, edition and equation number are the authority.
2. **Peer-reviewed paper** — for example AIAA 2006-6753 for SGP4, Vallado for the frame
   algorithms. The paper and its published verification values are the authority.
3. **Reference implementation** — IAU SOFA/ERFA, NASA SPICE, published reference output. Used for
   comparison; code is not copied.
4. **Mathlib** — Apache-2.0 mathematical definitions and classification. Mathlib supplies the
   mathematical vocabulary, not the space-domain constants or the engineering assumptions.
5. **Adoption record** — which institution, mission or standard uses the result. "Professional
   practice" is recorded as a source, never asserted from memory.

An expression that is only in a model's memory is not in the library.

## Expert package layout

```text
share/experts/<expert-id>/
  EXPERT.md          # scope, non-goals, inputs/outputs, tools, refusal conditions
  sources.toml       # documents: title, edition, DOI/URL, local path, SHA-256
  formulas/*.toml    # authoring records (schema below)
  evaluations/       # cases with expected answers, citations and refusals
  limitations.md     # sentences that must travel with every answer
```

`EXPERT.md` is a contract, not a biography. It names:

- the questions the expert may answer;
- the questions it must hand back or refuse;
- the engine commands it may run;
- the formulas it may cite;
- the limitations every answer must carry.

## Formula record

A source is registered once, in `sources.toml`:

```toml
[[source]]
id = "iers-2010"
title = "IERS Conventions (2010)"
edition = "IERS Technical Note 36"
url = "https://www.iers.org/IERS/EN/Publications/TechnicalNotes/tn36.html"
path = ""          # empty: external, not carried by the repository
sha256 = ""        # required only when path is non-empty
adopted_by = ["IERS"]
```

A formula file names a source id and a locator; a file may hold several `[[formula]]` records:

```toml
schema = 1
expert = "time-frames"

[[formula]]
id = "frames.gmst.iau82"
name = "Greenwich mean sidereal time"
domain = "time-frames"
expression_latex = "..."
expression_unicode = "..."
source = "vallado-2013"
source_locator = "eq. 3-45; ERFA eraGmst82"
mathlib = "none"
assumptions = ["UT1, not UTC", "IAU-82 expression"]
validity = "|UT1 - UTC| < 0.9 s; within the leap-second table"
status = "verified"
limitations = ["IAU-82; not IAU 2006/2000A"]

[[formula.symbols]]
symbol = "theta"
unit = "rad"
dimension = "angle"
meaning = "Greenwich mean sidereal time"

[formula.implementation]
crate = "stkcraft-astro-frames"
path = "crates/astro/frames/src/sidereal.rs"
function = "greenwich_mean"

[formula.verification]
reference = "ERFA eraGmst82 / Vallado AstroLib::gstime"
case = "..."
tolerance = "1e-12 rad"
test = "sidereal::tests::gmst_matches_the_published_reference"
```

### Status lifecycle

```text
extracted -> classified -> implemented -> verified
                                  \-> rejected
```

Rules enforced by the validator, not by trust:

- `extracted` and `classified` may be knowledge-only; they must not be presented as computed.
- `implemented` requires an existing `implementation.path` and `function`.
- `verified` requires a source citation, a reference value, a tolerance and an existing test.
- A formula with no source cannot leave `extracted`.
- A source stored locally must record `path` and a matching `sha256`; an external standard may keep
  `path = ""` because the repository does not carry the PDF. A citation is still mandatory.
- The validator fails on a `verified` formula whose test path no longer exists.

## Validation

`cargo xtask experts check` reads every `share/experts/*/sources.toml` and
`share/experts/*/formulas/*.toml` and fails on:

- a duplicate source or formula id;
- a formula whose `source` is not in `sources.toml`;
- an `implemented` or `verified` formula whose implementation file or function is missing;
- a `verified` formula whose reference, case, tolerance or test is missing, or whose test name is
  not in the implementation file;
- a local source whose SHA-256 does not match its path;
- an empty symbol, assumption or limitation.

The check is the tenth step of `cargo xtask ci`. It reads files only; it does not run the kernels.

## Answer contract

Every expert answer carries:

1. the formula id and the source citation (`document, edition, page/equation`);
2. the engine computation identity (command, version, inputs digest) when a number was computed;
3. the assumptions and validity range;
4. the expert's `limitations.md` sentences;
5. either a verified record or an explicit statement that the library does not cover the question.

Two failure modes are forbidden:

- **A formula from memory.** If no record exists, say the library does not cover it.
- **A number without a computation.** A knowledge-only expert may explain a method, but it may not
  present a numeric result as if the engine had produced it.

The existing access limitations are part of this contract: point sampling, union of vertex windows,
geometric line of sight, no refraction/light-time/aberration, and zero Earth-orientation parameters
until an EOP loader exists.

## Non-goals for this foundation

- No model-generated formula is stored as a formula.
- Mathlib is not a physics authority.
- A prompt-only "expert" is not a specialist.
- The multi-agent executor, task graphs and durable scheduling are not required to start; the
  evidence pack and its validator come first.
- No new propagation, release or packaging work while this foundation is being built.
