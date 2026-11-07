# Architecture

STKCraft answers questions about spacecraft and ground stations over time: where is a
satellite at a given instant, what can it see, and for how long. This document explains how
the crates fit together and why they are split the way they are.

## Crates and layers

```text
crates/framework/        the platform — nothing here knows what an orbit is
  core                   units (Angle, Distance, Duration, EpochOffset), the Error type, Vector3
  time                   time scales, the IERS leap-second table, Julian dates and epochs
  i18n                   the interface catalogs: format, lookup, plurals
  extensions             the package contract: manifests, ids, trust tiers
  storage                the record (SQLite), the key-value cache and the vector store

crates/astro/            the orbital-analysis domain — the astronomy lives here
  body                   IAU constants, ellipsoid and rotation models for 18 bodies
  frames                 reference frames and rotations
  ephemeris              NAIF SPK/DAF, VSOP87A and Keplerian backends
  sgp4                   SGP4/SDP4 propagation from a two-line element set
  access                 line of sight, elevation masks, occultation, access windows
  engine                 scenario, propagation, analysis, the command registry

crates/agent/            the model-facing surface
  knowledge              the user's own documents, searched before a model answers
  mcp                    the command registry spoken to an outside client
  runtime                a model driving the engine through those tools
  control                the launcher's socket, driving the same registry

crates/ui/               presentation
  viewport               the 3D viewport: GPU rendering and generated geometry

apps/stkcraft            window, CLI, MCP server and assistant
```

Planned, in the order they will land:

```text
crates/astro/geom        conic sections, sphere coordinate transforms, intersections
crates/astro/atmos       neutral atmosphere density
crates/astro/coverage    grids and grids-of-figures over time
```

`cargo xtask layers` enforces that a crate depends only on a strictly lower layer, and that
nothing at or below `LAST_NON_UI_LAYER` touches any crate in `GUI_CRATES` (`egui`, `eframe`,
`egui-winit`, `egui-wgpu`, `winit`, `rfd`, `wgpu`). Every new
crate must be registered in `xtask/src/layers.rs`; the task fails for a workspace member that
is missing from the table.

The viewport sits above `LAST_NON_UI_LAYER` because it *is* the user interface's drawing code,
but it depends only on `stkcraft-framework-core`: it takes a `Frame` of already-computed positions and
knows nothing about satellites, elements or access. That is the boundary that makes it
replaceable — see below.

## Why the viewport never computes

The other Craft apps recreate what a user can see, and a screenshot is a valid acceptance test.
STKCraft computes physics, where a plausible-looking wrong answer is worse than a crash because
nobody notices it. If the viewport derived an orbit from the current elements — an osculating
ellipse, for instance — it would disagree with the engine the moment drag or J2 mattered, and
the disagreement would be invisible.

So the rule is structural rather than a convention: the viewport's whole input is a `Frame` of
positions the engine produced, and it has no access to an element set. Replacing it cannot
change an answer.

## Why units and time are separate crates

Two classes of mistake in mission analysis produce answers that look right and are wrong:

1. **A unit mix-up.** Passing degrees where radians are expected is a silent factor of 57.
2. **A time-scale mix-up.** Propagating on a uniform scale but stamping the result with a
   UTC time that contains leap seconds shifts everything by a whole second, and a
   conjunction screen that was meant to show a miss now shows a hit — or the reverse.

Both are prevented by types rather than by discipline. `Angle`, `Distance` and `Duration`
carry their unit, and there is deliberately no `From<f64>` so a bare number cannot slip
into a typed position. `Epoch` carries the scale it is expressed in and refuses to take an
interval between two different scales.

## Why Julian dates are stored in two parts

A Julian Date near the present is about 2.46 × 10⁶. An `f64` has about 16 significant
digits, so a single Julian Date resolves only about 40 microseconds. That is coarser than
the 1-second timestep a typical scenario uses, and far coarser than an ephemeris
interpolation wants.

`JulianDate` therefore keeps a whole-day part and a fractional part, following the IAU SOFA
two-part convention. Every interval and every difference is computed part by part — never
by subtracting two summed Julian Dates, which would throw away exactly the precision the
split exists to preserve. A regression test locks this in: a one-microsecond step must
survive a round trip.

## The leap-second table

UTC is steered by inserting whole seconds, so `TAI - UTC` changes. The table in
`stkcraft-framework-time::leaps` is the one in the IAU SOFA / ERFA reference implementation, which in
turn comes from the IERS `tai-utc.dat` file.

Two consequences follow, and both are visible in the API rather than buried:

- A lookup reports its **confidence**. A date before 1960 or more than five years past the
  table's revision is flagged, so a caller can warn instead of pretending the offset is a
  fact.
- A scenario dated past the revision can supply its own `LeapSecondTable` rather than
  waiting for a release.

A day that ends with an inserted second uses the offset in force *before* that leap, which
is SOFA's convention and the one ephemeris files assume.

## The agent-first core

STKCraft is meant to be driven by a model, and the difference from a product that merely
*offers* an assistant is structural: the model is not inside the kernel, and the kernel is
completely reachable from outside it.

The kernel — everything through `stkcraft-astro-engine` — makes no network call, links no HTTP
client, and does not know a language model exists. What it does instead is expose itself
completely:

- **Every command is a tool.** `tools/list` is generated from the command registry, so a
  command's id, its description and the schema of its parameters come from the same
  descriptor the menu and the CLI use. There is no second list to keep in step; a command
  that lands with tests is an agent capability the same day. Each parameter declares its kind —
  a number, text, or degrees — so the schema and the conversion of what a caller actually sent
  are one decision rather than two.
- **A tool returns the answer.** A command that computes pass times returns the times: as lines
  in the text a model reads, and as the same list in `structuredContent` for a program to trust.
  A count is a summary of an analysis, not the analysis.
- **Scenario, catalogue and skills are resources**, read as JSON: the live scenario, the
  bodies and their constants, the command catalogue, and every skill document.
- **Skills are prompts.** A skill is a markdown procedure with a one-line description, served
  through `resources/read` and `prompts/get`, and listed in the system prompt by name only.
  That is deliberate progressive disclosure: a procedure is loaded when a job needs it, so a
  large library costs nothing until it is used.

The payoff is testability. `stkcraft_agent_mcp::Server::handle` is a pure function of a request and
the session, so the whole protocol is covered without a pipe, and an agent's tool call is the
same call a test makes. A model that cannot reach a network can still be tested end to end: the
`ChatModel` trait means a scripted turn exercises the entire loop — the request body, the tool
dispatch, the result fed back, the final answer — with no credential and no socket.

Two layers hold this up, and the split is the point. `stkcraft-agent-mcp` (L5) is a *transport*: it
turns the registry into a protocol and knows nothing about models. `stkcraft-agent-runtime` (L6) is a
*client* of that protocol — from the server's point of view it is indistinguishable from Claude
Desktop connected to the same stdin. The agent therefore cannot reach a capability the protocol
does not expose, which keeps "the model can do anything the user can" a property of the design
rather than a promise. Swapping `DeepSeek` for another model, or moving the agent into another
process, changes nothing below it; `stkcraft mcp`, `stkcraft ask` and the window's assistant panel
are three clients of one surface.

The panel is the window's client, and it follows the same rule as every other panel here: it owns
no analysis. It asks a question on a worker thread and shows the tool calls as they arrive, because
a question is a network round trip around real engine calls and a frozen clock would be worse than
a failure. The worker gets a copy of the scenario, and the window adopts what the model produced
only when the loop has finished — so an answer describes the scenario it was asked about, and a
question in flight cannot half-edit the document while a person keeps working. Nothing in the
engine knows the panel exists.

The worker lives for the session rather than for one question, because the *conversation* lives on
it: a follow-up can say "and the day after?" only if the agent still holds the scenario, the tool
calls and the answers before it. `stkcraft chat` is the same thing on a terminal — one agent, many
questions — and the history is trimmed at a turn boundary when it outgrows `MAX_HISTORY`, never in
the middle of a tool exchange, because a tool result whose call has been dropped is a request the
model API rejects long after the cause. A second, size-triggered compaction replaces the dropped span
with a summary the model wrote; the two paths are described in `docs/design/knowledge.md`. Deleting
an answer is not the same as forgetting it: the
scenario lives in the server, so a session can be emptied (`Agent::forget`) without losing what the
tools already built.

The model's transport is `curl` on standard input rather than an HTTP crate: no TLS stack in the
build, no client to audit, and the request and response stay text a person can read when a number
looks wrong. That is the same reasoning that makes the bulk data a `curl` download.

### What "plugin" means today, and what it does not

A plugin, at this stage, is a *registration*, not a loaded shared object:

- a **command** is one registry entry (id, label, menu path, parameters, `enabled`, `run`), and
  the menu, shortcut, CLI, control channel, MCP server and agent all dispatch by id;
- a **skill** is one markdown document picked up from `share/skills/`, needing no code and no rebuild;
- a **bulk file** is one row in `cargo xtask fetch-data`, with a checksum recorded in
  `docs/guides/ephemeris-data.md`.

Dynamic third-party code is deliberately *not* built yet. Running someone else's code inside an
analysis process needs a capability story — what a plugin may read, what it may compute, what it
may spend — and a story invented after the fact is a hole. The registry already gives the useful
half: anything expressible as a command or a document is a plugin today.

## Verification

Every numerical layer is finished only when it matches a published reference. The test
vectors live next to the code that uses them, and the constant or formula they verify is
cited in a comment beside it. See `AGENTS.md` for the standard a layer must meet before it
is called done.

`stkcraft-astro-sgp4` is the first layer to meet that standard, and it is worth looking at as the
pattern:

- The verification data is a transcription of the AIAA 2006-6753 archive — the 33 element
  sets and the reference implementation's own output for them — kept in
  `crates/astro/sgp4/src/tests/vectors/data.rs` with its source named.
- The tests compare against that data in two groups, because the two halves of the theory are
  in different states. The near-Earth half is held to `4e-9 km`, which is the limit of what
  a table printed to eight decimals can express; the deep-space half is held to the accuracy
  it actually has today, and `CHANGELOG.md` says plainly that it is not yet exact.
- The tolerances are set to the measured accuracy, never to the intended accuracy, so a
  regression fails the test instead of being absorbed by a loose bound.

A tolerance that a wrong answer can pass is not a test. That is why the deep-space bound is
documented as a limitation rather than presented as a result.
