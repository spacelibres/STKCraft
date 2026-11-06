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
| Crate names repeat their directory, all groups | done, enforced by the gate | 0.1.42 |
| The window serves the control channel, with `ui.state` and `ui.screenshot` | done | 0.1.43 |
| The model layer: any OpenAI-compatible endpoint, the credential stored `0600` | done | 0.1.44 |
| Every CCSDS publication collected, hashed and searchable | done, 495 documents | 0.1.47 |
| Compaction writes a model-written summary that survives a resume | done | 0.1.46 |
| Formula region detection, storage, provenance, search, transcription table | done | 0.1.51–0.1.65 |
| Mathlib lexicon and classifier, with no-match acceptance | done, 217 declarations | 0.1.66–0.1.69 |
| `cargo xtask verify-formulas` (re-extract, re-classify, fail on drift) | done, 0 drifts | 0.1.68 |
| OCR through a configured engine, verified in CI | done, `ocr` job green | 0.1.62, 0.1.70 |
| Extension runtime: packages load and their commands are tools | done | 0.1.71–0.1.72 |
| Sensors (`station.sensor`) and targets (`target.add`, `access.targets`) | done, hand-computable tests | 0.1.73–0.1.75 |
| CCSDS and NAIF corpora, hashed and searchable | done | 0.1.47, 0.1.76 |
| Sources register: who says what, and where they disagree | done | 0.1.76–0.1.77 |

The engine speaks eight commands today: `scenario.rename`, `scenario.set-duration`, `satellite.add`,
`satellite.remove`, `station.add`, `station.remove`, `access.compute`, `planet.access`.

## Next

### Foundation, in this order — before any new domain feature

The framework and the agent come first: every later step (sensors, coverage, the timeline) is a
feature *of* them, and building a feature on a foundation that is still moving is how both end up
half-done. The order below is dependency order, not preference.

#### F1. ~~The model layer~~ — shipped in 0.1.44

`Provider` (endpoint, model, credential, timeout) instead of one hard-wired client; the credential
stored `0600` in `model.key`; `stkcraft model` to show and set it; settings beat the environment and
the environment beats the shipped defaults. **Verified** with nothing in the environment at all.

#### F2. Memory and context, at harness parity — next

Today a long conversation is trimmed **by message count** and compaction *drops* turns with a fixed
note. That is not memory; it is forgetting with a comment.

**Done when**: the drop is triggered by the size of the request the model would receive, not by a
count; the dropped span is replaced by a **summary the model wrote** (with a test proving a fact from
the dropped span can still be recalled); pinned material — system prompt, skill index, the scenario
document — is never cut; a session carries metadata (title, created, last used, scenario); and the
window lists sessions and resumes one.

#### F3. The extension runtime

`stkcraft-framework-extensions` validates a manifest and admits a trust tier, and nothing loads one.
**Done when** an installed package can contribute commands (an engine command with preset parameters),
skills, a language catalog and a knowledge root; discovery, validation and registration go through the
same command registry the menus use, so the CLI, MCP and the control channel see it too; third-party
packages stay behind the non-default feature; and `stkcraft extensions` says which loaded and which
were refused, with the reason.

#### F4. The window's agent surface

The assistant panel runs one conversation with no session, no model choice and no stop. **Done when**
it uses F2's sessions, shows citations as places (`paper.pdf:page 7`), can be stopped mid-turn, and has
a settings section for model, key, language and the knowledge directory — the parts F1 and F3 made
settable, reachable without a command line.

#### F4b. Mathematics as a first-class store — the correctness floor

A misread formula makes every number computed from it wrong, and nobody notices — the failure this
project exists to prevent. `docs/design/mathematics.md` is the decision record: formulas get their own
table, their own recogniser, their own box-shaped citation, and a rule that a formula is never guessed.

**Done when** (1) display mathematics is segmented from the text layer deterministically, stored with its
box and Unicode, indexed, and cited as `paper.pdf:page 12, formula at (120,340)-(300,388)`; (2) a
transcription table over the CCSDS corpus and the SGP4 papers passes; (3) OCR and a formula model run
behind two environment commands, recording engine and version, with the `unrecognised` path tested;
(4) every formula records **what problem it solves** (the introducing sentence) and is **classified
against Mathlib** — a pinned declaration index, a scored match, a domain taxonomy — with the limits
stated: classification, never proof; and (5) every formula carries its **provenance chain** — file
digest, page, box, region digest, reading engine, the introducing sentence, the Mathlib revision — with
`cargo xtask verify-formulas` re-extracting and re-classifying it and failing on drift.

#### F5. Sensors and targets, and what they can see — mostly shipped (0.1.73–0.1.75)

**Shipped**: `station.sensor` (a cone, a horizon mask, a range), `Target { vertices, sensor }` in the scenario
document, `target.add` from `lat,lon;lat,lon`, and `access.targets` whose windows are the union of the target's
vertices' — verified by a test that compares the region's window list to `union(corner, corner)`.

**Still open**: the window does not offer either command, and a target is not drawn on the globe.

#### The corpora, and what each authority is

| Authority | State | Where |
|---|---|---|
| CCSDS | 495 publications, 548 MB, hashed | `cargo xtask standards`, `docs/guides/ccsds-standards.md` |
| NASA / NAIF | 27 documents, 2.2 MB, hashed | `cargo xtask naif`, `docs/guides/naif-corpus.md` |
| ESA / ECSS | **not fetched**: distributed through DOORS, no program-fetchable URL | `docs/guides/sources.md` records the probe and the condition that would change it |

#### F5. Sensors and targets, and what they can see

Access today is a satellite and a station with a horizon mask. A mission analyst's question is
narrower: *with this sensor's field of view, at this maximum off-nadir angle, over this target area,
when and for how long?*

**Done when** a scenario can carry a sensor (cone half-angle, max off-nadir, min/max range) and a
target (point or polygon), `access.compute` accepts them, and the geometry is verified against
hand-computable cases: a target directly under the satellite is visible, one just outside the cone is
not, and a polygon target's window is the union of its vertices' windows.

## Later

- **The extension store**: install, update and verify a package from the window, with the trust tiers
  `stkcraft-framework-extensions` already defines.
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
