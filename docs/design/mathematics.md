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
    source        TEXT NOT NULL,          -- deterministic/text-layer | model/<engine>/<version> | unrecognised
    confidence    REAL,                   -- when the source reports one
    digest        TEXT NOT NULL,          -- SHA-256 of the region image
    FOREIGN KEY (document) REFERENCES documents(path)
) STRICT;
```

The knowledge index (`crates/agent/knowledge/src/index.rs`) indexes three things from each row: the
LaTeX tokens, the Unicode fallback, and the surrounding sentence — so a search for `off-nadir` finds the
equation whose caption says off-nadir, and a search for `\frac` finds it too. The citation a hit prints
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
2. **The transcription table** for layer 1, so the segmentation is measured rather than claimed.
3. **OCR and the formula model** behind the two environment commands, with `source` and `confidence`
   recorded, and the `unrecognised` path tested — including that it never produces text it did not read.
4. **Agent and panel**: `knowledge.search` returns formulas as their own hits with the box citation, and
   a formula is quoted with its page so a person can check the pixels.
