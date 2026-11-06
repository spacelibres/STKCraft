# Roadmap

Where STKCraft is, what is next, and what "done" means for each step. `docs/standards.md` is the rule
book and `AGENTS.md` is the contract every change keeps; this page is the order.

Every stage below is a release or a short run of them. A stage is finished when three things are true,
and no stage is finished before all three:

1. **`cargo xtask ci` is green** — the same gate GitHub Actions runs.
2. **A numerical layer matches a published reference** — SGP4 verification vectors, SOFA test values,
   IERS leap seconds, hand-computable access geometry. A plausible number is not evidence.
3. **The user can see it work** — a screenshot, a CLI transcript, or a citation into a real document,
   produced by the release rather than by a claim in a changelog.

## Where the project is

Shipped, in the order it landed:

| Area | State | Since |
|---|---|---|
| Time: IAU scales, leap seconds, two-part Julian dates | done, verified against published values | 0.0.1 |
| SGP4 / SDP4 propagation | done, AIAA 2006-6753 verification vectors | 0.0.2–0.0.3 |
| Reference frames and rotations | done, SOFA transformation checks | 0.0.4 |
| Access geometry (satellite ↔ station) | done, hand-computable cases | 0.0.5–0.0.6 |
| Scenario document, commands, CLI | done | 0.0.6–0.0.7 |
| 3D viewport and the window's panels | done, images checked numerically | 0.1.0, 0.1.21 |
| Ephemerides: SPK kernels, planets, moons | done, JPL kernels fetched and verified | 0.1.2–0.1.18 |
| Access on other bodies | done, planet-centred, from the CLI too | 0.1.9–0.1.13 |
| Agent: MCP server, tools, skills, a real model | done | 0.1.19–0.1.24 |
| i18n: one English source, catalogs, DeepSeek translation | done, `zh` shipped; numbers and dates not yet | 0.1.25–0.1.30 |
| API reference generated from the code | done, translatable | 0.1.26 |
| Extensions: manifest, ids, trust tiers | done, no store yet | 0.1.27 |
| Knowledge base: chunking, BM25 index, cache | done, measured on 2 000 documents | 0.1.31, 0.1.35 |
| Crate taxonomy, storage decision record | done, enforced by the gate | 0.1.32–0.1.34 |
| Control channel: token, methods, MCP-routed | done, CLI only | 0.1.36 |
| Crate names repeat their directory | done, enforced by the gate | 0.1.37 |
| PDF reading with page-level citations | done, measured on real papers | 0.1.38 |
| Paper corpus fetcher, checksummed | done | 0.1.39 |

The engine speaks eight commands today: `scenario.rename`, `scenario.set-duration`, `satellite.add`,
`satellite.remove`, `station.add`, `station.remove`, `access.compute`, `planet.access`.

## Next

### 1. The window serves what the CLI serves

**Why now.** The platform's claim is that anything a person can do, a launcher or a model can do
through the same registry. The control channel exists, but only a CLI process can open it: the window
— which is the thing a launcher is supposed to drive and photograph — does not serve it, and
`ui.screenshot` is not a method. Every sibling app in the family exposes exactly this, and
`AGENTS.md`'s display section already promises the recipe.

**Done when** a running window answers `version`, `engine.commands`, `engine.execute`, and a new
`ui.screenshot` that returns the frame it is showing, with the same token authentication the CLI path
uses; and a test proves the round trip without a display, using the offscreen path the screenshot
modes already provide.

### 2. Sensors and targets, and what they can see

**Why now.** Access today is a satellite and a station with a horizon mask. A mission analyst's real
question is narrower: *with this sensor's field of view, at this maximum off-nadir angle, over this
target area, when and for how long?* That is the product's reason to exist, and every later feature —
coverage, a timeline, a report — is built on it.

**Done when** a scenario can carry a sensor (cone half-angle, max off-nadir, min/max range) and a
target (point or polygon), `access.compute` accepts them, and the geometry is verified against
hand-computable cases: a target directly under the satellite is visible, one just outside the cone is
not, and a polygon target's window is the union of its vertices' windows.

### 3. A timeline the user can read

**Why now.** The viewport shows one instant. A pass is an interval, and an interval is what a person
takes to a review. The engine already computes the list (`access.compute` returns every pass with
start, end and duration); nothing draws it.

**Done when** the window has a timeline panel with the computed passes on it, the viewport can be
driven to a pass instant, and a screenshot of the panel passes the same numeric check the other panels
do — content in both halves, label pixels where the labels belong, and a blank image as the control.

### 4. Recall over the corpus, beyond words

**Why now.** The knowledge base answers exact-token questions well and fails a paraphrase. The vector
store is written and unused; what it needs is a decision — which embedding model, run through whose
key — and the discipline to keep it a *candidate* generator over BM25 rather than an authority.

**Done when** a query that shares no word with the document still finds it, the result is labelled as
recall rather than evidence, the citations remain page-level, and an offline run is identical on two
machines because the embeddings are cached with the index.

### 5. Numbers, dates and direction in every language

**Why now.** The interface translates; the numbers do not. "45°" is written differently in German,
"1 200 km" in French, and right-to-left layout is not a font problem but a layout one.

**Done when** `cargo xtask i18n` reports a locale's number and date formats, the panels use them, and
a `ar` catalog renders right-to-left in the screenshot path.

## Later

- **The extension store**: install, update and verify a package from the window, with the trust tiers
  `stkcraft-extensions` already defines.
- **OCR for scanned papers**, so the knowledge base reads the documents that have no text layer.
- **A corpus regression table**: a known phrase landing on a known page of each fetched document, run
  by a person who has run `cargo xtask papers` rather than by `cargo xtask ci`.
- **Coverage analysis**: a grid or a polygon over a window of time, with a percentage that can be
  checked against a published example.
- **Team scenarios**: more than one person editing a scenario, which the record's revisions already
  make possible.

## The shape that does not change

Three properties are not stages; they are the design, and a change that breaks one is rejected
whatever it delivers.

- **The model stays outside the kernel.** No prompt, model name or network call below the app layer;
  the agent reaches the engine only through the protocol.
- **The user's documents come first.** A factual answer starts with a search over them, and a search
  that finds nothing is reported as nothing.
- **Nothing crashes and nothing guesses.** `Result` instead of `unwrap`, `Error::Unsupported` instead
  of a plausible number, and a published reference behind every formula.
