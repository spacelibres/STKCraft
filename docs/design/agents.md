# Specialist agents

STKCraft's agent programme has two execution domains: mission analysis in the application and
software development in isolated source workspaces. DeepSeek is the primary reasoning provider;
Rust validates requests, executes numerical algorithms, schedules work and accepts state changes.
This is an implementation direction, not a claim that a multi-agent executor already ships.
The lessons taken from the published PhyE2E and CosmoAgent systems, and what is deliberately not
copied, are recorded in `docs/design/reference-systems.md`.

## Existing foundation

The engine's command registry defines mission capabilities. MCP exposes that registry and the
knowledge library, while the agent runtime drives the protocol. Skills are lazily loaded documents.
The model remains outside the numerical kernel, and the viewport displays engine-produced positions.

The runtime currently runs one model/tool loop. Its ability to issue several tool calls is not a
multi-agent scheduler. Specialist roles, task graphs, durable recovery and global budgets remain
separate milestones below. Do not infer their availability from the existence of the MCP server.

## One role, one contract

A role definition consists of its versioned identity, one responsibility, input/output contracts,
allowed tools, required evidence, resource limits and an evaluation set. An invocation is a task
instance, not a new permanent model process. Many roles can share one provider connection.

Use a deterministic component when a job has a deterministic implementation. Propagation,
reference-frame conversion, visibility geometry, validation, aggregation and citation-location
checks do not become language-model reasoning tasks. Models interpret requests, find and extract
evidence, propose plans and explain verified results.

An agent's role prompt is not evidence of expertise. Every enabled specialist needs rejection
conditions and tests with expected outcomes, including missing evidence and unsupported inputs.

## Object capabilities

`share/capabilities/<object>.toml` declares what a satellite, station, target, scenario or planet
may do: the command ids, the evidence a result must carry and the limitations it must state. The
manifest is not a second command registry. `cargo xtask capabilities check` requires every command
id to exist in the engine registry, and the engine's parameter schema remains the only schema. An
MCP client reads the manifests at `stkcraft://capabilities`.

A role may name `objects` in its `role.toml`. The MCP dispatch then requires the tool to be both in
the role's tool list and declared by one of those object manifests; `knowledge.*` reads remain
available. A role with objects and no loaded manifests fails closed for object commands. A session
without a role is still governed by the role/tool allowlist alone, so the manifest remains a
declaration there, not an authorization boundary.

## Product execution domain

The initial mission workflow is:

1. Interpret the request and identify missing requirements.
2. Search the user's documents and retain exact passages and place citations.
3. Extract parameters with units and sources; distinguish explicit values from assumptions.
4. Propose a plan using registered capabilities.
5. Validate proposed scene changes and execute the engine's deterministic commands.
6. Check output provenance and stated limitations.
7. Explain the result with document citations and computation references.

Product roles get only their tool allowlists. A hidden tool must also be refused at dispatch;
filtering the tool description alone is not authorization. No product agent receives arbitrary
shell, Git, file-write or software-development capabilities.

The first pilot is a document-backed satellite-to-station access report. Geometry visibility
does not establish radio-link availability. The existing target analysis is a union of vertex
windows, not complete polygon-interior coverage. Point sampling can miss a window opening and
closing between samples; a boundary tolerance is not a completeness guarantee. These limits
must travel with the answer rather than being erased by a specialist's name.

## Development execution domain

Development roles produce reviewable source changes in isolated workspaces:

| Responsibility | Deliverable |
|---|---|
| Requirements | Scope, non-goals and acceptance cases |
| Public-reference research | Exact source, version, assumptions and reference values |
| Numerical verification | Independent expected results and error bounds |
| Implementation | A bounded change in an assigned module |
| Test design | Behavioural cases able to expose an incorrect implementation |
| Review | Evidence-backed defects and regression risks |
| Documentation | Current behaviour and explicit limitations |
| Integration | Serialized conflict resolution, quality gates, version and release |

The implementing model cannot certify its own physical assumptions by agreement with another
model. Published references remain the acceptance criterion. Numerical and interface drawing
verification retain the standards in AGENTS.md.

Development workers do not inherit mission documents or provider credentials. Maintenance commands that a
maintainer runs, such as `cargo xtask i18n translate`, are not development workers: they run on demand, under
the maintainer's own key from the environment, and they are not agent runs. Their repository
root, writable files and command capabilities are explicit. An agent's worktree is an isolation
boundary for changes; it must not be described as a security sandbox without an actual sandbox.

Model transport is the exception to the retry policy in "Task lifecycle and resource policy" below. A model request is never retried automatically,
because a retried request can run a tool twice and cost twice; see `docs/guides/model-provider.md`. Retrying
it is an explicit user action.

## Shared infrastructure and separate authority

Provider adapters, task-envelope formats, scheduling primitives, tracing and evaluation machinery
can be shared. Product and development runs retain distinct tools, credential access, queues,
state and acceptance policies. Neither execution domain can grant itself new capabilities.

Only one component accepts changes to a live scenario. Workers read a fixed base and return
proposed changes. Before adopting a result, check that the base still matches current state;
a conflict preserves the newer document and is reported. Content comparison is an initial
optimistic guard, not persisted revision tracking or automatic merge.

Keep facts outside conversation summaries. User-provided values, document evidence, engine
results, model inferences and pending assumptions must remain distinguishable. Summaries are
navigation aids; authoritative parameters and results need structured records and identifiers.

The computation ledger is the structured record for this rule: a tool call stores its command,
parameters, scenario digest, engine version, result JSON and verifier errors. A ledger write
failure is reported but does not turn a valid answer into an error; the transcript is still not the
workflow database.

`stkcraft agent-eval` runs the fixed cases in `share/evals/agent/` against a real provider. It is a
manual, networked check, not a CI step: a case records the expected status, result kind, window
count and formula citation, and a failure stays visible instead of being relaxed in the fixture.
The first recorded failure was a capability gap: an RF link-budget question produced access windows
and a `complete` envelope because "some structured result exists" is not "the question was
answered". The model now concludes with `agent.report`; the real run reports `unsupported` for that
case and passes both fixed cases. The status is still model-declared, so a deterministic capability
preflight remains a milestone; a model that ignores the tool falls back to inference.

## Task lifecycle and resource policy

The proposed task envelope names the task/parent, role version, input-schema version, fixed
scenario or artifact references, allowed tools, deadline, model/tool budgets and idempotency key.

Outcomes distinguish completed, needs-input, unsupported, failed and cancelled. Empty access
windows, an unevaluable geometry and a failed computation are different outcomes. A completed
artifact carries evidence references, input identity, computation identity and limitations.

A Rust executor validates dependency graphs before execution, bounds concurrency and task depth,
and maintains the whole-request budget. Retry only classified transient failures; parameter
correction is bounded, while unsupported input and absent evidence must not trigger endless
retries. Mutating calls need idempotency handling before retry. Cancellation propagates through
model requests, pending tasks and tool boundaries.

Durable task transitions and authoritative artifacts belong behind framework/storage, following
storage.md. A transcript is not the workflow database; a cached result is not an authoritative
record. Do not add independent JSON state files as each role is introduced.

## Delivery order and evidence

| Stage | Acceptance |
|---|---|
| Development inputs | All four pinned manifests verified; real-kernel, formula and retrieval checks |
| Response boundary | Invalid model calls rejected before dispatch; Unicode errors cannot panic |
| Knowledge and state | CLI/window library parity; stale assistant results cannot overwrite edits |
| Provider protocol | Explicit thinking modes, preserved protocol fields and compatible session resume |
| Provider resource handling | Output/usage contract, bounded subprocess I/O, errors and cancellation |
| Role/task contracts | Invalid input, unauthorized tools and unsupported tasks fail closed |
| Controlled executor | Tested dependencies, budgets, cancellation, recovery and serialized writes |
| Product pilot | One document-backed mission workflow with engine-equivalent results |
| Development pilot | One isolated source change with independent verification and full integration gate |
| Role expansion | Measured improvement over the simpler workflow on quality, cost or latency |

Download development inputs with `cargo xtask fetch-data`, `papers`, `standards` and `naif`.
Bulk files stay in ignored directories. Mathlib and craft-fonts are optional external checkouts;
record their actual revisions, and do not invent a historical pin when none was recorded.
ECSS and real-engine OCR requirements remain the explicit acquisition/tooling gaps documented
in guides/sources.md and the relevant corpus guides.

Detailed local execution steps and evidence live in the ignored plan/ directory. This document
records the lasting boundaries; ROADMAP.md and CHANGELOG.md record what has actually shipped.
