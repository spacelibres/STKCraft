# Mathematics as a first-class store

A mission-analysis platform is asked questions whose answers are formulas: *what does the ODM mean by
mean elements*, *what is the off-nadir limit for this attitude*, *which force model does this paper
integrate*. If the formula is misread, every number computed from it is wrong in a way nobody notices —
which is the failure this project exists to prevent. So mathematics is not a kind of prose that happens
to contain symbols. It is a separate store, with its own recogniser, its own confidence and its own
citation form.

## The rule

**A formula is never guessed.** Every stored formula carries:

- what the recogniser produced (LaTeX where it has one, Unicode where it has only characters);
- **the source**: the page, the bounding box, and the image bytes of the region, so a person can look at
  exactly what was read;
- **the confidence and the engine+version** that produced it — `deterministic/text-layer`,
  `model/<name>/<version>`, or `unrecognised`;
- a digest of the region, so a re-run can prove it read the same pixels.

A region that cannot be recognised is stored as **`unrecognised` with its image and its box**, and the
assistant is told so. An invented formula is worse than a missing one: a missing formula says "look at
page 12", an invented one says "trust this number".

## Why a separate store and not a paragraph

Prose and mathematics fail differently, are recognised by different means, and are cited differently.

| | Prose | Mathematics |
|---|---|---|
| found by | the text layer, or OCR | a text layer with math fonts, or a formula model |
| unit of citation | a line range | a **page and a box**, because a formula has no line |
| failure mode | a wrong word, detectable by reading | a plausible rearrangement — subscript moved, `∫` read as `f` — undetectable |
| retrieval | BM25 over words | LaTeX tokens *plus* the Unicode fallback *plus* the sentence that introduces it |

Mixing them is what makes a wrong formula invisible: it joins the prose, it gets a line range, and no
test can tell it apart from the sentence around it.

## The pipeline, in layers

Each layer is separately testable and separately skippable, and the later ones only run on regions the
earlier ones could not settle.

1. **Text layer, deterministic.** The interpreter already delivers every glyph with its font, size and
   matrix (`crates/agent/knowledge/src/pdf.rs`). Display mathematics is recognisable without a model:
   a run of glyphs whose font differs from the body (math italic, symbol faces), whose baseline sits
   off the text line (sub/superscripts), or whose characters are drawn from the math blocks
   (U+2200–U+22FF operators, U+0370–U+03FF Greek, U+2A00–U+2AFF supplemental operators, ∫ ∑ ∂ ∇ √).
   The result is **Unicode text plus a box**, with the font names kept as evidence.
2. **Layout, deterministic.** A region is *display* mathematics when it is centred or indented against
   the body margin, spaced away from the text above and below, and holds an operator that cannot be
   prose. This is what distinguishes an equation from a symbol mentioned mid-sentence.
3. **Model, for region images.** A formula recogniser converts the region's image to LaTeX. It is
   **optional and external** — the same pattern as the model transport, which is `curl` rather than a
   linked HTTP client: a configured command
   (`STKCRAFT_FORMULA_RECOGNIZER="<command>"`) that takes a PNG on stdin and writes LaTeX on stdout,
   with the engine and version recorded. Nothing is linked in; a build with no recogniser stores
   `unrecognised` boxes rather than failing.
4. **OCR, for pages with no text layer.** A scanned page is recognised to prose by the same external
   pattern (`STKCRAFT_OCR="<command>"`, PNG in, text out), and its formulas then go through layer 3.
   The two are separate knobs because they fail separately: a page can have a good text layer and no
   formula model, or an OCR pass that gives prose and nothing else.

Layers 1 and 2 are ours, deterministic, offline and testable to the character. Layers 3 and 4 are
external because the honest state of formula recognition is *models*, and a model is configuration in
this project, not a dependency baked into the binary.

## What a formula is for, and what mathematics it is

A formula on page 12 is not useful because it has a LaTeX spelling. It is useful because a person can
ask *what does this solve* and *which mathematics is this*, and get an answer that points at both the
paper and the concept. Two fields carry that, and neither is guessed:

- **`problem`** — what the formula is for, in the paper's own words: the sentence or caption that
  introduces it, captured with the region. A formula without its sentence is a fragment; the sentence
  is what makes it citable.
- **`classification`** — the mathematics it belongs to, resolved against **Mathlib**, with a score and
  the declaration it matched.

### Mathlib as the vocabulary

Mathlib is the largest machine-readable body of mathematics that exists under a licence this project can
use (Apache-2.0), and its *taxonomy is the useful part*: `Mathlib/Analysis/SpecialFunctions/Sqrt.lean`
says what a square root is, `Mathlib/Analysis/Calculus/FDeriv/Basic.lean` says what a derivative is, and
the docstring of each declaration says what it is for.

So the integration is an **index, not a dependency**: `cargo xtask mathlib` fetches a pinned Mathlib
checkout, parses it for declarations (`def`, `theorem`, `lemma`, `structure`, `class`) with their module
path, statement and docstring, and writes one table:

```sql
CREATE TABLE mathlib_declarations (
    name          TEXT PRIMARY KEY,   -- `Real.sqrt`, `HasDerivAt`, `Finset.sum`
    kind          TEXT NOT NULL,      -- def | theorem | lemma | structure | class
    module        TEXT NOT NULL,      -- `Mathlib/Analysis/SpecialFunctions/Sqrt.lean`
    domain        TEXT NOT NULL,      -- the top-level taxonomy: Analysis, Algebra, Topology, …
    statement     TEXT,               -- the declaration's type, as written
    docstring     TEXT,               -- what it is for, verbatim
    digest        TEXT NOT NULL       -- SHA-256 of the declaration text, for reproducibility
) STRICT;
```

Nothing of Lean is linked and no proof is checked: this is a **lexicon of names, statements and
purposes**, which is exactly what classification needs and what a Rust program can hold.

### How a formula is classified

Deterministic, scored, and honest about being a guess with a number:

1. **Symbol normalisation.** The recognised LaTeX and its Unicode fallback are normalised to canonical
   operators (`√`→`sqrt`, `∑`→`sum`, `∂`→`deriv`, `∫`→`integral`, `|x|`→`abs`, …). Math symbols in the
   text layer are the strong evidence; OCR'd symbols are the weak kind and are recorded as such.
2. **Candidates.** Declarations whose name or statement contains the normalised operators, fetched from
   an inverted index over `mathlib_declarations` — the same interned index the knowledge base uses.
3. **Score.** Operator overlap, arity shape (a derivative has one argument and one variable, a sum has an
   index and a body), and the paper's own surrounding words against the declaration's docstring. Each
   formula keeps its top candidates with their scores.
4. **Domain.** The winning candidate's module path yields the taxonomy (`Analysis`, `Algebra`,
   `Topology`, `Probability`, `Geometry`, `Numerics`), which is what makes a corpus *classifiable*: a
   library can be listed by domain, a search can be restricted to one, and a mission engineer can ask
   "which of my papers are about orbit determination as a numerical problem".

**What this is not.** It is not verification, and it is not a proof. A classification says *this formula
is about `Real.sqrt`, with score 0.83, and the words around it match `Real.sqrt`'s docstring*. The
assistant is required to present it that way — "the library's formula for X (page 12) matches Mathlib's
`Mathlib.Analysis.SpecialFunctions.Sqrt.sqrt`, which is defined as …" — and never as "this paper proves
the theorem". A wrong classification costs a bad tag; a fake proof costs the project its reason to exist.

## Provenance: what makes a formula checkable

A classification is only worth something if the thing being classified can be traced back to a place in
a file that a person can open. So every formula carries a **chain**, and every link in it is immutable:

| Link | What it is | Why it can be trusted |
|---|---|---|
| the library | the manifest of every file's path, size and modification time | the key the index cache already uses |
| the file | **SHA-256 of the PDF** | the same digest `cargo xtask standards` verified when it downloaded it |
| the page | one-based page number | a page cannot be renumbered by a later version of the file, a line can |
| the box | `(x0,y0)-(x1,y1)` in PDF points, after rotation | what a person selects on the page |
| the pixels | **SHA-256 of the region image** | re-extracting the same box must produce the same bytes, or the chain is broken |
| the reading | `source`, engine and version, and the glyph/font evidence for layer 1 | a recogniser is identified, not anonymous |
| the wording | the introducing sentence (`problem`) and its line range in the text layer | the paper's own words, cited as written |
| the classification | Mathlib revision, declaration name, **SHA-256 of the declaration's statement and docstring** | the concept is pinned to a commit, so "what Mathlib said in 2026" is checkable in 2030 |

That chain is stored with the formula and printed with it:

```text
paper.pdf:page 12, formula at (120,340)-(300,388) — Real.sqrt (Analysis, 0.83)
  problem: "The slant range follows from the law of cosines:"
  paper sha256 6afd…, region sha256 391f…, text-layer/0.1.50, mathlib c5a1…/Real.sqrt
```

**The chain is re-verifiable, and that is the point.** `cargo xtask verify-formulas` re-reads every
document in the transcription table, re-extracts each recorded box, re-runs the classifier, and fails on
drift — a moved box, a changed digest, a classification that no longer reproduces. Without that command
the provenance is a snapshot of an afternoon; with it, the claim "this formula is about `Real.sqrt`" is a
statement that either still holds or loudly does not.

### What provenance cannot claim

- **A paper is not the original source.** If the paper quotes a formula from Vallado, the chain leads to
  *the paper we read*, not to Vallado. When the introducing sentence carries a citation, it is recorded
  as `as cited in` — the honest form — and never presented as the primary source.
- **A stored digest proves bytes, not meaning.** It proves the same pixels produced the same reading; it
  does not prove the reading is right. That is what the transcription table is for.
- **A Mathlib match is a lexical and structural match.** It says the formula is *about* a concept that
  has a name, not that it equals the declaration. `mathlib_score` is what carries the strength, and the
  assistant states it.

## Where a formula lives

The relational store (SQLite, `docs/design/storage.md`) gains one table, because a formula is a *record*
with provenance:

```sql
CREATE TABLE formulas (
    id            INTEGER PRIMARY KEY,
    document      TEXT NOT NULL,          -- path relative to the library root
    page          INTEGER NOT NULL,
    x0, y0, x1, y1 REAL NOT NULL,         -- the box, in PDF points
    latex         TEXT,                   -- when the recogniser produced it
    unicode       TEXT,                   -- always: what the text layer says, if anything
    surroundings  TEXT,                   -- the sentence that introduces it, for retrieval
    problem       TEXT,                   -- the sentence or caption that introduces it: what it solves
    source        TEXT NOT NULL,          -- deterministic/text-layer | model/<engine>/<version> | unrecognised
    confidence    REAL,                   -- when the source reports one
    mathlib_name  TEXT,                   -- the declaration it matched, when it matched one
    mathlib_score REAL,                   -- and how strongly
    mathlib_domain TEXT,                  -- the taxonomy the match implies
    classifier    TEXT,                   -- the scoring engine and version, like `source`
    -- provenance: every link that makes the row checkable
    paper_digest  TEXT NOT NULL,          -- SHA-256 of the PDF the box was read from
    region_digest TEXT NOT NULL,          -- SHA-256 of the region image
    mathlib_rev   TEXT,                   -- the Mathlib revision the index was built from
    problem_lines TEXT                    -- the line range of `problem` in the text layer

    digest        TEXT NOT NULL,          -- SHA-256 of the region image
    FOREIGN KEY (document) REFERENCES documents(path)
) STRICT;
```

The knowledge index (`crates/agent/knowledge/src/index.rs`) indexes four things from each row: the LaTeX
tokens, the Unicode fallback, the sentence that introduces it, and the Mathlib name it was classified
as — so a search for `off-nadir` finds the equation whose caption says off-nadir, a search for `\frac`
finds it too, and a search for `sqrt` finds the formula classified as `Real.sqrt`. The citation a hit prints
is `paper.pdf:page 12, formula at (120, 340)-(300, 388)`, not a line range: a box is what a reader can
find on a page that has no lines.

## What "recognised well" will be measured against

Not "it looks right". The corpus already contains the standards
(`docs/guides/ccsds-standards.md`, 495 documents) and papers whose formulas are published elsewhere in
typeset form. The acceptance test is a **transcription table**: a set of (document, page, box, expected
LaTeX) rows, chosen from the corpus, where a mismatch is a failing test. It starts small — a dozen
formulas across CCSDS and the SGP4 papers — and grows. A recogniser that cannot pass it is not shipped
as the default.

## Order of work

1. **Detection and storage** (ours, deterministic): segment math regions from the text layer, store them
   with their boxes and Unicode, index them, and cite them by box. A test proves a display equation in a
   CCSDS PDF is found on the page it is on and stored with its glyphs.
2. **The transcriptions and the problem sentences**: each row of the table records the expected LaTeX
   *and* the sentence that introduces the formula, so "what does it solve" is measured with the formula.
3. **OCR and the formula model** behind the two environment commands, with `source` and `confidence`
   recorded, and the `unrecognised` path tested — including that it never produces text it did not read.
5. **Agent and panel**: `knowledge.search` returns formulas as their own hits with the box citation, and
   a formula is quoted with its page so a person can check the pixels.
