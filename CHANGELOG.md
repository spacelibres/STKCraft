# Changelog

All notable changes to STKCraft are recorded here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Versions in the `0.0.x` series are early alpha. Each one states what it can and cannot do,
so a reader is never left guessing whether a missing capability is a bug or simply not built
yet.

## [0.1.49] — A formula records what it solves, and which mathematics it is

Two additions to `docs/design/mathematics.md`, both from the same objection: a LaTeX string on page 12
is not an answer.

**What it solves.** Every formula now carries `problem` — the sentence or caption that introduces it,
captured with the region. A formula without its sentence is a fragment; the sentence is what makes it
citable and what tells a person which question it answers.

**Which mathematics it is.** Formulas are classified against **Mathlib**, the largest machine-readable
body of mathematics under a licence this project can use. The integration is an *index*, not a
dependency: `cargo xtask mathlib` parses a pinned Mathlib checkout into one table of declarations — name,
kind, module path, statement, docstring, digest — and a deterministic classifier normalises the formula's
operators, scores candidates against that table, and records the match (`mathlib_name`,
`mathlib_score`, `mathlib_domain`, `classifier`). The module path gives the taxonomy — Analysis,
Algebra, Topology, Probability, Numerics — which is what makes a corpus *classifiable*: listed by domain,
searched within one, or asked "which of my papers are about this as a numerical problem".

The limits are written into the document because they decide whether the feature helps or lies:
**classification is not verification**. "This formula is about `Real.sqrt` with score 0.83, and the words
around it match that declaration's docstring" is the claim; "this paper proves the theorem" is not, and
the assistant is required to say it the first way. A wrong classification costs a bad tag; a fake proof
costs the project its reason to exist.

### Changed

- **`formulas` gains `problem`, `mathlib_name`, `mathlib_score`, `mathlib_domain`, `classifier`**, and
  the knowledge index now indexes the Mathlib name as well, so a search for `sqrt` finds the formula
  classified as `Real.sqrt`.
- **The transcription table records the problem sentence with the LaTeX**, so "what does it solve" is
  measured at the same time as "what does it say".

### Not built yet

- **The implementation**, in the document's order: segmentation and storage, the transcription table,
  OCR and the formula model, then the Mathlib index and classifier.

## [0.1.48] — Mathematics is a store, not a kind of prose

The requirement: formulas must be recognised well, and stored separately, because a misread formula
makes every number computed from it wrong and nobody notices. That is a design problem before it is a
model problem, so this release writes the design down as a decision record — `docs/design/mathematics.md`
— and puts it in the roadmap as the correctness floor ahead of sensors and targets.

The decisions it records:

- **A formula is never guessed.** Every stored formula carries what the recogniser produced (LaTeX where
  it exists, Unicode where only characters do), the page and **box**, the image of the region, the
  engine and version that produced it, and a digest of the pixels. A region that cannot be recognised is
  stored as `unrecognised` with its image — an invented formula is worse than a missing one, because a
  missing one says "look at page 12" and an invented one says "trust this number".
- **Why separate and not a paragraph.** Prose and mathematics fail differently (a wrong word is visible
  on reading; a moved subscript is not), are found by different means, and cite differently: a formula
  has no line to cite, it has a box.
- **Four layers, each skippable.** (1) deterministic segmentation from the text layer the interpreter
  already delivers — glyphs whose font, baseline or Unicode block say "mathematics"; (2) deterministic
  layout — centred or indented, spaced away from the body, holding an operator that cannot be prose;
  (3) an optional external formula recogniser (PNG in, LaTeX out) behind `STKCRAFT_FORMULA_RECOGNIZER`;
  (4) OCR for scanned pages behind `STKCRAFT_OCR`. Layers 3 and 4 are external for the same reason the
  model transport is `curl`: a model is configuration in this project, not something linked into the
  binary.
- **Where it lives.** One `STRICT` table, `formulas`, with the box, both renderings, the surrounding
  sentence, the source, the confidence and the digest. The knowledge index indexes the LaTeX tokens, the
  Unicode fallback *and* the surrounding sentence, so a search for `off-nadir` finds the equation whose
  caption says off-nadir. The citation is `paper.pdf:page 12, formula at (120,340)-(300,388)`.
- **"Recognised well" is measured, not asserted.** A transcription table of (document, page, box,
  expected LaTeX) rows taken from the 495-document CCSDS corpus and the SGP4 papers, where a mismatch
  fails the test. A recogniser that cannot pass it is not shipped as the default.

### Not built yet

- **The implementation**, in the order the document sets: segmentation and storage first (ours,
  deterministic), the transcription table next, then OCR and the formula model behind their two
  environment commands.

## [0.1.47] — Every CCSDS publication, hashed and searchable

The engine claims to implement CCSDS formats; the knowledge base could search four of the books. Now it
can search **all of them**: 495 documents, 548 MB, 393 468 passages, every one downloaded, hashed and
verified.

### Added

- **`cargo xtask standards`** — downloads the entire public CCSDS catalogue from
  `https://public.ccsds.org/Pubs/` into `data/standards/` (which `.gitignore` excludes), and verifies
  each file against the SHA-256 in `xtask/src/main.rs`. The table stores the name and the hash only, and
  the URL is derived: a table of five hundred near-identical URLs is a table where one typo is
  invisible.
- **`docs/guides/ccsds-standards.md`** — all 495 rows, plus what each family covers (1xx mission
  operations, 2xx onboard interface, 3xx cross support, 4xx space link, 5xx data exchange — the ODM,
  TDM, ADM and CDM this project parses — 6xx internetworking, 7xx system engineering, 9xx space traffic
  coordination), and where the alignment stops.
- **Verified twice**: `cargo xtask standards` re-hashed all 495 files it had just downloaded
  ("0 fetched, 495 already present") and the knowledge base then read the same files: **495 documents,
  393 468 passages**, 496 of them cited by page.

### Measured

- **cold open 66 s** (495 PDFs parsed and indexed), **warm open 1.6 s** — the interned index and the
  key-value cache from 0.1.35 doing exactly what they were built for at ten times the scale they were
  measured on.
- A search for `ORB_AVERAGING allowed values` lands on `922x27o1.pdf:page 41`, which is the SANA
  registry's "Allowed Values Column Symbols" — the document that *defines* the keyword the ODM book
  merely refers to.

### Not built yet — and now visible

- **Two documents extracted badly**: `101x0b1s.pdf` (41 pages, 2 passages) and `102x0b1s-1.pdf`
  (59 pages, 5 passages). Both are almost certainly image-only or built from fonts with no usable
  `ToUnicode` map, which is the OCR limit the changelog has carried since 0.1.38 — now measured on real
  files instead of asserted. They remain in the library, visible in the listing with their passage
  count, rather than silently dropped.
- **Alignment is partial and the doc says which parts**: 5xx is implemented; 1xx–4xx, 6xx and 7xx are
  collected and searchable, and nothing here implements them yet.

## [0.1.46] — Compaction that remembers, because the model writes down what it loses

F2 of the foundation. A long conversation used to be trimmed by dropping the oldest turns and leaving
a fixed sentence: *"N earlier messages were condensed"*. That is not memory, it is forgetting with a
comment — and a person who asked about a number ten turns ago got an assistant that no longer knew it
had ever been told.

Three changes, and the third is the one you notice:

- **The trigger is the request, not a count.** `Agent::context_characters` measures what the next call
  would carry — the messages *and* the tool schemas — against the budget. A conversation of forty tiny
  exchanges is not compacted; a conversation of four huge ones is.
- **The span is decided before it is removed.** `Session::plan_compaction` returns the messages a
  compaction would drop, so the summary is written about text that still exists. A summary asked for
  after the drop would describe a transcript nobody kept.
- **The model writes the summary.** `Message::Summary` is a new kind of message: it travels to the
  provider as a `system` message ("earlier in this conversation, in summary: …"), so it is context and
  cannot be mistaken for something the person said; it sits where the system prompt is and is **never
  cut**; and it is written into the transcript, so a resumed session starts with the same memory a
  live one has. If the summarising call fails, the compaction still happens and the replacement says
  the summary is not available — an honest gap, never an invented one.

The prompt asks for facts, numbers, names and decisions, not prose: the summary is the only record of
the turns it replaces, and a fluent paragraph that drops the number has lost the conversation.

### Added

- **Session metadata** — `created`, `updated`, `title` (the first question, truncated) and an optional
  scenario, written as a `meta` line after every message so the last one wins. `chat --sessions` can
  now show what a conversation was about instead of a timestamp.
- **`Session::plan_compaction` / `Session::apply_compaction`** — the span, the replacement, and the
  count, separately, because they are three decisions.

### Changed

- **A summary is pinned with the system prompt.** Compaction keeps `System` and `Summary` messages and
  drops only conversation, so a second compaction summarises the turns, not the memory of the first.

### Verified

`the_summary_replaces_the_span_and_keeps_the_facts_the_model_chose` builds a conversation larger than
the budget, plans the span, applies a summary containing a fact from it, and then **reopens the session
from disk** and finds the fact still there. The old behaviour would have failed that last step: the
fact was gone and the note did not contain it.

### Not built yet

- **The window still lists no sessions** (`chat --sessions` does): that is F4, with the rest of the
  agent surface.
- **A token-accurate budget** — characters are a good proxy and a poor measurement; the exact count
  wants a tokenizer per model.

## [0.1.45] — The order of work is the foundation, and the roadmap says so

0.1.40 wrote the roadmap and put sensors and targets at the top of it. That was the wrong order and
the correction matters more than the document: a sensor cone is a feature *of* the agent, the memory
and the extension runtime, and building it while those are still moving is how both end up half-done.

So `ROADMAP.md`'s *Next* is now a foundation sequence with a test for each step, and the sensor work
moved behind it:

- **F1 — the model layer.** Shipped in 0.1.44 (any OpenAI-compatible endpoint, credential stored
  `0600`, settings beat the environment).
- **F2 — memory and context at harness parity.** Next. The drop is triggered by the size of the
  request the model would receive rather than a message count, the dropped span is replaced by a
  summary the model wrote, pinned material is never cut, and the window lists and resumes sessions.
- **F3 — the extension runtime.** A manifest that validates is not a package that loads: an
  extension must be able to contribute commands, skills, a catalog and a knowledge root, and reach the
  same registry the menus use.
- **F4 — the window's agent surface.** Sessions, citations as places, a stop button, and settings
  reachable without a command line.
- **F5 — sensors and targets**, then coverage.

The *Where the project is* table gained the two foundation steps that shipped, so the roadmap and the
changelog tell the same story.

### Changed

- **Nothing executable.** This release is the correction of a plan plus the record of it; the code is
  0.1.44's.

### Not built yet

- Everything in F2–F5, which is the point of writing them down in order.

## [0.1.44] — Your key, in the app, for whichever endpoint you point it at

The foundation had one gap that made every other gap worse: the model was a single hard-wired client.
`DEEPSEEK_API_KEY` was read from the environment and nowhere else, which on macOS and Windows means a
person who launches the application from a dock **has no model at all** — and the whole point of this
platform is that the model drives the engine. This release turns the transport into configuration.

### Added

- **`Provider` replaces `DeepSeek`** in `stkcraft-agent-runtime`: an OpenAI-compatible endpoint, a model
  name, a credential and a timeout, and nothing else. Every provider that speaks that protocol is
  therefore supported without a rebuild — which is what "the model is configuration, not code" has to
  mean for a mission shop with its own gateway. The loop, the tools and the prompt are unchanged.
- **Settings that hold the model**: `ui.json` gains `model_endpoint` and `model`, and the credential
  goes in a **separate file, `model.key`, written `0600`** beside it. Not in `ui.json`: a file a person
  is likely to paste into a bug report is the worst place for a secret.
- **`stkcraft model`** — shows the endpoint and model in force and whether a credential is stored (never
  the credential), and changes them: `--key <KEY>`, `--endpoint <URL>`, `--name <MODEL>`, `--clear`.
  It also prints where both files live, because "where did it save it" is the next question.
- **Resolution order is a documented rule**: a stored credential beats the environment (a decision beats
  a convenience), the environment is the fallback so a container and a CI job keep working, and the
  shipped defaults fill in the rest. `DEEPSEEK_BASE_URL`, `DEEPSEEK_MODEL` and `DEEPSEEK_TIMEOUT_SECONDS`
  still override for a headless run.
- **The credential is redacted in `Debug`**: a provider prints `<15 bytes>`, never the value. A key in a
  log line is a key in a bug report.

### Fixed

- **A model request now has a configurable timeout**, and the refusal message says "the model endpoint
  refused the request" with the provider's own words instead of naming DeepSeek — a gateway that is not
  DeepSeek should not read as if it were.

### Verified

With **no `DEEPSEEK_API_KEY` in the environment**:

```text
$ stkcraft model --key sk-… --name deepseek-chat
STKCraft 0.1.44 — model deepseek-chat at https://api.deepseek.com/v1/chat/completions
  credential: present (13-byte stored or environment key, timeout 120 s)
  changed: model deepseek-chat, credential stored
  settings: /home/…/.config/stkcraft/ui.json
  credential: /home/…/.config/stkcraft/model.key        # -rw------- 13 bytes

$ env -u DEEPSEEK_API_KEY stkcraft ask "What is 2+2? Answer in three words."
Four, exactly, plainly.
```

A wrong stored key answers with the endpoint's own 401 text, which is the other half of the check: the
stored credential is what was used, and a failure explains itself.

### Not built yet

- **The window has no model section yet** — the settings exist and the CLI sets them; a person driving
  the assistant panel still needs the command line for now.
- **Memory, extensions and the window's agent surface** are the next three foundation steps, in that
  order (`ROADMAP.md`).

## [0.1.43] — The window answers the launcher, and hands over the picture

Since 0.1.36 the control channel existed, but only a CLI process could open it. The process that owns
the scenario a person is looking at is the *window*, and a launcher could drive a copy of the engine
that nobody was watching — or, more usefully, nothing at all. This release makes the window serve the
channel, which is what the family's other apps do and what `AGENTS.md` had been promising.

The protocol splits in two, and the split is the design:

- **the stateless half** — `version`, `engine.commands`, `knowledge.search`, `knowledge.documents` —
  is answered on the socket thread, which has its own session: a command list is a command list, and
  the person's documents are files on disk.
- **the half that reads or changes the window** — `engine.execute`, `scenario.read`, `ui.state`,
  `ui.screenshot` — is forwarded to the thread that owns it, once a frame. A launcher that renames the
  scenario and a person watching the screen are then looking at one scenario instead of two copies
  that drift apart.

`ui.state` answers with leaf fields — the scenario's name, what is in it, where the clock is, how many
frames have been painted, the interface language, the last status — so a launcher can ask "is the
thing I am driving the thing I think it is" and get an answer rather than a guess.

`ui.screenshot` asks egui for the frame it paints next, writes it to `screenshots/` under the data
directory, and returns the path, the size and the bytes in base64. **A capture that did not happen is
an error with the reason in it** — no painted frame, a window that is minimised — because a launcher
told "ok" with an image nobody drew is worse than one told why it waited. The request carries a
deadline for the same reason: a client has to be able to tell a slow window from a hung one.

### Added

- **`crates/agent/control::UiHost`** — the trait a window implements, and `WINDOW_METHODS`, the four
  names that are forwarded. `Control::with_ui` wires it in; without it (the CLI's `control`
  subcommand) the same methods are answered by the process's own session, so both paths stay working.
- **`apps/stkcraft/src/control.rs`** — the bridge: a channel of requests, each with its own reply
  sender, drained once a frame; a PNG encoder for an egui frame; and a hand-written base64 (§10 vectors
  in the tests) rather than a crate for forty lines of table lookup.
- **`stkcraft --control <port>`** — and `STKCRAFT_CONTROL_PORT` — opens the window and serves the
  channel. `stkcraft control --port <port>` still serves it with no window at all.
- **`stkcraft_astro_engine::args_from`** — the JSON-to-arguments conversion the MCP server had privately,
  now in the engine so every transport makes it identically: the kind a parameter declares decides the
  typed value a command receives.

### Verified

Against a live window on this machine, over a real socket:

```text
auth              → {"ok": true, "version": "0.1.43"}
ui.state          → scenario "Demo", 1 satellite, 1 station, 526 frames, language zh
engine.execute    → scenario renamed to `Gemini`
ui.state          → scenario "Gemini"          # the window's own scenario, not a copy
ui.screenshot     → /…/screenshots/window-….png, 1280×800, 3 073 098 bytes
engine.commands   → 10 tools
```

The captured PNG was decoded and inspected rather than trusted: 1280×800, 8-bit RGB, and a colour
histogram whose most common value is `#04060c` — the space background the viewport paints — with the
orbit and track colours present. A blank image would have had one colour.

### Not built yet

- **The screenshot is the whole window**, not a chosen region or a single panel: a launcher that wants
  the viewport alone will need a rectangle in the request.
- The roadmap's next step is **sensors and targets** — the access the product exists for.

## [0.1.42] — The platform is named like everything else, because a rule with a carve-out is not a rule

0.1.37 gave the workspace one naming convention and one exception: `crates/<group>/<leaf>` became
`stkcraft-<group>-<leaf>` for the domain, the agent and the UI — and `crates/framework/` stayed bare,
as `stkcraft-core`, `stkcraft-time` and so on, on the theory that the platform is the part a sibling
app shares and deserves a subject name rather than a group name.

That theory does not survive a reader. `stkcraft-time` and `stkcraft_astro_sgp4` are two conventions
in one workspace, and the one that is documented is the one nobody has to guess. Worse, the platform
is the layer an outsider is most likely to misread: `stkcraft-storage` could be a feature of the
astronomy, `stkcraft-framework-storage` cannot.

### Changed

- **The five platform crates are renamed with their group**: `stkcraft-framework-core`,
  `stkcraft-framework-time`, `stkcraft-framework-i18n`, `stkcraft-framework-storage` and
  `stkcraft-framework-extensions`. This is a breaking change for any downstream `use` line, and it is
  the last one of its kind: `cargo xtask layers` now derives every expected name from every path with
  no special case, so a new crate cannot reintroduce a second convention.
- **Every `use` line, manifest, document and generated API page follows**, including the layer diagram
  in `xtask/src/layers.rs` and the rule in `docs/standards.md` §7, which now says the longer name is
  the price of a rule with no carve-out.

### Not built yet

- **The window's control channel** — the roadmap's next step, unchanged by this rename — is still the
  next step.

## [0.1.41] — The gate was green here and red everywhere else, for eleven releases

Since 0.1.30 every push to `main` has failed CI, and every one of them passed `cargo xtask ci`
locally. The failing test is the one that asserts the font stack can draw every shipped catalog: it
walks the `zh` catalog, asks the stack which characters it cannot draw, and finds — on a bare Linux
runner — that it cannot draw any of them. Locally the machine has a CJK font installed, so the same
test found nothing missing and the gate passed.

The test was right and the environment was thin. `AGENTS.md` says the stack renders CJK "on any
machine that has a CJK font", and a CI runner is not such a machine: `craft-fonts` is checked out for
the build, but the shipped faces do not include Han — a Han face is tens of megabytes, and the
family's answer is to search the fonts that are installed instead. So the fix is to install the font
the catalog needs, in the workflow that runs the gate.

### Fixed

- **`.github/workflows/ci.yml` adds `fonts-noto-cjk` to the packages the gate installs**, with a
  comment saying why: it is not a build dependency, it is the face the assertion needs. Weakening the
  assertion instead would have made the check pass by not checking, which is the failure mode the
  test exists to prevent.
- **The eleven red runs from 0.1.30 to 0.1.40 were all this one test** —
  `fonts::tests::the_font_stack_can_draw_the_shipped_catalogs`, failing under
  `cargo test --workspace` in `stkcraft --bin stkcraft`. Nothing else was red and nothing those
  releases contained was wrong, which is why it read as the known upstream packaging lint and was
  left alone for eleven releases.

### Not built yet

- The roadmap's next step is unchanged: the window serves the control channel, with `ui.screenshot`.

## [0.1.40] — The order of work is written down

Forty releases in, the plan lived in one place only: the changelog, newest first, which says what
happened and not what is next. `AGENTS.md` has told every session to start by reading a `ROADMAP.md`
that did not exist. This release writes it.

`ROADMAP.md` states three things:

- **Where the project is**, as a table of areas and the releases that delivered them — time and SGP4
  against their published references, ephemerides, access geometry, the agent, i18n, the knowledge
  base, PDF reading, the control channel — so a reader can see the whole of it in one screen instead
  of reading 47 changelog sections.
- **What "done" means for a step**: the gate green, a numerical layer matching a published reference,
  and the release itself producing the evidence — a screenshot, a CLI transcript, a citation — rather
  than a changelog claiming it.
- **What is next, in order, with the reason it is next and the test it has to pass.** The window
  serving the control channel and `ui.screenshot`; sensors and targets, which is the access the
  product exists for; a timeline a person can read; recall beyond exact words over the corpus; numbers
  and direction in every language.

### Changed

- **`AGENTS.md` no longer says "once it exists"** about the roadmap, and its bulk-data rule is split
  in two: the ephemeris fetch and the paper corpus, because they are two tables and one rule.
- **`docs/README.md`** lists the roadmap first among the things to read.

### Not built yet

- Everything in the roadmap's *Next* section, which is the point of writing it down: the window's
  control channel and `ui.screenshot`, sensors and targets, the timeline, the vector recall layer, and
  number/date/RTL localization.

## [0.1.39] — A corpus to measure the reader on, and to search when nothing else is at hand

0.1.38 taught the knowledge base to read PDFs. It had nothing but hand-built fixtures to prove it
with — PDFs this repository wrote, which is a test of the assembly and not of the parser. Real
papers bring what fixtures cannot: two-column layouts, embedded fonts, Type 3 glyphs, tables and
running headers. So this release adds the documents, and the command that fetches them.

Six files, 8.8 MB, 580 pages:

- the four **CCSDS blue books** whose formats this project parses and emits — *Orbit Data Messages*
  (502.0-B-3), *Tracking Data Message* (503.0-B-2), *Attitude Data Messages* (504.0-B-2) and
  *Conjunction Data Messages* (508.0-B-1) — so a search over the corpus is a search over the
  documentation a person would actually consult;
- two openly licensed papers (**CC BY 4.0**) on the subjects the engine is built around: closing the
  gap between SGP4 and high-precision propagation, and how long a Starlink TLE can be trusted.

### Added

- **`cargo xtask papers`** — fetches the corpus into `data/papers/` (which `.gitignore` excludes),
  verifies each file against a recorded SHA-256, and prints the command that points a session at it.
  The same code as `cargo xtask fetch-data`: a document that is not the document the release
  documented is a citation nobody can check.
- **`docs/guides/paper-corpus.md`** — what each file is, where it comes from, its hash, its licence,
  and how to add one. Every hash was taken from a file that was downloaded, extracted and searched;
  none was guessed.
- **The arXiv entries pin a version** (`v5`, `v1`): a revised paper is a new row, not an edit,
  because an updated document would turn the recorded hash into a lie.

### Changed

- **`fetch_verified` is shared** by the two fetches instead of living inside the ephemeris task, so
  the checksum rule is the same code for both rather than two copies that can drift.

### Measured

On the corpus, with a cold cache: 580 pages extracted, 4 854 passages, and a search for `SGP4 mean
elements propagation accuracy` returning `ccsds-502.0-b-3-odm.pdf:page 65, lines 34-35` and a page of
the differentiable-propagation paper. A second open is **0.29 s** — the index comes from the
key-value cache, and no PDF is parsed at all.

### Not built yet

- **OCR.** The corpus contains no scanned document, because nothing here could read one.
- **A regression table over the corpus** — asserting that a known phrase lands on a known page of
  each file — is the natural next test, and it needs the fetched files, so it belongs to a run that
  has run `cargo xtask papers` rather than to `cargo xtask ci`.
- **The window does not open the control channel**, and `ui.screenshot` is not a method yet.

## [0.1.38] — The knowledge base reads papers, and cites the page

A paper is a PDF. Until now the library read text and nothing else, so the one kind of document a
mission engineer actually works from — the standard, the paper, the interface control document — had
to be converted by hand before it could be searched. This release opens PDFs, and it cites them the
way a person cites a paper: by page.

The reader is not new code. It is `craft-pdf`, the ArtCraft family's shared reader over the patched
`hayro` parser and interpreter, at the same `craft-libs` revision as `craft-ui` — so a PDF parses here
exactly as it does in `pdfcraft`, and a defect in parsing is one fix for the whole family rather than
six. What is new is the part a parser does not do: **placing text**.

A PDF has no lines. It has glyphs, each painted at a position with a font and a size, in the order the
generator wrote them — which for a two-column paper is column-interleaved. So the interpreter's device
is implemented, every glyph arrives with the matrix that put it there, and the assembly works from the
geometry:

- **lines** are runs of glyphs whose baselines agree to within a fraction of the type size;
- **word breaks** are the gap left over after the previous glyph's own advance, past a tenth of an em
  — measured against the font's advance, because an `M` advances three times as far as an `i` and
  comparing origins alone put a space before every wide letter;
- **paragraph breaks** are vertical gaps past the line pitch, which is what keeps a paragraph whole
  when the page is cut into passages;
- **columns** are read one at a time. Fragments are split at gaps wider than an em, and a page is
  ordered column-major when a vertical strip no fragment crosses divides it into groups that each hold
  a real share of it. A page with no such gutter — single column, or a table — is read line by line.
  Both directions have a test, because a page read in the wrong order is text no human wrote.

### Added

- **`stkcraft_agent_knowledge::pdf`** — the text extractor: pages in reading order, with the glyphs,
  the unnamed glyphs and the pages a bound cut short all reported. Tested against hand-built PDFs
  (a line, a paragraph gap, a positioned word gap, a two-column page, a page with no text layer, a
  truncated file) rather than a committed fixture nobody can read.
- **Page-level citations.** A hit carries the page it came from, and `Hit::cite` prints
  `paper.pdf:page 7, lines 3-9` — a place a person can turn to, which a line number in a PDF is not.
  The same page travels through the index cache, so a warm open cites it identically.
- **`Opened::skipped`** names the files in the knowledge directory that are *not* in the library and
  says why: a scan with no text layer, a PDF that will not open, a binary dressed as text. One
  unreadable paper does not make the other nine hundred unusable, and silence about it was the worse
  answer.
- **`stkcraft knowledge`** lists a paper with its page count, and prints the skipped files with their
  reasons.

### Changed

- **The library reads content, not extensions.** A PDF named `.txt` is read as a PDF and a text file
  named `.pdf` is refused as one, because the header is what a reader can trust.
- **A PDF has its own size bound** (64 MiB against 8 MiB for text): a scanned paper is tens of
  megabytes of images, and a text file of that size is not a document anybody reads.

### Fixed

- Nothing in this release changes a computed number. The engine is untouched.

### Not built yet

- **OCR.** A scanned page has text in its picture and none in its file; it is reported as a scan
  rather than guessed at.
- **The paper corpus fetcher** — pinning NASA NTRS and ESA documents with checksums into `data/` — is
  the next step, and now it has somewhere to put what it downloads.
- **The window does not open the control channel**, and `ui.screenshot` is not a method yet.

## [0.1.37] — A crate name you can read without opening the file

No behaviour changed in this release. Every crate in the workspace was renamed so that its package name
repeats the directory it lives in, and the gate now refuses a name that does not.

The problem was smaller than it sounds and worse than it looks. `crates/agent/runtime` was named
`stkcraft-agent`, so the directory and the name disagreed: a reader looking for "the runtime" found
nothing, and a reader looking for "the agent" found the runtime. Worse, the names carried no group at
all — `stkcraft-engine`, `stkcraft-access`, `stkcraft-viewport` said nothing about where they sat, and
`use stkcraft_engine::analysis` did not tell a professional reader whether that was the platform or the
astronomy. In a suite whose whole claim is that the engine, the agent and the UI are separate, the
names were the one place that separation was invisible.

### Changed

- **`crates/<group>/<leaf>` is now `stkcraft-<group>-<leaf>`**: `stkcraft-astro-body`,
  `stkcraft-astro-frames`, `stkcraft-astro-ephemeris`, `stkcraft-astro-sgp4`, `stkcraft-astro-access`,
  `stkcraft-astro-engine`, `stkcraft-agent-knowledge`, `stkcraft-agent-mcp`, `stkcraft-agent-runtime`,
  `stkcraft-agent-control`, `stkcraft-ui-viewport`.
- **The platform kept a bare name in this release.** `crates/framework/` stayed `stkcraft-core`,
  `stkcraft-time`, `stkcraft-i18n`, `stkcraft-storage` and `stkcraft-extensions`. *0.1.42 reversed
  that:* it is the one convention the workspace had that a reader could not predict from the path, and
  a rule with a carve-out is a rule two people read two ways. **This release's one breaking change** —
  `use stkcraft_agent::…` becomes `use stkcraft_agent_runtime::…`, and every other old name maps the
  same way.
- **`cargo xtask layers` enforces the naming rule**, not only the layering: the layer table pairs every
  package with its path, and a name that does not match what the path implies is a violation. The check
  is tested with a control case — the old `stkcraft-body` under `crates/astro/` fails it — so it cannot
  be satisfied by a rule that accepts everything.
- **`docs/standards.md` §7** states the four naming rules, including why the hyphen is the delimiter
  (`stkcraft-astro-frames` in the manifest, `stkcraft_astro_frames` in code) and that a group directory
  is never itself a package.

### Not built yet

- **PDF ingestion** and the **paper corpus fetcher** are still the next two steps.
- **The window does not open the control channel**, and `ui.screenshot` is not a method yet.

## [0.1.36] — The launcher's socket, sessions that outlive the process, and the machine-freeze bug

Three things this release, and the third is the one that mattered most.

### Fixed

- **A library read that took a machine down.** `new_token()` read `/dev/urandom` with
  `std::fs::read`, which reads *to the end of the file* — and a device file never ends. The buffer grew
  until the kernel OOM-killed the process at **130 GB RSS**, taking the desktop with it (`journalctl`
  shows the kill, and two other processes failing with `oom-kill`). It now reads a stated 32 bytes with
  `read_exact`, and `AGENTS.md` states the rule: a device file is never read to the end.
- **An empty library claimed a match.** `Library::default()` left the evidence threshold at `0.0`, so a
  search of no documents reported `missed: false` with zero hits and the tool told a model to "quote
  them". A derived `Default` and a threshold of zero is a search that always finds something.
- **`cargo xtask ci` now ends with the workflow's lockfile check**, done as a before/after digest: the
  gate must not re-resolve a dependency. Comparing against `HEAD` instead would fail on a dependency
  that is about to be committed, which is the normal case.

### Added

- **`crates/agent/control`** — the launcher-facing socket, the family's shape: `stkcraft control --port
  7979`, a token in `$XDG_RUNTIME_DIR/stkcraft/control-<port>.token` with mode `0600`, `auth` required
  before anything else, and a constant-time token comparison. Methods: `version`,
  `engine.commands`, `engine.execute`, `scenario.read`, `knowledge.search`, `knowledge.documents` — all
  of them through the MCP server, so the registry stays the single source of what can be done.
- **Sessions.** A conversation is a transcript on disk (`~/.local/share/stkcraft/sessions/<id>.jsonl`),
  one JSON object per message, appended as it happens: `chat` prints its id, `chat --sessions` lists
  them and `chat --session <id>` resumes one. When the conversation outgrows the context budget the
  oldest turns are dropped at a user message and the transcript gains a note, so the model knows it has
  a past it cannot see. This is the shape a harness keeps its own sessions in.

### Changed

- **`docs/standards.md` §7** describes `crates/agent/` as the *driving* surfaces — a model and a
  launcher are the same thing from the engine's side — and §6 records the control channel as built,
  with the window-side integration still open.

### Not built yet

- **The window does not open the control channel** and `ui.screenshot` is not a method, so a launcher
  can drive the engine but not photograph the window.
- **PDF ingestion** and the **paper corpus fetcher** are the next two steps.

## [0.1.35] — A paper corpus opens in half a second, because the index is interned and cached

This was supposed to be "put the knowledge index in the key-value cache". Measuring first said the
cache was not the problem: on a 27 MB corpus of 2 000 documents the library took 2.4 s to open and
**416 MB of memory**, and skipping the parse saved 0.4 s. The cost was the index itself — a `String`
per token and a map per passage — and no cache hides that.

So the index was rewritten before it was cached:

- **Tokens are interned.** Every distinct word becomes one `u32` in a dictionary; a passage stores
  term ids and counts instead of strings. Memory on that corpus fell from 416 MB to 173 MB cold, and
  the cold open from 2.4 s to 1.4 s.
- **The whole assembled library is cached**, documents and index, keyed by a manifest of every file's
  path, size and modification time. A re-open is one read: **1.43 s → 0.55 s**, 124 MB, and a search
  over 20 000 passages still answers with a citation (`paper-0812.md:lines 1-24`).
- **One library at a time in the cache.** A corpus is re-keyed whenever a file changes, so an edited
  library would otherwise leave the whole corpus behind on every save; the previous entry is pruned.

### Added

- **`stkcraft-agent-knowledge::index`** — the inverted index: a dictionary of interned terms, one posting per
  passage, document frequencies, and BM25 scoring by term id. It serialises and reads back with a
  bounds-checked reader, so a truncated cache file is a rebuild rather than a panic or a wrong answer.
- **`stkcraft_agent_knowledge::cache`** — the derived library in the key-value store, with the key changing
  when any file's metadata does, and the manifest kept so the cache holds one corpus rather than every
  revision of it.
- **`Library::open_cached`**, and `stkcraft knowledge` reports `parsed / reused` so a person can see
  whether the cache worked.

### Fixed

- **A hit could resolve to the wrong passage.** The index kept the passage's global position where it
  meant the passage's position within its document, so every document after the first resolved to
  nothing — a ranking bug that looked like "one document matched". Caught by a test that asserted the
  right document comes first, and now pinned by `owner()` returning the two indices separately.

### Not built yet

- **No embedding model and no filled vector index** — the store is ready; the model is a decision.
- **The control channel**, **PDF ingestion** and the **paper corpus fetcher** are the next three steps.

## [0.1.34] — Storage, and the provenance that makes a number checkable

The storage design was fixed last release; this one implements it in `crates/framework/storage`, and
gives it its first real consumer. `stkcraft passes` and `stkcraft planet` now record what they computed
— the command, its parameters, the engine version and **the SHA-256 of the inputs they ran on** — and
the access windows they found. `stkcraft history` reads them back. A number quoted from a report can be
traced to the run that produced it.

### Added

- **`Store`** — the relational record: SQLite with `STRICT` tables, WAL, foreign keys on, and
  forward-only migrations keyed on `PRAGMA user_version`. Scenarios and their append-only revisions,
  satellites and stations, computations, access windows, knowledge documents, citations, translation
  rows and installed packages; every table carries the type it means, so a `REAL` cannot quietly become
  `TEXT` and make an analysis unreproducible.
- **`Cache`** — one file per key, and the key is **never** a file name: the name is the key's SHA-256 and
  the key is stored in the header and compared on read, so `../../etc/passwd` is a hash and not a path.
  Every value carries a CRC-32; a truncated, flipped or foreign file is a *miss*, not an error and not a
  panic, because the premise of a cache is that the value can be recomputed. Writes go through a
  temporary file and a rename, so a crash leaves the old entry rather than a half-written one.
- **`Vectors`** — a set from one model, with the dimension checked on insert *and* on query: comparing a
  query against another model's embedding space produces a number that looks meaningful and is not, and
  that is the one mistake this store makes impossible. Non-finite components are refused. Brute-force
  cosine at this scale is milliseconds, and an approximate index is a change to the index, not the
  format.
- **`stkcraft history`**, and provenance recording in `passes` and `planet` — command, parameters, engine
  version, inputs digest, the windows' UTC instants, and how many windows the run stored.

### Changed

- **`docs/design/storage.md` §6** now says what is built and what is not: the record, the cache and the
  vector store exist; PostgreSQL, an embedding model, the knowledge index cache and the corpus fetcher
  do not.

### Not built yet

- **The knowledge index is not cached yet** — a library is re-tokenized at start. That is the next
  consumer and the precondition for a paper corpus.
- **No embedding model and no filled vector index**, for the reason above: the store is ready, the
  model that would fill it is a decision.
- **The control channel**, **PDF ingestion** and the **paper corpus** are still the next three steps.

## [0.1.33] — The repository root is a map, and content is not mixed with code

The crate tree was regrouped last release; the root was not. `skills/` and `locales/` — content the
*application loads at run time* — sat beside the source, `docs/` was a flat pile of six documents of
three different kinds, and there was no statement anywhere of what a top-level directory is for. A
professional reading the tree could not tell content from code from generated text from fetched data.

### Changed

- **Content moved to `share/`**, by kind: `share/skills/<name>/SKILL.md` and
  `share/locales/<locale>/stkcraft.tsv`. It is what the app loads, it is copied beside the binary in a
  deployment, and it is neither code nor licensed artwork. `build.rs` embeds the catalogs from there,
  and `share/skills` is found beside the executable and then in the working directory.
- **`docs/` grouped by kind**: `docs/design/` (architecture, storage, knowledge — decisions),
  `docs/guides/` (localization, ephemeris data — how-tos), `docs/api/` (generated), `docs/plans/`
  (dated history), with `docs/standards.md` staying at the top because it governs all of them.

### Added

- **`docs/standards.md` §9 — repository layout**: every top-level entry, what belongs there, what may
  not, and the three rules that keep the map honest (content is not scattered beside the source;
  generated and fetched things are never committed, except `docs/api/` which the gate checks; a
  document's home is chosen by kind, not by date).
- **`docs/README.md`** — an index of the documentation, so nobody has to guess where something is
  written down, plus where a new document of each kind belongs.
- **A "Repository layout" section in `README.md`** — the map a first-time reader sees.

### Not built yet

- **The storage crate**, the **control channel**, **PDF ingestion** and the **paper corpus** remain the
  next four steps, in that order (the order is in `CHANGELOG.md` 0.1.32 and in §8).

## [0.1.32] — The tree says what each crate is, and the storage design is decided

Two things a professional looks at first: how the crates are organised, and where the data lives.
Both were decided by accretion until now — a `crates/` directory in the order things happened to be
written, and state in whatever file was nearest. This release fixes the shape of both, on paper and in
the tree, before more code makes the wrong shape permanent.

### Changed

- **`crates/` is grouped by subject.** `crates/framework/` is the platform — units, time, translations,
  the extension contract — and nothing in it may know what an orbit is. `crates/astro/` is the domain:
  bodies, frames, ephemerides, propagation, access geometry, the engine. `crates/agent/` is the
  model-facing surface: the knowledge base, the MCP server, the runtime. `crates/ui/` is presentation.
  A reader can now tell which is which without opening a file, and a crate that needs a higher layer
  than its directory implies is in the wrong directory.
- **Crate names keep the `stkcraft-` prefix**, so `use stkcraft_astro_body::Body` is unchanged and the family's
  one-prefix convention holds; it is the *directory* that carries the taxonomy.

### Added

- **`docs/standards.md` §7 (crate taxonomy)** — which directory means what, with the rule that a new
  crate goes where its subject puts it and `cargo xtask layers` is what enforces membership.
- **`docs/standards.md` §8 and `docs/storage.md`** — the storage decision record: the **relational**
  store holds the record (scenarios and their revisions, satellites, stations, sensors, access windows,
  the provenance of every run, documents and citations, translation rows, installed packages); the
  **key-value** store holds derived, disposable caches (kernel indexes, the knowledge index, vectors,
  generated catalogs); the **vector** store holds recall candidates once an embedding model exists and
  never the authority. One crate will own persistence; nothing else may open a database.

### Not built yet

- **The storage crate itself** — this release fixes the design, and the schema lands next with the
  SQLite record, the key-value cache and the vector table.
- **The control channel** the siblings expose, **PDF ingestion** for the knowledge base, and the
  **paper corpus** fetcher are the next three steps, in that order.

## [0.1.31] — The user's documents are the expert panel

A model asked "what is the maximum off-nadir angle for this sensor" answers from its training data,
which may be wrong, may be about another spacecraft, and cannot be checked. STKCraft now reads the
person's own documents **before** the model speaks: drop requirements, data sheets and notes into one
folder, and every factual answer quotes a passage and cites `path:lines a-b` so a reader can open the
file and check it.

That is the difference between an assistant and a source of record, and it is the rule the platform is
built around now — not a feature beside the model but the order in which it is asked.

### Added

- **`stkcraft-agent-knowledge`** — a directory of documents, chunked by paragraph with line ranges kept, and
  BM25 search over it. Deterministic, local, no embeddings and no network: the same library and query
  rank the same way on every machine, which is what lets a test assert that a specification value is
  found and a guess is not.
- **`knowledge.search` and `knowledge.documents`** — MCP tools on the server, so an MCP client and the
  built-in agent see the same library. A search returns the passages, their citations and their
  scores; when nothing clears the evidence floor it says so in words, and the tool result includes
  `structuredContent` with `missed` for a program.
- **`stkcraft knowledge [<query>]`** — list the library or search it from the terminal, printing the
  same citations the model sees.
- **`STKCRAFT_KNOWLEDGE`** and a default of `<config>/knowledge/`, searched by `ask`, `chat` and `mcp`
  alike, because they build the same session.
- **A `knowledge-base` skill** — the procedure as a document: search before answering, quote, cite,
  say when the library is silent, and never re-derive a document's number with the engine.
- **`docs/standards.md` §2a**: the rule, stated as a product rule rather than a feature.
- **`LICENSE-APACHE`** — the dual licence the manifest and `NOTICE` already declared but the
  repository did not carry. §6 of the rule book now records licensing and family interop, including
  the one interop item not built: the siblings' control channel.

### Verified against the real model

```text
> What is the maximum off-nadir angle and the swath width for our sensor? Cite the document.
> "Maximum off-nadir angle: 45 degrees. Swath width at that angle: 1200 km."
  — sensor-spec.md:lines 1-5

> What does our knowledge base say the maximum reaction-wheel torque is?
  knowledge.search → No passage matches … Say that the knowledge base does not cover this
> The knowledge base does not cover this. … I won't substitute one from memory as though it were
  theirs. If you have a spacecraft bus or AOCS data sheet, adding it to the knowledge base would let
  me answer this properly.
```

The second answer is the one that matters: a platform that can say "my library does not cover this"
is a platform whose numbers mean something.

### Not built yet

- **No PDF, DOCX or XLSX reading** — a citation into a PDF is a page, not a line. Extract the text
  beside the file (`pdftotext spec.pdf spec.txt`) and it is indexed; `docs/knowledge.md` states it.
- **No index cache**: a large library re-tokenizes at start.
- **No side-by-side comparison** of a document's answer with the engine's computation — the "expert
  panel" idea taken literally is the next step, not this one.
- **The control channel** (`--control <port>`, token file) that the siblings expose is still missing.

## [0.1.30] — The interface is drawn by the family's stack, and Chinese looks like Chinese

A translated interface was tofu boxes. `egui` ships a Latin face and nothing else, so every Chinese,
Japanese, Korean, Arabic and Thai label rendered as `▯▯▯▯` — the catalog was right and the pixels were
wrong. The window now installs the ArtCraft family's font stack, which searches the fonts installed
on the machine, and the same stack is what every sibling uses.

The other half of this release is the unification it took to get there: `egui`, `eframe` and `winit`
now come from the pinned `storytold/egui` fork that every Craft app builds against, and the shared UI
library is `craft-ui` from `storytold/craft-libs` at the one revision the family pins.

### Added

- **`craft_ui::fonts::UiFonts` installs the interface fonts**: the app's faces, the faces for the
  interface language and the other scripts, then the machine's installed fonts, then emoji and
  `egui`'s defaults. STKCraft supplies what the library asks an app for — the scripts its languages
  are written in, and which language is on screen — and nothing else.
- **A coverage test over every shipped catalog**, using the family's `missing_glyphs`: a character
  the fonts cannot draw fails the build with its codepoint and the string it came from, instead of
  shipping as a box.
- **`STKCRAFT_FONTS=off`**, the family's reproducible-text mode: the same frame on every machine, and
  the fastest way to tell a glyph problem from a real one.

### Changed

- **`[patch.crates-io]` pins `storytold/egui`** (with `storytold/winit`) at the revision the family
  documents, so a widget behaves here as it does in the siblings and a fix lands once.
- **`craft-ui` replaces the local furniture** the window had started to grow. `docs/standards.md`
  gains §5, which states the rule: the window uses the family's stack, not a local arrangement of
  `egui`.

### Fixed

- **Translated text renders.** Verified rather than asserted: the same frame rendered with
  `STKCRAFT_FONTS=off` shows hollow `▯` boxes and with the stack shows the Han characters, and the
  scene images are byte-identical to the ones from before the fork swap — so the swap changed the
  text and nothing else.

### Not built yet

- **`craft-ui`'s theme tokens and widgets are adopted for text only**; the panels still draw their
  own layout and colours, which is the next step of the unification rather than part of this one.
- **No font file is committed**, as the family requires: fonts come from the machine or from
  `storytold/craft-fonts` as an optional build input.
- **Number and date localization, right-to-left layout and the extension store** are unchanged.

## [0.1.29] — Your key, your language, in the window

Until now a translation was a developer's job: `cargo xtask i18n translate`, in a checkout, with
cargo. A person running the released binary had no such path, and the language they wanted might not
ship at all. Now the panel offers it: pick a language the build does not carry, press **Translate
{n} strings with my key**, and `DeepSeek` — on `DEEPSEEK_API_KEY` — fills it in, validates every
proposal with the same rules the gate applies, and the interface changes language in place.

### Added

- **Self-translation in the window.** The catalog goes to `locales/<code>.tsv` in the *person's*
  configuration directory, never into the checkout, so an unreviewed machine translation cannot
  become the project's. It reloads the interface without a restart, and a second run asks only about
  strings that are new.
- **The interface's own string list, embedded at build time.** A window that translates itself has to
  know what to translate; `build.rs` extracts the strings from this crate's sources with the same
  scanner `cargo xtask i18n` uses, so a self-translation and a translator's catalog describe the same
  interface.
- **`stkcraft_agent_runtime::translate`** — the batch, the prompt and the acceptance rules, once, for both
  callers: the developer task and the window. Two implementations would be two prompts and two
  acceptance rules, and the one that drifts is the one a user meets.
- **`stkcraft_framework_i18n::extract`** — the macro scanner, moved out of `xtask` so the build script, the
  developer task and their tests share one answer to "which strings exist".

### Fixed

- **The scanner now reads code the way Rust does.** Three defects came out of using it on real
  sources: it stepped by byte and panicked in the middle of a non-ASCII character; it treated
  `("tr!(", None)` — the text it matches on, in its own source — as a call and put a row named
  `, Some(|)` into a real catalog; and it ended a raw string at the first backslash-quote, so a test
  that asserts on the extractor appeared in the catalog as `Yes"`. It now walks characters, skips
  strings, raw strings and both comment forms, and all three are pinned by tests.

### Changed

- **The plural check counts the locale's CLDR forms** wherever a row is validated — the developer task
  previously validated proposals as if they were English, which refused every correct CJK plural.

### Not built yet

- **One shipped catalog (Chinese)**; everything else is a person's own translation until someone
  reviews one into the repository.
- **No number or date localization**, no right-to-left layout.
- **The extension store and per-item API signatures** are unchanged.

## [0.1.28] — A real model, a real language, and the three defects that only they found

The assistant now works against the real `DeepSeek` API, and the first translated interface ships.
Getting there found three defects that no scripted test could have found, each now pinned.

**The one that mattered most:** every command id is `namespace.verb`, and an OpenAI-compatible
function name must match `^[a-zA-Z0-9_-]+$`, so the provider refused *every* request with
`400 Invalid 'tools[0].function.name'` before generating a token. The agent now maps ids at its own
boundary — `access.compute` → `access__compute`, a double underscore because an id may itself contain
a hyphen — and the registry stays the single source of truth. A dot is legal in MCP and not in a
function name, so the two vocabularies meet in exactly one place.

**The one that hid the first:** the transport ran `curl --fail`, which throws the response body away
with the status. A 400 arrived as an empty message. It is now `--fail-with-body` and the provider's
own words are reported, in the assistant and in the translation tasks alike.

**The one a Chinese catalog exposed:** a plural row was refused for listing one form where English
lists two. Chinese has exactly one CLDR form; the check now counts the *locale's* forms. A second,
quieter one came with it: a Rust line continuation inside a literal was extracted as a newline plus
indentation, putting a hole in a translated sentence.

### Added

- **`scenario.read`**, the agent's tool for the document it is asked about. Asked "how many
  satellites are there?", a model with only add/remove tools answered that it could not know — honest
  and useless. The system prompt now says to call it rather than guess a count.
- **`locales/zh/stkcraft.tsv`** — the first shipped catalog, generated by `cargo xtask i18n translate`
  with the maintainer's key and reviewed row by row. On a Chinese system the window now opens in
  Chinese: `version` → 版本, `Compute passes` → 计算过境, `{n} satellite|{n} satellites` → `{n} 颗卫星`.

### Fixed

- **Function names are translated at the agent boundary** — see above. Three tests pin it: every
  registry id maps to a legal, unique function name; a call named the way the model sees it reaches
  the command; and the schemas a model receives contain no dot.
- **The provider's error body reaches the message**, so a refusal says what it refused.
- **A plural row is validated against its own locale's CLDR forms**, not English's, and a Rust line
  continuation is no longer a newline in an extracted string.

### Changed

- **The interface picks its language from the shipped catalogs**: an explicit choice, then the system
  locale, then English — so a fresh Chinese install is Chinese without anyone opening a menu.

### Not built yet

- **One translated language.** `cargo xtask i18n translate --language <code>` makes the next one; the
  review is still a person's job.
- **No number or date localization** and no right-to-left layout.
- **The extension store, the API reference's per-item signatures, and runtime translation** are
  unchanged from 0.1.27.

## [0.1.27] — The extension contract, with the door bolted shut

STKCraft now has a package contract, in the shape the ArtCraft family specified: a manifest, ids that
are safe as directory names, themes that carry no code, and trust tiers the *host* assigns. It is
deliberately the half that cannot hurt anyone — pure data and validation, no unpacking, no store, no
code execution — and `stkcraft extensions` reports what is installed and why anything was refused.

Third-party packages are designed in and switched off. The `third-party` cargo feature exists so the
code path is written now and enabled later, no release build enables it, and a test fails if it ever
becomes a default: opening the door is a decision after the family's threat-model gates and an outside
review, not a side effect of a build flag.

### Added

- **`stkcraft-framework-extensions`** — manifest parsing with `deny_unknown_fields`, ids checked by character
  class (lowercase, dot-separated, no `..`, no `/`, at most 128 bytes, because an id becomes a path),
  version and name validation, `Kind::{Theme, Skills, Plugin}` and `TrustTier::{Internal,
  ThirdParty}`.
- **Refusals that say what to fix**: a manifest that states its own trust level, a theme that names an
  entry point, a permission list that is not empty, a package for another app, and a plug-in
  anywhere — version 1 has no capability model, so a plug-in that ran would be one that could do
  anything.
- **`stkcraft extensions`** — reads `extensions/<id>/extension.toml` from the configuration directory
  and prints each package as admitted or refused with the reason. A bad package is reported and does
  not hide the good ones.
- **The rule book's §3 now describes shipped behaviour** for the contract, and still marks the store,
  the index, signatures and code execution as unbuilt.

### Changed

- **`docs/standards.md`'s conformance table** says what enforces each row: `cargo test` for the
  feature and trust rules, `cargo xtask i18n`, `cargo xtask api-docs check`.

### Not built yet

- **No store, no installer, no unpacker, no signatures, and no third-party path.** A package on disk
  is data; nothing loads it.
- **A plug-in cannot be installed** — it is refused by name, which is the honest answer while the
  permission model does not exist.
- **The interface is still English-only in a fresh build**: no catalog ships, because a translation is
  a thing a person has to read.

## [0.1.26] — The API reference is generated, and it can be read in your language

`docs/api/en/index.md` is now built from the code — the command registry, the crate manifests and
each crate's module documentation — and `cargo xtask api-docs translate --language <code>` produces
the same document in another language with `DeepSeek` and the reader's own key. One English reference
is maintained; every other language is generated from it and says which English revision it came
from, so a page that lags behind the code is visible rather than trusted.

### Added

- **`cargo xtask api-docs`** — generates the reference: every command with its id, label, menu path
  and typed parameters (`number`, `text`, `degrees`, and whether it is required), every crate with
  its layer and description, and each crate's module documentation as prose.
- **`cargo xtask api-docs check`**, a step of `cargo xtask ci` — fails when the committed reference
  is stale. The gate checks and never writes: a check that rewrites the file it validates passes on
  its first run and hides a stale reference for ever after.
- **`cargo xtask api-docs translate --language <code>`** — translates the prose of the reference
  (headings, paragraphs, labels, menu names) and leaves ids, numbers and code alone; refuses a
  translation whose placeholders do not match; remembers what it translated in
  `docs/api/<code>/strings.tsv` so the second run only sends what changed, and names the English
  version it was made from.
- **`xtask/src/translate.rs`** — the one model call both translation tasks use. Moving it out of the
  interface-catalog task was the point: two copies of a request would drift, and a translation
  pipeline that behaves differently for documentation than for the interface is a second thing to
  debug.

### Changed

- **A command's parameters render as documentation, not as a broken code span.** The cell holds ids
  and prose together, so wrapping it in backticks produced `` `name` (text): … ``` — a Markdown
  reader sees an empty code block. It is now pre-formatted Markdown that is neither wrapped nor
  translated.

### Not built yet

- **The reference lists the command surface, the crates and their module documentation**; it does not
  yet document every public item's signature.
- **Command parameter descriptions stay English in the reference.** The same text is translated in
  the interface, where the catalogs carry it; the reference will use that translation once the
  registry exposes parameter text as catalog keys.
- **§3 of the rule book, extensions and plug-ins, is still rules without an implementation** — the
  package manifest, trust tiers and the third-party feature gate remain the next arc.

## [0.1.25] — One English interface, every language a catalog

The interface is now translatable, in the ArtCraft family's format: `context <TAB> English <TAB>
translation` catalogs keyed by the English source text, `storytold/craft-i18n`'s rules, and a
`cargo xtask i18n` task that checks them, reports what each language lacks, and fills the gaps with
`DeepSeek` **using the reader's own API key**. There is one English source and no English catalog —
the text in the code is the English interface — so a language that is 10% translated is 10% useful
and the rest shows English rather than a placeholder.

This release also lands the rule book: `docs/standards.md` states what the product must be where the
family has a standard — translations, agent/skills/MCP, extensions and plug-ins, API documentation —
and marks every rule that describes something not yet built.

### Added

- **`stkcraft-framework-i18n`** — the catalog format and its lookups: parse with per-line errors, the
  placeholder and ellipsis checks the shared project enforces, `@plural` rows with CLDR cardinal
  forms (1 form for Chinese and Japanese, 3 for Russian, Czech, Polish and Romanian, 6 for Arabic),
  `@id language.name` for the picker, region → base language → English fallback, and no failure
  mode: an unknown locale or an untranslated string is the English.
- **`cargo xtask i18n`**, `i18n report`, `i18n translate --language <code>` — the check is a step of
  `cargo xtask ci`; the report is a to-do list of what each language lacks; the translate command
  sends the missing rows to `DeepSeek` in one batch, refuses any row that breaks the format (a
  dropped or invented placeholder, a missing ellipsis, an empty translation), and writes a catalog
  that a person reviews before it ships. `--dry-run` shows the proposals without writing.
- **A language picker in the window** and `ui.json` in the platform's configuration directory, with
  the same first-run rule the family uses: an explicit choice, else the system locale, else English.
  A catalog this person generated is read from `locales/<locale>.tsv` beside `ui.json`, so a
  language works without rebuilding.
- **`docs/standards.md`** — the rule book, with a table of which family repository owns each
  standard and which task enforces it here.
- **`docs/localization.md`** — the catalog format, the commands, adding a language, and an honest
  list of what is not translated yet.

### Changed

- **Every interface string in the window goes through `tr!`, `trc!` or `trn!`**, which is what makes
  it extractable: `cargo xtask i18n` reads those literals out of the source and a string built at run
  time is invisible to it by construction. The extractor strips line comments so a documentation
  example is not mistaken for a call — a real trap: the crate's own docs show `tr!(…)` usage.

### Not built yet

- **No catalog ships with this release.** A catalog is a translation someone has to read, and machine
  output is a proposal: `cargo xtask i18n translate --language de` with your key produces one, and
  the search path for it is documented.
- **No number or date localization**, no right-to-left layout, and no translation at run time inside
  the window — the last one deliberately, because a window that translates itself on start is a
  window that makes a network call on start.
- **§3 of the rule book, extensions and plug-ins, is rules without an implementation**: the package
  manifest, the trust tiers and the third-party feature gate are the next arc, and until they exist
  this project runs no third-party code.

## [0.1.24] — A way out of a conversation

A conversation that has gone wrong — a wrong assumption the model keeps building on, a scenario
edited into nonsense — used to leave one remedy: close the window and start again. The assistant
panel now has **New conversation**: the discussion is forgotten, the scenario is kept. A discussion
that went wrong is not a document that went wrong, and a button that threw away the satellites as
well as the argument is a button nobody dares press.

### Added

- **New conversation** in the assistant panel, which empties the trace, the answer and the count,
  and tells the worker to forget — so the next question arrives in a session that knows the
  document and nothing about the argument.
- **`Responder::forget`**, defaulting to nothing, so the panel can ask a session to start over
  without knowing whether the responder keeps a conversation at all.

### Changed

- **The worker takes commands rather than jobs.** Asking and forgetting are the same message type,
  so the thread that owns the conversation is the only thing that has to know what forgetting
  means.

### Not built yet

- **The conversation is not saved**, and closing the window ends it.
- **No streaming of the model's prose**; the tool-call trace streams, the answer arrives whole.
- **`resources/updated` notifications, `stkcraft mcp` permission prompts and dynamic third-party
  plugins** are unchanged.

## [0.1.23] — One conversation, not one question

The agent now remembers. `stkcraft chat` asks questions one after another in a single session, and
the window's assistant panel keeps the same conversation across questions, so "and the day after?"
is answerable without restating the scenario the previous answer built. The first version of both
started from nothing every time: every question had to re-establish what was already known, and a
follow-up was a new problem rather than the next step of one.

### Added

- **`stkcraft chat`** — read a question, answer it, read the next, until `exit`, `quit` or end of
  input. One agent for the session, so the conversation and the scenario persist; a failed question
  does not end it, because the next question is usually the correction.
- **A conversation in `stkcraft-agent-runtime`** — messages are kept between questions, with the system
  prompt written once. `Agent::history_len` reports the length and `Agent::forget` empties the
  conversation while keeping the system prompt: the scenario lives in the server, so forgetting a
  discussion does not forget what the tools built.
- **`MAX_HISTORY`**, forty messages, trimmed at a *turn* boundary — never in the middle of a tool
  exchange, because a tool result whose call has been dropped is a request the model API rejects,
  and the rejection arrives long after the cause.
- **`Server::set_scenario`**, so a session can be handed the current document. The assistant worker
  takes the window's scenario with every question, which is what keeps an answer about the file the
  person has open.

### Changed

- **The assistant panel runs one worker for the session**, not one per question: the conversation
  lives on that thread, and a thread per question would throw it away between answers. The panel
  shows how many answers the conversation has produced, and a failed turn rebuilds the responder so
  the next question starts from a consistent history.

### Fixed

- **A failed question left the conversation in a state the next question could not use.** The
  responder is now rebuilt after a failure rather than reused with a half-finished turn inside it.

### Not built yet

- **No streaming of the model's prose**: the tool-call trace streams, the answer arrives whole.
- **The conversation is not saved**: closing the window or ending `chat` ends it.
- **`resources/updated` notifications, `stkcraft mcp` permission prompts and dynamic third-party
  plugins** are unchanged.

## [0.1.22] — A tool returns the answer, not a count

`access.compute` used to answer "20 window(s) over 4 pair(s)" and stop. The times — the thing the
question was about — stayed inside the process, so every caller had to recompute them or go without,
and a model could only paraphrase a count. Now a command that computes times returns them: as lines a
model reads, and as the same list in `structuredContent` for a program.

### Added

- **Pass instants in tool results.** `access.compute` lists each window as
  `2024-01-01 09:12:04 → 2024-01-01 09:18:16 UTC (6m 12s)`, and `planet.access` lists each pass the
  same way. Both are also in `structuredContent` — `start_utc`, `end_utc`, `start_seconds` from the
  scenario epoch, and `duration_seconds` — so a client can plot them without parsing prose. Long
  lists are summarised after twenty lines rather than truncated silently.
- **`stkcraft planet` prints the passes**, not only the summary, because the times are the answer.
- **`ParameterKind`** on every command parameter: `Number`, `Text` or `Degrees`. The MCP schema and
  the conversion of a caller's argument both come from it, and `tools/list` now says which
  parameters are text instead of guessing from their names.

### Fixed

- **`station.add` through MCP could not work at all.** The transport turned every JSON number into a
  plain number, but the command expects typed angles, so a model that asked to put a station at
  `{"latitude": 51.5}` was told "expected an angle" — with the right number in the right unit. The
  parameter kind is now declared by the command and applied by the transport, which is the only
  arrangement where the schema a model reads and the value the command receives agree.
- **The `compute-passes` skill named parameters that do not exist** (`latitude-deg`, `longitude-deg`,
  `mask-deg` on `station.add`, which takes `latitude`, `longitude`, `minimum-elevation`). A skill that
  sends a model into an error loop is worse than no skill: the procedure now names what the command
  actually takes, and says what the answer looks like.

### Changed

- **`planet.access` takes typed angles** rather than numbers in degrees, which is what the rest of
  the engine and the `AGENTS.md` rule about units already required.

### Not built yet

- **No conversation history** in the agent or the panel, and no streaming of the model's prose.
- **`resources/updated` notifications and dynamic third-party plugins** are unchanged.

## [0.1.21] — The assistant panel, and a screenshot that proves panels draw

The model reaches the window. A checkbox opens an **Assistant** panel: type a question, and the
engine's own commands run on a worker thread while the panel lists each call as it happens, then
shows the answer and adopts the scenario the model produced. It is the same loop as `stkcraft ask`,
so nothing new can happen in the panel that a command line could not already do.

The second half of this release is that the panels are now *tested*. Until now nothing drew them:
both screenshot paths rendered the scene and skipped the interface around it, so a panel could lay
out at zero height, clip its labels or swallow its own resize handle and every gate would still be
green.

### Added

- **The assistant panel** in the desktop app, closed by default (a window that opened with a model
  in it would make a network call on start). It runs the question on a thread of its own — a
  question is a network round trip around real engine calls, and a frozen clock is worse than a
  failure — and reports each `name(args) → summary` line through a channel the frame drains without
  blocking, so the trace appears while the work happens rather than after it.
- **`STKCRAFT_SCREENSHOT_PANELS`**, the third offscreen path: it draws the left panel and the
  assistant panel and checks the result numerically — each side must have content that is neither
  space nor the panel's flat fill, and enough label pixels for its text — with a blank image as the
  control that the check can fail at all.
- **`Agent::ask_observing`**, which reports each tool call as it happens; `ask` calls it with an
  observer that ignores the lines, so a caller that shows progress and a caller that does not share
  one loop. `stkcraft ask` prints the lines as they arrive for the same reason.

### Changed

- **The window adopts what the model did.** The worker answers against a copy of the scenario, and
  when the loop finishes the window takes that copy and rebuilds its frame — so "add a satellite
  called Kepler" leaves a satellite on screen, and a question still running cannot half-edit the
  document while a person keeps working.
- **`App::panels` is one function** for the left panel, the assistant panel and the adoption, shared
  by the window and the verification path, so the image cannot show a layout the window does not
  have.

### Fixed

- **The panels had never been rendered by anything.** Adding the assistant panel exposed why: the
  left panel's frame was being drawn by a call the screenshot paths did not make, so the interface
  around the scene had no evidence at all. The panel image is that evidence now, and the first run
  of it failed — the left panel drew zero lit pixels, because the panel wrapper had been dropped in
  the refactor that added the second panel.

### Not built yet

- **No streaming of the model's prose**: the trace streams, the answer arrives whole.
- **No conversation history**: each question starts from a fresh context, so a follow-up question
  cannot refer to the previous answer. That needs a `messages` list the panel owns and a decision
  about how much of it to send.
- **`resources/updated` notifications and dynamic third-party plugins** are unchanged.

## [0.1.20] — The model drives the engine

`stkcraft ask "<question>"` puts a model in front of the tools 0.1.19 published. `DeepSeek` reads
the question, calls the engine's own commands, sees what they returned, and answers from those
numbers — the loop that makes the MCP surface worth having rather than a second way to write a
command line. The split is unchanged and deliberate: the kernel gained no model, no prompt and no
network call, and `stkcraft mcp` and `stkcraft ask` are two clients of one protocol.

### Added

- **`stkcraft-agent-runtime`** — a chat-completions client and the loop over it: system prompt, tool
  schema from `Server::tools_json`, tool calls executed through `Server::handle`, and the result
  fed back until the model answers or [`MAX_TURNS`] is reached. `ChatModel` is a trait so the loop
  is tested with a scripted model — ten tests cover a plain answer, a tool call whose result comes
  back, an unknown tool, a failed command, the skills listed in the prompt, and an empty script
  that must end the loop rather than spin.
- **`skill.read`** — the one tool the agent adds to the server's set. A system prompt that carried
  every skill would be long, expensive and mostly irrelevant; the model is told which skills exist
  and reads one when a job needs it, which is the same progressive disclosure an MCP client gets
  from `prompts/get`.
- **`stkcraft ask "<question>"`**, printing every tool call as it happens — `name(args) → summary`
  — because an analysis whose steps a person cannot see is an analysis nobody can check. The model
  is configured by `DEEPSEEK_API_KEY`, with `DEEPSEEK_BASE_URL` and `DEEPSEEK_MODEL` overridable;
  without a key the message says so instead of failing inside a request.
- **`docs/architecture.md` gains "The agent-first core"**: why the model is outside the kernel,
  what the two layers above the engine are for, and what "plugin" means today (a command is one
  registry entry; a skill is a document; a bulk file is one verified row) and what a dynamic plugin
  registry would still need — a capability story, before any third-party code runs.

### Changed

- **The layer table grows to seven**, and the agent sits above the MCP server rather than beside
  it: the server is a transport onto the engine and knows nothing about models, the agent is a
  client of that transport, and the ordering now says so. A layering rule that allowed them to sit
  at the same height would have let the agent reach past the protocol.
- **`AGENTS.md` gains the agent rules**: the kernel stays free of prompts and network calls, an
  agent capability is added by adding a command, and a skill is a document with one heading and a
  description rather than code.
- **`planet.access`'s `body` description** loses the run of spaces it shipped with, so the schema a
  model reads is the sentence a person wrote.

### Fixed

- **0.1.19's changelog claimed a `docs/architecture.md` section that its commit did not contain.**
  The section is written here and the claim is removed from 0.1.19, because a changelog that
  describes documentation nobody wrote is the one thing a changelog must never do.

### Not built yet

- **The desktop app still has no agent panel.** `stkcraft ask` is the CLI client; the window's own
  channel, tool-call cards and a view of the trace are not built.
- **`resources/updated` notifications and subscribed resources** are not sent: a client reads the
  scenario once. Dynamic third-party plugins are unchanged and still need a capability story.

## [0.1.19] — The core is agent-first: MCP tools and skills

`stkcraft mcp` serves the engine to a large language model over the Model Context Protocol.
Every command is a tool with a schema built from its own parameter documentation, the scenario and
the command catalogue are resources, and the procedures a model needs are markdown skills served as
resources and prompts. The model stays outside the kernel: the kernel makes itself completely
usable from there, and swapping the model changes nothing in the analysis.

### Added

- **`stkcraft-agent-mcp`** — a JSON-RPC 2.0 server over standard input and output, implementing
  `initialize`, `tools/list`, `tools/call`, `resources/list`, `resources/read`, `prompts/list` and
  `prompts/get`. `Server::handle` is a pure function of a request and the session, so the whole
  protocol is tested without a pipe; nine tests cover the handshake, the tool catalogue, a real
  command call through the registry, an unknown tool, the resources and the prompts.
- **`skills/`** — three procedures written for a reader that will act on them: computing passes over
  a station, access from a station on another body, and the shape of the scenario document. Each
  states the order of commands, what the answer means and the traps, because a schema can describe a
  parameter and cannot describe a job.
- **`stkcraft mcp`** in the CLI, loading the skills from beside the executable or the working
  directory; absent skills are not an error, because the tools still work without them.

### Changed

- **The layer table grows to six**, with the MCP server one below the app: it consumes the engine and
  nothing consumes it, which is the same relationship the viewport has.

## [0.1.18] — A fetch that says whether it fetched anything

`cargo xtask fetch-data` now ends with a line you can read at a glance — `12 file(s), 3 fetched,
9 already present, 6.6 GB on disk` — and takes `--force` to re-download everything it already has.
Both exist because "did it run?" was a question the output did not answer: a run that found every
file in place printed twelve identical "present and verified" lines and looked like it had done
nothing.

### Added

- **A summary line** ending every run, counting what was fetched and what was already there.
- **`--force`**, which re-downloads and re-verifies, for proving the download path works without
  deleting files by hand first.
- **Readable sizes** in the per-file line: `32.7 MB` rather than `0.0 GB`.

## [0.1.17] — All the bulk data, fetched in the background

`cargo xtask fetch-data` now knows every bulk file this project will need, not only the ephemerides:
Earth orientation, the leap-second kernel, the planetary constants and the star catalogue join the
seven SPK kernels, and all twelve are fetched and verified by one command. The local `data/`
directory holds 6.2 GB of it, so working on the code never waits for a download.

### Added

- **Five more rows in the fetch table**, each with the SHA-256 of a file that was downloaded and
  hashed here: `finals2000A.all` (IERS Bulletin A, 3.8 MB), `naif0012.tls` (leap seconds, 5 KB),
  `pck00011.tpc` (IAU 2015 constants, 131 KB), `gm_de440.tpc` (DE440 masses, 12 KB) and
  `hygdata_v40.csv.gz` (HYG star catalogue, 13.6 MB). The `pck00011.tpc` checksum matches the one
  already recorded in the body crate's generated header, which is a useful cross-check on both.
- **`.gitignore` covers the new shapes**: `*.tls`, `*.all` and `*.csv.gz` in addition to the kernel
  patterns, so a fetched file cannot be committed by accident.

## [0.1.16] — Downloads in parallel

`cargo xtask fetch-data` now fetches large kernels as parallel byte ranges: `curl` processes on
several threads, joined and then verified, with `--jobs` to trade breadth for bandwidth. The 2.1 GB
Triton kernel is the reason — one connection took minutes per hundred megabytes.

### Added

- **Parallel ranged downloads** in `fetch-data`, for files of 16 MB and over (eight ranges by
  default, `--jobs 1..32`). Each range is checked for length before the pieces are joined, a short
  range is an error rather than a short file, and the recorded SHA-256 still decides whether the
  result is usable.
- **`--jobs <n>`** on the task, because the right number depends on the link rather than on a
  constant this repository can guess.

### Fixed

- **The task's own options were being eaten by the arg parser**: every argument starting with a dash
  was dropped as though it belonged to cargo, when only the leading ones before the task name do.
- **`curl`'s `-r` and `-o` were passed in the wrong order**, which made a ranged fetch write the
  response to standard output instead of the file.

## [0.1.15] — Triton

Triton was the one body in the table that no fetched kernel could place: the Neptune system file
carries Neptune and its moons but not Triton's own ephemeris, so the reader refused it by name. Its
2.1 GB kernel is now in the fetch table with the checksum of the file that was verified, and the
CSPICE regression covers **54 reference states** — eight planets and eight moons, three epochs each,
all under a metre.

### Added

- **`nep097xl-801.bsp`** in `docs/ephemeris-data.md` and `cargo xtask fetch-data`, with the SHA-256
  of the download the tests ran against. Triton resolves `801 → 8 → SSB` across two kernels, which
  is the chain the reader already walked for Mars and Jupiter.

### Changed

- **`Body::Triton` is placeable now**, so the notes that said a backend refuses it have been
  corrected rather than left to contradict the tests.

## [0.1.14] — VSOP87: the full analytic theory

The analytic backend the plan called for is here: VSOP87A, Bretagnon & Francou's published
planetary theory, carried as a generated table and agreeing with CSPICE to **under an arcsecond**
over fifteen heliocentric reference states — Mercury comes out at 0.04″. It replaces nothing: the
JPL Keplerian backend stays because it is a fraction of the size and a caller may want exactly that
trade.

### Added

- **`Vsop87Ephemeris`** — heliocentric position and velocity for the eight planets from the
  published Poisson series, rotated into the ICRF with the J2000 obliquity (84381.406″, IAU 2006),
  with the velocity obtained by differentiating the series analytically and checked against a
  finite difference.
- **`generated_vsop.rs`** — 9 537 terms truncated at 1e-7 au, with the distribution, the record
  format and the truncation stated in the module header. The Moon is refused by name: VSOP87 is a
  planetary theory and the lunar motion needs ELP2000.

### Fixed

- **The residual that blocked this for three rounds was the reference, not the theory.** Comparing
  VSOP87 against CSPICE's *heliocentric* vectors through the standard obliquity rotation gives
  agreement to tens of kilometres; the 657 000 km gap came from a frame conversion in the test rig.
  The lesson is recorded in the changelog because it is the second time in this arc that a
  reference conversion, not the code under test, was wrong.

## [0.1.13] — Planet-centred access from the command line

`stkcraft planet --body Mars --radius 4000 --inclination 30 --lat 0 --lon 0 --mask 10` runs the same
engine command the window would, and prints the passes. The option parsing lives in the CLI, the
answer comes from the registry, and nothing in the binary propagates or intersects anything itself.

### Added

- **`stkcraft planet`**, with every option documented in `--help` and a test that parses all of
  them. An unknown body is refused by name, and a value that is not a number names the option it
  was given for.

## [0.1.12] — Planet-centred access is a command

`planet.access` runs a satellite's passes over a station on any body the body crate models, from one
registry entry, so the CLI, a menu and an agent all reach the same code. It builds a circular orbit
about the body, samples it, bisects each visibility boundary to a second, and reports the passes.

### Added

- **`planet.access`** — parameters `body`, `radius-km`, `inclination-deg`, `latitude-deg`,
  `longitude-deg` and optionally `mask-deg`, `hours`, `step-seconds`. The body is named as the IAU
  names it (`Mars`, `Titan`); an unknown name is refused by name rather than defaulted.
- **`find_passes`** in the engine: the timeline half of the planetary access question, with the
  bisection tolerance and the sampler's one honest limitation — a step long enough to step over a
  short pass — stated in the module rather than left to be discovered.

## [0.1.11] — A station on Mars can see a satellite

Phase 4's last piece: `PlanetAccess` answers "does this station see that target" on any body the
body crate models, using that body's own ellipsoid and its IAU 2015 rotation to put the target into
body-fixed axes. The existing access machinery does the rest, so a bug here can only be a rotation
or an ellipsoid — both of which the body crate already checks against CSPICE.

### Added

- **`PlanetAccess`**: a planetodetic station, a sensor and a body, with `visible_at` taking a
  target in ICRF axes about that body — what a two-body propagation produces — and returning
  whether it is in view. Verified with the hand-computable case: a target on the zenith is visible
  and its antipode is not, on Mars, whose flattening is four times the Earth's.

### Changed

- **`stkcraft-astro-access` takes `stkcraft-astro-body`** (L2 → L3, allowed by the layer rule), which is what
  lets a station sit on any body instead of only on WGS-84.

## [0.1.10] — Moons

The body crate knows seven moons of other planets as bodies in their own right — Phobos, Deimos,
Io, Europa, Ganymede, Callisto and Titan — with IAU 2015 radii and rotation models and DE440
masses, and the CSPICE regression now covers fifty-one reference states: eight planets and seven
moons, three epochs each, all reproduced to better than a metre in position through the satellite
system kernels.

### Added

- **Eight moons in `Body`**, each with its own GM, its three radii (several are triaxial: Phobos is
  13 by 11.4 by 9.1 km), its pole and prime-meridian model, and the periodic terms the kernel
  carries. Triton is in the table as well; no kernel this repository fetches carries its
  ephemeris, so an ephemeris backend refuses it by name instead of placing it.

### Changed

- **The radius invariant is physical, not assumed.** The old test required `x <= y`, which triaxial
  moons violate; it now requires that the polar radius not be the largest, which is what the IAU
  data actually guarantees.

## [0.1.9] — A scenario can be about another planet

The engine's document now names its central body, and an orbit about Mars, Jupiter or Titan has a
propagator: the exact two-body solution about that body's own gravitational parameter, in ICRF axes
with the body's centre as the origin. SGP4 remains what it is — an Earth model — and the two are not
confused for one another.

### Added

- **`Scenario::central_body`**, defaulting to the Earth and serialising by name, so every scenario
  written before the field existed still loads and still means what it meant.
- **`PlanetOrbit`**: classical elements about any body the body crate knows, with
  `state_at(epoch)` returning position and velocity from the two-body solution. Checked against
  `v = sqrt(mu/r)` for a circular orbit, against the conserved energy and angular momentum
  `E = −mu/2a` and `h = sqrt(mu·a·(1−e²))`, and against the antipode a circular orbit reaches after
  half a period. An element set inside the body, an eccentricity of one, a non-finite angle and an
  epoch on the wrong time scale are all refused by name.

### Not built yet

- **`PlanetOrbit` is two-body.** No J2, no third body, no drag: for a low orbit about a real planet
  that is a kilometre-per-minute error along the track, which is why it is stated in the module
  rather than discovered later.
- **Moons are not in `Body` yet**, so an orbit about Titan or Io has no central body to name.

## [0.1.8] — All eight planets, and the kernels to reach them

Uranus and Neptune are in the regression table: thirty CSPICE reference states now cover every
planet the kernel models, at three epochs each, and all of them reproduce to better than a metre in
position. The five satellite-system kernels that make the outer planets placeable are recorded in
`docs/ephemeris-data.md` with the SHA-256 of the files that were actually verified, and
`cargo xtask fetch-data` will fetch and check all six.

### Added

- **`ura111xl-799.bsp` and `nep097xl-899.bsp`** in the data table, with the checksums of the
  downloads that were used: Neptune's is the 370 MB file the tests ran against, not a guess at what
  the publisher's copy hashes to.

### Fixed

- **The velocity tolerance now says what it is.** Uranus's kernel spreads centuries over long
  Chebyshev records, so the last bits of an evaluation are a micrometre per second; the test bounds
  that explicitly instead of holding every kernel to the inner planets' precision.

## [0.1.7] — An ephemeris with no data file

JPL publishes Keplerian elements and rates for the eight major planets, with the algorithm and the
accuracy they reach. That table is now a backend: `KeplerianEphemeris` places every planet with no
kernel, no download and no build step, which is what a wasm build, a test and an offline machine
need.

### Added

- **`KeplerianEphemeris`**, transcribed from JPL Solar System Dynamics' *Keplerian Elements for
  Approximate Positions of the Major Planets* (Table 1, 1800–2050) with the published accuracy
  stated in the module: from 15 arcseconds for Mercury to 600 for Saturn. The tests hold it to
  those figures over three epochs per planet against CSPICE, and the measured errors are recorded
  beside the bounds.
- **`Origin`** on the `Ephemeris` trait. The Keplerian elements are heliocentric and a kernel is
  barycentric, so a backend now says which it is instead of a caller assuming; the Keplerian one
  reports the Sun and names the Earth as the Earth-Moon barycentre, which is what JPL's table
  actually describes.
- **The Moon is refused by name** in that backend rather than given a Keplerian guess: the table
  carries the planets, and the Moon's motion needs the lunar theory.

## [0.1.6] — Every planet, checked against NASA's own numbers

The reader places Mars, Jupiter and Saturn as bodies, not only as barycentres, and the regression
table now holds 24 CSPICE reference states covering the Sun, Mercury, Venus, the Earth, the Moon
and those three giant planets at three epochs each. All 24 agree to better than a metre in position
and a nanometre per second in velocity.

### Fixed

- **Summary records are a linked list, not a range.** Saturn's kernel puts its summary records at
  either end of a 660 MB file with the data between them, so iterating `FWARD..=BWARD` read data
  records as summaries. Following each record's `NEXT` pointer is what the format asks for; the old
  code at least failed loudly on a nonsense count rather than inventing segments.
- **A target can appear more than once.** A kernel that covers a long span writes one segment per
  time range, so taking the first match for a body answered "outside the coverage" for epochs the
  kernel does cover — Jupiter did exactly that at J2000. Segments are now selected by coverage.
- **Type-3 segments**, which store position *and* velocity coefficients; the satellite-system
  kernels use them, and their velocity comes from the stored coefficients rather than from
  differentiating the position series.

### Added

- **`de440s.bsp`, `mar099.bsp`, `jup365.bsp` and `sat441.bsp` read as one ephemeris**, which is what
  puts Mars, Jupiter and Saturn themselves in reach: `Body::Mars` resolves `499 → 4 → SSB` across
  two files without the caller knowing which file holds which step.

## [0.1.5] — Every version is tagged, and data is fetched rather than committed

Releases are tags in this repository and in its siblings, and 0.1.2 through 0.1.4 were committed
without them. That is now a command rather than a habit: `cargo xtask release` refuses to tag unless
the changelog names the current version, creates the annotated `vX.Y.Z` tag, pushes it, and prints
the `gh release create` line that publishes it with generated notes. The three missing tags have
been created and released.

### Added

- **`cargo xtask release`** — tag and push the version the changelog names, and print the release
  command. The step exists because it was forgotten three times in a row and everything else about
  a release here is mechanical.
- **`cargo xtask fetch-data`** and `docs/ephemeris-data.md` — the ephemeris kernels are downloaded
  from NAIF with a recorded SHA-256 and verified, into a `.gitignore`d directory; nothing is
  committed and a deployment fetches the same files from the same URLs. A row is added to the table
  only with the checksum of a file that was actually verified, never in advance.

### Changed

- **`.gitignore` excludes ephemeris data**: `*.bsp`, `*.bpc`, `*.tpc`, `*.tf` and `data/`. A kernel
  in the repository would age silently against the file a deployment actually reads.

## [0.1.4] — Several kernels, one ephemeris, and an asset gate that means what it says

A planetary kernel carries the system barycentres and a satellite-system kernel carries a planet
and its moons about one of them, so `Mars → Mars barycentre → SSB` spans two files. `EphemerisSet`
reads any number of kernels as one ephemeris and walks each chain across them, which is what makes
"ask for any planet" a single call rather than a question about which file to open.

### Added

- **`EphemerisSet`** — several SPK kernels behind the one `Ephemeris` trait, with the chain lookup
  crossing files. A body no loaded kernel holds is still refused by NAIF identifier.
- **`cargo xtask assets` now ignores directories git ignores.** `plan/` is the local scratch area
  `AGENTS.md` describes, and a file parked there was failing the gate as though it were shipping. A
  directory named by a plain `.gitignore` entry no longer counts as an asset of this repository.

## [0.1.3] — The ephemeris reads a trajectory

The SPK reader evaluates type-2 segments, and `stkcraft-astro-ephemeris` now answers the question it was
built for: where is a body at a TDB instant, in barycentric ICRF coordinates, out of a JPL kernel.

### Added

- **Type-2 SPK evaluation.** Records first, then a four-number directory at the end of the segment
  carrying `INIT`, `INTLEN`, `RSIZE` and `N`; `RSIZE` is a whole record in array elements, so the
  coefficient count is `(RSIZE − 2) / 3`; and a segment with `N = 0` is degenerate rather than
  empty, which is how DE440 writes a barycentre that coincides with its planet.
- **Verified against CSPICE**, which is the only claim that matters here: fifteen reference states
  — the Sun, Mercury, Venus, the Earth and the Moon at three epochs each, the Moon's being a
  two-step chain through the Earth-Moon barycentre — agree to better than 1e−3 km in position and
  1e−9 km/s in velocity over `de440s.bsp`. A previous attempt, reading the directory from the front,
  did not come within 2 000 km; the format document settled it, and the difference between the two
  layouts is recorded in the module documentation so it is not guessed at again.

### Not built yet

- **The analytic backend** (VSOP87 and ELP2000 truncations) that would place the planets with no
  data file at all.
- **Mars, Jupiter, Saturn, Uranus and Neptune themselves.** `de440s.bsp` carries only their system
  barycentres; the body-to-barycentre offset needs a satellite-system kernel, and the reader says
  so by name instead of returning the barycentre as if it were the planet.

## [0.1.2] — The ephemeris crate, and the layout it will not guess

`stkcraft-astro-ephemeris` exists: the trait every backend will fill, a Chebyshev evaluator, and a reader
for the NAIF container the JPL ephemerides are published in. What it does *not* do is read a
trajectory out of a segment, because that layout did not check out, and a wrong ephemeris produces
a wrong mission timeline quietly.

### Added

- **`stkcraft-astro-ephemeris` (L3), with an `Ephemeris` trait** — barycentric ICRF position and velocity
  for a `Body` at a TDB instant, and nothing else. Keeping the contract that narrow is what makes a
  second backend a drop-in later. A TDB epoch is required rather than converted silently, because
  the kernel's argument is TDB and the conversion belongs to `stkcraft-framework-time`.
- **A DAF/SPK structure reader** implemented from NAIF's public format documents: the file record,
  the summary records (whose control words are doubles so they read the same on either endianness),
  and every segment's interval, target, centre, frame, type and word addresses. Checked against
  `de440s.bsp`: fourteen segments, the ten barycentres plus the Earth, Moon, Mercury and Venus
  chains, exactly as the file advertises. Little-endian IEEE only — a big-endian file is refused
  rather than byte-swapped into a plausible wrong trajectory.
- **A Chebyshev evaluator** using Clenshaw for the value and a derivative series for the rate,
  tested against the closed forms, the `Tₙ(±1)` identities and a finite difference.

### Not built yet

- **Reading a type-2 trajectory.** The documented layout — initial state, interval length,
  coefficient count, record count, records — does not reproduce CSPICE on `de440s.bsp`: the word a
  summary points at is the first record's midpoint, and no coefficient count, header length or axis
  ordering tried brought the evaluation within 2 000 km of `spkezr`. The evaluator therefore returns
  `Error::Unsupported`, states why, and the plan document records what was tried. The next step is
  to read the SPK Required Reading against a known-good implementation instead of fitting numbers.
- **The analytic backend** (VSOP87 and ELP2000 truncations) that would place the planets with no
  data file at all.

## [0.1.1] — The kernel learns more than the Earth

The window draws through the same `wgpu` pipelines as the offscreen verification path, time
reaches TDB, and a new crate carries the solar-system bodies with the IAU's own rotation models.

### Fixed

- **The window draws the Earth.** It draws the globe, the ground tracks, the graticule, the
  markers and the star field through the same `wgpu` pipelines the offscreen verification path
  uses, instead of projecting vertices on the CPU and handing `egui`'s painter line segments.
  Two consequences a user sees on the first frame: the Earth is a lit sphere rather than a
  starfield with an outline on it, and the stars are drawn at all — the painter path never
  added them.
- **A ground track on the far side is hidden by the Earth.** The painter had no depth buffer,
  so the half of every track behind the globe was drawn over the disc. The window's pass now
  carries a depth attachment (`NativeOptions::depth_buffer`), the globe writes depth and the
  lines test against it. This is checked numerically rather than by eye: both verification
  paths sample every track vertex, skip the ones within 2 % of the limb, and require that no
  vertex geometrically behind the globe shows its own colour on screen — with the vertices in
  front required to be drawn, so a check that saw nothing could not pass. With the depth
  compare on the line pipeline changed to `Always`, that check reports 100 of 100 far-side
  samples drawn over the globe, which is the defect it exists to catch.

### Changed

- **The pipelines, the shader and the buffers moved into `stkcraft-ui-viewport`.** `SceneRenderer`
  owns the two pipelines, the uniform and one growable buffer per drawable, so the window and
  the screenshot path share them by construction rather than by both happening to create the
  same descriptors. Buffer writes go through `queue.write_buffer` into a buffer that is
  reallocated only when it has to be, instead of a fresh allocation per drawable per frame.
- **`SceneCallback` is the window's drawing path.** It is an `egui-wgpu` paint callback — the
  way to draw into the pass `egui` already records — so the scene lands in the pass that has
  the depth attachment and nothing has to be blitted in from a second swapchain. The surface
  format is read at callback time, and a surface format that changes (dragging the window to a
  monitor driven by another GPU) rebuilds the pipelines instead of drawing with the wrong ones.
- **`STKCRAFT_SCREENSHOT_EGUI` renders the window's own path offscreen**, and with the central
  panel unframed its image is byte-identical to `STKCRAFT_SCREENSHOT`'s. `AGENTS.md` documents
  both.
- **The default branch is `main`**, matching the sibling Craft apps. The two workflows that
  filtered on `master` moved with it in the same commit, because a rename without them would
  have left every push to the default branch unchecked.

### Added

- **`TDB` is a real time scale.** `stkcraft-framework-time` used to treat it as TT; it now carries the
  seven largest terms of the Fairhead & Bretagnon (1990) series in the truncated form of Kaplan
  (2005), USNO Circular 179. Tests compare it with ERFA's full series at eight epochs from 1985
  to 2025: worst disagreement 5.2 µs. `TDB → TT` is a fixed-point inversion, because the series
  argument is a TT quantity that is not known until the correction is.
- **`stkcraft-astro-body`, a new crate beside `stkcraft-astro-frames`,** answers three questions about a
  solar-system body and no others: how massive and how large it is (DE440 masses, IAU 2015
  radii), how it is oriented at a TDB instant (IAU 2015 pole and prime-meridian models, with the
  periodic terms the kernel carries), and where a point on its surface is (planetocentric and
  planetodetic latitudes against that body's own ellipsoid).
  - Ten bodies: the Sun, Mercury, Venus, the Earth, the Moon, Mars, Jupiter, Saturn, Uranus and
    Neptune, keyed by NAIF identifier as well as name.
  - The rotation matrices agree with CSPICE's `pxform` over the same kernel to 1e−8 or better for
    nine bodies at three epochs. That comparison is what established the convention — the pole's
    right ascension uses a sine where the declination uses a cosine — and a structural test
    would have accepted three of the wrong answers.
  - **Mars is incomplete and says so.** Its coefficient arrays in `pck00011.tpc` do not map
    positionally onto its angle list, and no offset reproduces CSPICE, so the periodic terms are
    not guessed at: the body ships with its polynomial terms, and a test records the difference
    and what to do when it is resolved.

### Changed

- **Version control rule, now enforced by `cargo xtask ci`.** Every commit bumps the smallest
  version component and writes the `CHANGELOG.md` section that names it; `cargo xtask version
  bump` does the first half, and the gate fails if the newest section and the workspace version
  disagree, or if an `[Unreleased]` heading is left behind.

### Not built yet

- **Lines are one pixel wide.** `wgpu` has no portable line width, so a thick line needs
  screen-space quad expansion — a geometry change, not a parameter.
- **No anti-aliasing on the geometry.** The window runs one sample; `eframe` supports MSAA, but
  enabling it means building the pipelines for the sample count, so it is a decision rather
  than a flag.
- **The ephemeris, the engine's central body and the moons** are the next three steps of the
  solar-system arc; `docs/plans/2026-11-05-solar-system-kernel.md` records their acceptance
  criteria.

## [0.1.0] — The 3D viewport: the analysis, seen

The first release whose minor version changes, because the product now has a user interface.
`cargo run --release -p stkcraft` opens a window showing the Earth, the satellites in a
scenario and their ground tracks, with a clock, a camera and the access table in a side panel.
The command line is unchanged; a subcommand still goes to the CLI, and an agent still gets no
window.

**The rule from `AGENTS.md` is what shaped this release.** The viewport draws; it never
computes. Everything it shows arrives in a `Frame` the app has already built from propagated
positions, so replacing the viewport cannot change an answer and a number on screen is always
the engine's number.

![The 3D viewport at 0.1.0](docs/images/viewport-0.1.0.png)

### Added

- **`stkcraft-ui-viewport`, a new crate at the UI layer** (`APP_LAYER - 1`), depending only on
  `stkcraft-framework-core`, `wgpu`, `egui` and `bytemuck`:
  - `camera` — a turntable camera and the four matrices the viewport needs, written out rather
    than pulled from a maths crate. `Mat4` is column-major, which is what WGSL expects.
  - `frame` — `Frame`, the whole interface between the engine and the viewport: markers, ground
    tracks, colours and the Earth radius. A plain data structure with no reference to the
    scenario, which is what makes the viewport swappable.
  - `geometry` — the sphere, the graticule, the star field, polylines and markers, all drawn in
    code. Generated geometry carries no licence and needs no `ATTRIBUTION.md` row.
  - `renderer` — the `Globals` uniform, the vertex layout and the WGSL shader, as a string in
    the source so it cannot go missing at runtime.
  - `scene` — the pure `(Frame, Camera) -> SceneData` assembly, which is where the drawing
    decisions live and which is testable without a GPU.
- **The desktop application**, in `apps/stkcraft`:
  - `app` — the `eframe` shell: a window, the camera input (drag to orbit, wheel to zoom at a
    constant ratio per notch), a simulated clock with a rate multiplier, pause/reset/step, and
    a side panel listing the scenario, the access table and any problems. The access panel runs
    the engine's `access.compute` command rather than computing anything itself.
  - `frame_builder` — `Scenario` plus an instant into a `Frame`: positions through
    `Propagator::itrf`, ground tracks sampled over one orbit period, every failure reported
    rather than swallowed.
  - `screenshot` — an offscreen `wgpu` render of the same scene to a PNG, switched on by
    `STKCRAFT_SCREENSHOT`. This is how the viewport is verified on a machine with no window
    system, and it is what produced the image above.
- **`docs/images/viewport-0.1.0.png`**, generated by this repository and attributed.
- **`docs/plans/2026-11-05-viewport.md`** records the arc.

### Fixed

Five defects, every one found by rendering and *looking* rather than by a test passing. They
are listed because each cost real time and each has a lesson:

- **The star field was invisible.** The stars were placed at 60 camera distances while the far
  plane was 100 camera distances — inside the frustum — and they still did not draw. The cause
  was the *ratio* between the near and far planes: a 300 km near plane against a 3 000 000 km
  far plane leaves 24-bit depth unusable at the star radius, so the depth test discarded them.
  The camera now carries explicit `near_ratio` and `far_ratio`, the planets are `0.05` and `4.0`
  of the camera distance, and the star sphere is at `0.9` of the far plane *by construction*.
  A test now sweeps the zoom range and asserts the stars stay inside the far plane.
- **The graticule and the ground track were invisible.** Both sat at `r × 1.001` above the
  surface, which is a quarter of a pixel at any useful zoom, and a coplanar line loses the
  depth test to the surface it hugs about half the time. The obvious fix — a `DepthBiasState`
  on the line pipeline — is rejected by `wgpu` for a line list ("depth bias is not compatible
  with non-triangle topology"). The graticule is therefore lifted by `0.4 %` (25 km, about one
  line width) and ground tracks are lifted to at least the same radius.
- **The ground track was hidden behind the globe.** With the camera looking at the geocentre,
  the satellite passes between the camera and the Earth, so a polar-orbit track was only
  visible where it crossed the limb. The camera now has a `globe_lift`, and the default view
  is lifted a third of an Earth radius so the track shows on the near face. The framing is
  arithmetic, not taste: `asin(6378 / 23000) = 0.28 rad`, about 70 % of the 0.8 rad field of
  view.
- **The ground-track sampler used the wrong unit.** `Elements::mean_motion` is radians per
  minute, not revolutions per day; the sampler treated it as the latter and produced a track
  spanning a day, whose nearest point was 10 800 km from the satellite. The test that caught
  it now asserts the marker lies within one sample of its own track.
- **The app's colour conversion turned infinity into black.** Rust's `clamp` propagates NaN, so
  the `is_finite` guard was mapping `+inf` to zero; NaN and infinity are now handled separately.

### Changed

- **`eframe` 0.37 changed the application trait.** `App::update(ctx, frame)` is now
  `App::ui(ui, frame)` and panels are shown inside a `&mut Ui`; `SidePanel` is `Panel::left`,
  `default_width` is `default_size`, and `raw_scroll_delta` is `smooth_scroll_delta`. Recorded
  here because a future upgrade will move again.
- **The `wgpu` in use is 30.0, via `eframe`,** so the two cannot drift: `wgpu` 30 rejects a
  depth bias on a line list, requires `Option<bool>` for `depth_write_enabled`, takes
  `&[Option<&BindGroupLayout>]` for a pipeline layout, and spells `multiview` as
  `multiview_mask`.
- **`xtask/src/layers.rs`** registers the viewport and adds `wgpu` to `GUI_CRATES`, which is
  what the layering rule is about even though no analysis crate may name it.

### Not built yet

- **The window draws through egui's painter, not through the `wgpu` pipelines.** The offscreen
  path in `screenshot` uses the real pipelines, and the two share `SceneData`, so the geometry
  is identical; the window pass is the next arc. Dragging, zooming and the clock all work.
- **Lines are one pixel wide.** `wgpu` has no portable line width, so a thick line needs
  screen-space quad expansion — a geometry change, not a parameter.
- **Lines ignore the depth of the globe** in the painter path, so the far side of a ground
  track is visible through the Earth. The `wgpu` path does not have this problem.
- **No coastlines, no night-side lighting, no satellite models.** All three want a data file or
  an asset with a source and a checksum, which is a separate decision.
- **The Earth orientation parameters are still `negligible()`**, so ground positions carry up to
  about a kilometre of polar-motion and UT1 error. Unchanged from 0.0.7 and still stated.

## [0.0.7] — A command line that does something

### Added

- **A command line that does something.** `stkcraft passes --tle <L1> <L2> --lat --lon --mask
  --hours --step --name` prints a pass table with absolute UTC times, durations and the total
  as a percentage of the window. Every subcommand dispatches through the engine's command
  registry by id, so the CLI is a transport rather than a second implementation.

### Fixed

- **Times were displayed 37 seconds early.** `Epoch::from_utc` stores the instant on the TAI
  scale, and the display path asked for a UTC conversion that `to_scale` deliberately refuses
  (a TAI Julian Date does not carry the leap offset). The refusal was swallowed, so the
  unconverted value was printed as UTC — exactly the TAI−UTC offset. The new `Epoch::to_utc()`
  reads the leap table instead of guessing.

## [0.0.6] — The engine: from a TLE to an access report

The four numerical layers are now joined into something that runs. A new crate,
`stkcraft-astro-engine`, holds a scenario document, turns each satellite's two-line element set into
an Earth-fixed position through the frame chain, runs access over the timeline, and exposes
every user-visible action as a command.

**The product can now answer its central question end to end.** Give it a TLE and a station;
it reports when the satellite is visible and for how long.

### Added

- **`stkcraft-astro-engine` at L4**, above `stkcraft-astro-access`:
  - `scenario` — the document: satellites with their element sets, stations with their sites
    and sensors, a start epoch and a duration. Data, not behaviour, so a saved scenario never
    changes meaning when the physics improves.
  - `propagate` — the join that had no home: TLE → SGP4 → TEME → ITRF/GCRF. It belongs here
    because `stkcraft-astro-sgp4` and `stkcraft-astro-frames` are both L2 and cannot depend on each other.
  - `analysis` — access over a scenario, one satellite-to-station pair at a time or all pairs.
  - `commands` — the command table: seven commands, each with an id, label, menu path,
    documented parameters, an `enabled` predicate and a `run` function.
- **`Epoch::now()` and `Epoch::j2000()`** in `stkcraft-framework-time`.
- **`docs/plans/2026-11-05-engine.md`** records the arc.

### Fixed

- **A satellite above a mid- or high-latitude station was never visible.** The occultation
test asked whether the line of sight's closest approach to the geocentre was inside a sphere of
volumetric radius, `6371.0 km`. That is right for an equatorial station, whose own radius is
`6378.1 km`. But WGS-84 puts a station at 45° at `6367.5 km`, at 60° at `6362.1` and at the
pole at `6356.8` — every station above about 40° latitude sits *inside* that sphere, so the
closest approach of any line of sight from it was its own radius, every line was reported
occulted, and those stations could see nothing at all.

  Every existing occultation test used an equatorial station, so all of them passed. The bug
  surfaced only when a low-Earth satellite over a 45° station found zero passes in a day. The
  test now covers 45°, 60°, 75° and 89° for both an overhead satellite and the antipode.
- **`Epoch` did not survive JSON.** `serde_json`'s shortest-decimal formatting returned some
  values one unit in the last place away — a 20-nanosecond error that only an equality
  assertion notices. `Epoch` now serialises as `"<scale>:<day>+<fraction>"`, two exact decimal
  numbers, which also makes a scenario file legible to a human.
- **Satellite and station ids could be reused** after removal, because the next id was
  `max(existing) + 1`. Removing the last object freed its id and the next object took it,
  silently rebinding any reference still held. Ids now come from a monotonic counter.

### Correctness

Every claim names the test that holds it.

- **A high-latitude station can see its satellites.** Checked at four latitudes for an
  overhead satellite and its antipode. (`a_high_latitude_station_can_still_see_its_satellites`)
- **The published case 00005 reproduces its TEME state through the engine join,** so the join
  is verified before any frame conversion is involved.
  (`a_near_earth_element_set_reproduces_its_published_teme_state`)
- **A geostationary element set stays within 100 km of one Earth-fixed point over a day,** which
  is the end-to-end check that SGP4, precession, nutation and sidereal time agree.
  (`a_geostationary_satellite_stays_near_one_earth_fixed_point`)
- **A low-Earth satellite gives 3 to 8 passes a day** over a station, each lasting 1 to 30
  minutes. Bounded rather than pinned, so the test means something without failing on an
  element-set edit. (`a_low_earth_satellite_has_several_passes_a_day`)
- **Raising the horizon mask strictly reduces total access.**
  (`raising_the_horizon_mask_reduces_the_total_access`)
- **Commands dispatch by id,** and a missing or wrongly typed argument is refused by name.
  (`commands_dispatch_by_id`, `a_missing_required_argument_is_named`,
  `an_argument_of_the_wrong_type_is_refused`)
- **A broken two-line element set never reaches the scenario.** (`an_invalid_element_set_is_refused_by_the_command`)

### Verification

- `cargo xtask ci --offline` passes all six gates: fmt, clippy `-D warnings`, **208 tests**
  workspace-wide, assets, layers, and the wasm build of all six portable crates.

### Known limitations

- **No transports.** Nothing yet calls the command registry from a menu, a shortcut, a CLI or
  a socket. The table is built and tested through the same dispatch path they will use.
- **No persistence beyond serde.** There is no project file format, no undo and no recent-file
  list.
- **Earth orientation parameters still default to negligible.** They are validated and can be
  supplied, but nothing loads them from an IERS file.
- **No inter-satellite access and no coverage grid** — only satellite-to-ground.
- **Still no viewport and no UI.** The `stkcraft` binary prints its version and exits.

## [0.0.5] — When can this satellite see that station

The product's central question is now answerable. A new crate, `stkcraft-astro-access`, turns a
station, a sensor and a target position over time into **access windows with start and end
times**, and every piece of it is held to a case that can be checked by hand.

It does not yet connect to a two-line element set: the target position is supplied by the
caller. Joining SGP4 to this is the engine's job and is the next arc.

### Added

- **`stkcraft-astro-access` at L3**, above `stkcraft-astro-frames`. Six modules, each independently tested:
  - `ellipsoid` — the WGS-84 Earth, so a station is where it really is rather than where a
    sphere would put it. The two defining constants are written down and everything else is
    derived, because a derived constant typed from memory is a transcription error waiting to
    happen.
  - `site` — a geodetic station and its local east/north/up axes, with `up` along the
    **ellipsoid normal** rather than the geocentric radius.
  - `geometry` — elevation, azimuth and range from a station to a target, azimuth clockwise
    from north.
  - `sensor` — a horizon mask, a field-of-view cone and an optional range limit.
  - `occultation` — whether the Earth is in the way.
  - `window` — the window finder, and `problem` — the end-to-end computation.
- **`find_windows`** brackets visibility changes and bisects them to a millisecond. It does
  not sample and threshold: sampling snaps every boundary to the sample grid, so the reported
  start time changes when the step changes, and it loses any pass shorter than the step.
- **`docs/plans/2026-11-05-access-geometry.md`** records the arc, its reference values and the
  reasoning behind each layer.

### Correctness

Every claim names the test that holds it.

- **The acceptance case from `AGENTS.md` holds end to end.** A satellite 500 km directly
  overhead is visible; one below the surface is not. (`a_satellite_directly_overhead_is_visible_and_one_below_the_horizon_is_not`)
- **A sensor cannot see through the Earth.** Even a mask of `−90°` with a `180°` cone reports
a satellite on the far side as not visible, because occultation is checked separately from the
  sensor. (`a_satellite_on_the_far_side_is_rejected_even_at_high_elevation`)
- **A window boundary is a root, not a sample.** The boundary is placed at `1234.5678 s`,
  nowhere near a `60 s` sample, and must be recovered to `1 ms`. This is the test that fails for
  a sampling implementation. (`the_boundary_is_a_root_not_a_sample`)
- **A pass shorter than the step is still found** when it straddles a sample.
  (`a_short_window_between_two_samples_is_found_when_it_straddles_one`)
- **The cone is round, not square.** With the boresight north at 45°, a point 4° off in
  azimuth and 4° off in elevation is 4.84° away on the sky — outside a 4.5° cone, inside a
  4.5° box. (`the_cone_is_round_not_square`)
- **The local frame is the ellipsoid normal, not the radius.** A target along the geocentric
  radius at 45° N sits at `90° − 0.1924°` elevation, not at 90°.
  (`the_up_axis_being_the_normal_matters`)
- **A station at the North Pole sits on the z axis at the polar radius**, which is the test
  that fails if the `(1 − e²)` factor is dropped from the z term.
  (`a_site_at_the_north_pole_is_on_the_z_axis_at_the_polar_radius`)
- **The sidereal, occultation and geometry layers are each checked against computed values**,
  not recalled ones — see the commit messages for the cases where a recalled figure was wrong.

### Verification

- `cargo xtask ci --offline` passes all six gates: fmt, clippy `-D warnings`, **170 tests**
  workspace-wide, assets, layers, and the wasm build of all five portable crates.
- 49 of those tests are in `stkcraft-astro-access`, a new crate with no test debt.

### Known limitations

- **No propagation in this crate.** The target position arrives as a closure. Connecting a
  two-line element set through SGP4 and the frame chain is the engine's job.
- **The line of sight is geometric.** No atmospheric refraction, no light time, no aberration.
  Refraction is zero above about `5°` and half a degree at the horizon, so a mask near `0°` is a
  geometric horizon, not an optical one. Both omissions are named in the crate docs.
- **No antenna gain pattern.** The cone is a hard cutoff; a real dish has a main lobe and
  sidelobes.
- **The Earth occultation test uses a sphere of volumetric radius** (`6371.0008 km`) rather
  than the ellipsoid, because the difference moves a grazing elevation by `0.06°`. The
  elevation test itself uses the exact ellipsoid.
- **No ground coverage grids, no chains, no access between two satellites.** Only
  satellite-to-ground access is implemented.
- **Still no engine, no scenario document, no viewport and no UI.**

## [0.0.4] — From a TEME vector to a usable position

0.0.3 could compute where a satellite is but nothing could use the answer: SGP4 is defined in
TEME, a frame tied to the mean equinox of date, so the position was not comparable with a
ground station, a star or a 3D view. `stkcraft-astro-frames` now carries the whole conversion
chain, and both of its endpoints reproduce the published worked example to `1e-6 km`.

### Added

- **`stkcraft-astro-frames` gains the frame chain, in five tested layers.** Each is a separate module
  with its own reference, so a mistake is caught where it is made rather than at the end:
  - `arguments` — the five IAU-80 Delaunay arguments, checked against ERFA and Vallado to
    `1e-13 rad`.
  - `nutation` — the 106-term IAU-80 series and the mean obliquity. The two references agree
    with each other to `3e-15 rad`, and this crate agrees with both to `1e-14`.
  - `precession` — the IAU-77 angles and matrix, matching the reference matrix to fifteen
    digits.
  - `sidereal` — Greenwich mean sidereal time, matching the reference to `1e-12 rad`.
  - `teme` — the composite conversions, reproducing the worked example to `1e-6 km`.
- **`teme_to_gcrf`.** TEME to the inertial frame as `P · N · E`, where `E` is the geometric
  part of the equation of the equinoxes. This is the frame an ephemeris, a star field or a
  coverage grid should be expressed in.
- **`teme_to_itrf`.** TEME to the Earth-fixed frame as `W · R3(gmst)`, including the
  `ω × r` term that distinguishes a rotating-frame velocity from a merely rotated one. That
  term is about `0.5 km/s` at low Earth orbit, so a conversion that omitted it would look
  right and be badly wrong.
- **`CartesianState`, `EarthOrientation` and `EquinoxTerms`.** `EarthOrientation` never
  defaults silently: `negligible()` is named so that ignoring `UT1 − UTC` and polar motion is
  a visible decision, because the omission costs about a kilometre of ground position.
  `EquinoxTerms` is an explicit argument because the standard changed in 1997 and the wrong
  branch costs about 15 m.
- **`docs/plans/2026-11-05-frames-teme-conversion.md`** records the arc, the reference values
  and the reasoning for each layer.

### Correctness

Every claim names the test that holds it.

- **The GCRF conversion reproduces the published example** to `1e-6 km` in position and
  velocity. (`teme_to_gcrf_reproduces_the_worked_example`)
- **The Earth-fixed conversion reproduces both of its published references** — the kinematic
  terms that apply after 1997 and the geometric ones that applied before — to `1e-6 km`.
  (`teme_to_itrf_with_kinematic_terms_reproduces_the_worked_example`,
  `teme_to_itrf_with_geometric_terms_reproduces_its_own_reference`)
- **The Earth-fixed velocity really is Earth-fixed.** A test compares it against a plain
  rotation and fails if the `ω × r` term is missing. (`the_earth_fixed_velocity_carries_the_rotation_term`)
- **A frame change preserves the radius,** and an inertial one preserves the speed. This is
  the only check that catches a transposed rotation without a reference table at hand.
  (`a_frame_change_preserves_radius_and_speed`)
- **The 106-term nutation table is the published one.** The transcription was diffed field by
  field against its source: 106 rows, zero mismatches. The test also asserts the length and
  the first and last rows, so a truncated paste fails. (`the_table_has_all_published_terms`)
- **Nutation stays inside its published envelope** of 20″ in longitude and 10″ in obliquity
  for every year from 1900 to 2100. (`nutation_stays_inside_its_published_envelope`)
- **Precession is exactly the identity at J2000,** and the matrix agrees with the angles it
  was built from. (`precession_is_the_identity_at_j2000`,
  `precession_accumulates_forty_six_arcseconds_a_century`)
- **Impossible Earth orientation is refused rather than applied.** A pole coordinate of a
  radian is a degrees-where-radians-belong mistake and would tilt the Earth by thousands of
  kilometres. (`impossible_earth_orientation_is_refused`)

### Verification

- `cargo xtask ci --offline` passes all six gates: fmt, clippy `-D warnings`, **121 tests**
  workspace-wide, assets, layers, and the wasm build of all four portable crates.
- 41 of those tests are in `stkcraft-astro-frames`, up from 17.

### Known limitations

- **No Earth orientation file loader.** `UT1 − UTC` and the polar motion coordinates are
  caller inputs until an IERS `finals2000A` reader exists. They are validated against the
  IERS ranges, never guessed.
- **The ground-station local horizon is not here yet.** It needs an ellipsoid and a site,
  which belong with the access geometry rather than the frame chain.
- **No single call from TLE to GCRF.** The conversion takes a `CartesianState`; joining it to
  `stkcraft_astro_sgp4`'s output is the engine's job, and there is no engine yet.
- **TEME only in.** There is no inverse (`gcrf_to_teme`) because nothing needs one yet.

## [0.0.3] — Deep-space propagation made exact

Deep-space (SDP4) propagation was the one known-wrong numerical layer in 0.0.2: worst case
about 85 km over the published verification suite. It is now exact — every one of the 33
verification cases reproduces the reference output to within `4e-9 km`, the limit of the eight
decimals that table prints. The tolerance in the test was tightened from the measured 100 km
to 1 mm, so the old error could not survive unnoticed a second time.

### Fixed

Each item names the defect, because the diagnostics that found them are reusable.

- **The epoch passed to `dscom` was the fraction of a day, not the day count.**
  `Elements::sgp4_epoch` computes the whole days since 1950 January 0.0 and the fraction of
the day, but `sgp4init` read only the first return value and used it as `epoch`. The lunar and
solar arguments `day` therefore had no multi-decade ramp: `zmol`/`zmos` were wrong in their
first digit, and every deep-space secular rate built from them drifted. This was the root cause
of the 85 km error. Regression test:
`the_sgp4_epoch_counts_whole_days_from_the_1950_zero`.
- **`JulianDate` held a half-day inside `day`.** `day` was `X.5` for a UTC midnight, so
  subtracting the SGP4 epoch zero from it could not be exact and the whole-day count came out
  half-integral. `day` is now always an integral day count from JD 0, with all fractional and
  half-day content in `fraction`. That invariant is what makes the fix above possible, and it is
  now documented on the type and asserted in `the_epoch_lands_on_the_stated_day`.
- **`dsinit` derived `inclm` as `asin(sin(i))` instead of using the perturbation.** The
  reference assigns the epoch inclination before calling `dsinit`; recomputing it from the
  sine folds every orbit above 90° of inclination into the wrong quadrant, which silently
  disables or mis-applies the 180° guard that drops the node terms.
- **`dspace` negated the Euler–Maclaurin `step2` term for negative time.** The reference uses
  `+259200` in both directions; only the first-order `delt` changes sign. `xni` was therefore
  wrong by twice the second-order term when stepping backwards.
- **`xfact` used the Kozai mean motion instead of the un-Kozai'd one,** for both resonances.
- **The Kepler iteration clamped at 1.0 rather than 0.95,** and used `signum()` where the
  reference chooses `+0.95`/`-0.95` with an explicit comparison. The published case 23333
  exists to exercise this clamp.
- **The lunar secular terms read the wrong `z` set.** `didt`, `dmdt`, `sghl` and `shll` must
  use the lunar `z` values that `dscom` leaves after its second pass, while `sis` uses the
  solar `sz` capture from the first. The reference implementation's parameter names are
  misleading here, so each term is now checked against the reference's own printed values
  rather than against its naming.

### Changed

- **`DEEP_SPACE_TOLERANCE_KM` is `1e-6 km`,** matching the near-Earth bound. A single named
exception (`NEAR_PARABOLIC_TOLERANCE_KM`, `1e-5 km`) covers case 23333, whose `e = 0.97`
multiplies the round-off by a near-unity `1/(1 - e cos E)` while converting to position.
- **The verification test is renamed** from `deep_space_cases_stay_within_the_documented_tolerance`
to `deep_space_cases_reproduce_the_published_output`, since it now asserts accuracy rather
than a bound.

### Verification

- All 33 published cases, near-Earth and deep-space, match `tforverf.out` to `4e-9 km`.
- `cargo xtask ci --offline` passes all six gates: fmt, clippy `-D warnings`, 97 tests,
  assets, layers, and the wasm build of all four portable crates.

The defect localisation used a small C++ harness compiled against the published reference
source to print intermediate `dscom`/`dsinit`/`dspace` values, and compared them term by term
with the Rust implementation. That harness was scratch work and is not committed.

### Known limitations

- **TEME only.** The output frame is still the one SGP4 is defined in; conversion to ICRF and
to an Earth-fixed frame is the next arc.
- **Two error codes differ from the published file, deliberately** — unchanged from 0.0.2 and
documented in that section.

## [0.0.2] — SGP4 and SDP4 propagation

The first version that can compute where a satellite is. It reads a two-line element set and
turns it into an inertial state vector, using the same theory the public element sets are
generated for.

**It is verified against the published reference for near-Earth orbits and is not yet exact
for deep-space orbits** — see *Known limitations* before trusting a high-orbit result.

### Added

- **`stkcraft-astro-sgp4` (L2).** A from-scratch implementation of SGP4 and SDP4, written against
  the published theory: Vallado, Crawford, Hujsak and Kelso, *Revisiting Spacetrack Report 3*,
  AIAA 2006-6753, together with its reference source code, courtesy of CelesTrak, and the
  original Hoots and Roehrich Spacetrack Report 3 (1980). Every constant and every
  intermediate quantity carries the routine it comes from in a comment.
- **A strict TLE parser.** Fixed-column parsing with the checksum enforced, the implied
  decimal and exponent notation (`28098-4` is `0.28098e-4`) decoded, the two-digit epoch year
  resolved with the published 1957–2056 window, and Alpha-5 catalog identifiers carried
  through as text. A malformed line is refused with the line and column named; nothing is
  guessed and nothing is silently defaulted.
- **Typed elements and states.** `Elements` converts once, at one boundary, from the TLE's
  degrees and revolutions per day into the radians and minutes SGP4 is written in.
  `StateVector` carries `Distance` and `Velocity`, not bare `f64`, and is documented as TEME —
  the frame SGP4 is defined in.
- **The reference error codes as a type.** `PropagationError` keeps the reference
  implementation's numbering (1–6) so published logs and test files still read correctly, but
  as an enum so the meaning travels with the value. A decayed, sub-orbital or
  out-of-family element set returns an error; it never returns a plausible-looking number.
- **The full verification suite.** All 33 element sets and all 667 expected rows from
  `tforverf.out` are transcribed into `crates/sgp4/src/tests/vectors/data.rs`, byte for byte,
  with their source named.
- **`cargo xtask`** now checks `stkcraft-astro-sgp4` builds for `wasm32-unknown-unknown`, and the
  crate is registered in the layering table at L2.

### Correctness

Each claim names the test that holds it.

- **Near-Earth propagation matches the published output exactly.** All eight near-Earth
  verification cases reproduce `tforverf.out` to within `4e-9 km` — the limit of what a table
  printed to eight decimals can express. (`near_earth_cases_reproduce_the_published_output`)
- **Deep-space propagation runs the right branches.** Every deep-space case that initializes
  takes the `dspace` path and stays within the documented tolerance; the one-day and half-day
  resonances are both exercised.
  (`deep_space_cases_stay_within_the_documented_tolerance`)
- **Both halves of the theory are covered.** 29 of the 33 cases initialize and both the
  near-Earth and the deep-space branches are exercised; the other four are refusals.
  (`every_case_initializes_or_is_one_of_the_three_broken_sets`)
- **A decayed element set fails rather than lying.** Case 29141 propagates, then reports an
  error before its published run ends. (`a_steadily_decaying_element_set_eventually_fails`)
- **A sub-orbital element set is refused.** Case 28872 has a perigee inside the Earth, so no
  state vector is produced at all. (`a_sub_orbital_element_set_is_refused_at_initialization`)
- **The checksum catches a single transposed digit,** and a mismatch between the two lines'
  catalog numbers is rejected. (`a_flipped_digit_is_rejected_by_the_checksum`,
  `a_mismatched_catalog_number_is_rejected`)
- **A degree/radian mix-up cannot compile,** because `Angle` has no `From<f64>`.

### Known limitations

- **Deep-space propagation is not yet exact.** The near-Earth half of the theory reproduces
  the published output to the last printed digit; the deep-space half is currently within
  about 85 km worst case over the verification suite, and several resonant cases (the
  Molniya-like 12-hour orbits and some one-day ones) are off by tens of kilometres. The
  cause is in the deep-space secular and periodic terms, not in the parser, the epoch
  handling or the near-Earth path, all of which are exact. **Treat a high-orbit position as
  indicative until this is resolved.** The tolerance in the test is set to the current
  accuracy rather than to the intended one, so a regression cannot pass unnoticed.
- **Two error codes differ from the published file, deliberately.** Case 33333 is refused
  with code 5 (sub-orbital) rather than 4, because its perigee is inside the Earth and an
  orbit that does not exist gets no state vector. Case 33335 is documented in the published
  file as exercising code 3, but it propagates cleanly here and in an independent
  transcription of the reference, so it is asserted to succeed.
- **TEME only.** The output frame is the one SGP4 is defined in. Converting to ICRF or to an
  Earth-fixed frame needs precession, nutation and Earth rotation, which belong to
  `stkcraft-astro-frames` and are not wired up yet.
- **WGS-72 only in practice.** `GravityField::Wgs84` exists for reproducing comparison runs,
  but an operational element set propagated with WGS-84 is subtly wrong and the type says so.
- **No access, coverage, scenario document, viewport or UI.** The `stkcraft` binary still
  prints its version and exits.

## [0.0.1] — 2026-11-05

The first version. It establishes the engineering standards and the numerical foundation the
rest of the product is built on. **It cannot yet propagate an orbit, find an access or draw a
3D view**; those are the next three versions.

### Added

- **Workspace and quality gate.** A Cargo workspace (edition 2024, resolver 3) with a single
  version source in `[workspace.package]`, and `cargo xtask` tasks matching the sibling Craft
  apps: `ci`, `fmt`, `layers`, `assets`, `wasm` and `version`.
- **Enforced layering** (`cargo xtask layers`). A crate may depend only on a strictly lower
  layer, and nothing at or below `LAST_NON_UI_LAYER` may depend on egui, eframe, winit or rfd.
  Every workspace member must be registered in `xtask/src/layers.rs`; an unregistered crate
  fails the task.
- **Enforced asset attribution** (`cargo xtask assets`). A file with an asset extension must
  have a row in `ATTRIBUTION.md`, and a row must name a file that exists, so the table cannot
  rot in either direction.
- **`stkcraft-framework-core` (L0).** Typed quantities — `Angle`, `Distance`, `Velocity`, `Mass`,
  `Duration`, `EpochOffset` — with named conversions and no `From<f64>`, so a degrees/radians
  mix-up is a compile error. A single `Error` type whose `Unsupported` variant is the only
  sanctioned way to represent an unfinished feature. A `Vector3` whose `normalized` returns
  `None` for a zero or non-finite vector rather than producing NaN.
- **`stkcraft-framework-time` (L1).** The reference time scales (UTC, TAI, TT, TDB, TCB, TCG, UT1, GPS)
  as a type, the IAU SOFA / ERFA constants with their sources cited, and the published
  `TAI - UTC` leap-second table from the IERS `tai-utc.dat` file.
- **`stkcraft-astro-frames` (L2).** `Rotation3`, which refuses to exist unless its rows are
  orthonormal and its determinant is +1, so a reflection or a scaled matrix cannot be mistaken
  for a rotation.
- **CI wiring.** `.github/workflows/` calls the shared `storytold/craft-common-actions`
  workflows at `@main`; there are no app-local jobs.
- **`AGENTS.md`**, **`docs/architecture.md`**, **`NOTICE`**, **`ATTRIBUTION.md`** and the
  dual MIT / Apache-2.0 licences.

### Correctness

These are the properties the numerical layers guarantee, each with a regression test:

- **Time-scale offsets are exact.** `TT - TAI` is 32.184 s and `TAI - GPS` is 19 s to the
  limit of an `f64`. An earlier implementation subtracted two summed Julian Dates and was
  wrong by about 13 µs; intervals are now computed part by part on the split representation.
- **Sub-microsecond resolution survives.** A one-microsecond step at J2000 is preserved
  through a round trip, which a single-`f64` Julian Date cannot do.
- **Leap-second lookups report their confidence.** A date before 1960 or more than five years
  past the table's revision is flagged rather than presented as a fact.
- **A scenario can carry a newer leap second** through `LeapSecondTable`, without waiting for
  a release, and the built-in table is never mutated by a caller's extension.
- **UT1 conversion refuses to guess.** `DUT1` is published only by IERS, so converting to UT1
  without it is an error, not a defaulted zero.
- **Degree-minute-second formatting carries correctly.** `0°59'59.99999999"` renders as
  `1°00'00"`, never `0°60'00"`.
- **No panics in production code.** Every crate root denies `unwrap`, `expect`, `panic`,
  `unimplemented`, `todo` and `unreachable`, and the workspace forbids `unsafe`.

### Known limitations

- **No propagation yet.** `stkcraft-astro-sgp4` does not exist, so no orbit can be computed.
- **No access or coverage.** The geometry that answers "when can this see that" is not built.
- **No scenario document, viewport or UI.** The `stkcraft` binary prints its version and exits.
- **TDB, TCG and TCB are approximated by TT.** The periodic TDB term (under 2 ms) and the
  TCG/TCB rates are not applied; `Epoch::to_scale` documents this where the offset is added.
- **Converting a uniform-scale epoch back to UTC is not implemented** and returns
  `Error::Unsupported`, because it needs the leap offset at that instant.

[Unreleased]: https://github.com/spacelibres/STKCraft/compare/v0.0.6...HEAD
[0.0.6]: https://github.com/spacelibres/STKCraft/compare/v0.0.5...v0.0.6
[0.0.5]: https://github.com/spacelibres/STKCraft/compare/v0.0.4...v0.0.5
[0.0.4]: https://github.com/spacelibres/STKCraft/compare/v0.0.3...v0.0.4
[0.0.3]: https://github.com/spacelibres/STKCraft/compare/v0.0.2...v0.0.3
[0.0.2]: https://github.com/spacelibres/STKCraft/compare/v0.0.1...v0.0.2
[0.0.1]: https://github.com/spacelibres/STKCraft/releases/tag/v0.0.1
