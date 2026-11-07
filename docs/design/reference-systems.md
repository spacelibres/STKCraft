# Reference systems: PhyE2E and CosmoAgent

This note records what STKCraft learns from two published systems, what it deliberately does not
copy, and where each lesson lands in this repository. It is a design note, not a claim that either
system has been integrated. The lasting boundary is `docs/design/agents.md`; the implemented
state is `CHANGELOG.md`.

## PhyE2E

PhyE2E is the neural-symbolic symbolic-regression framework described in "A Neural Symbolic Model
for Space Physics" (*Nature Machine Intelligence*, 2025). It derives interpretable formulas from
observational data by combining a Transformer, LLM data augmentation, Monte Carlo tree search and
genetic refinement. Its formula-decomposition module uses the Hessian of a learned model: when
second-order partial derivatives between variables are near zero, it treats them as independent
and splits a complex equation into independently solvable sub-equations.

Sources:

- <https://www.nature.com/articles/s42256-025-01126-3>
- <https://www.ee.tsinghua.edu.cn/en/info/1062/2573.htm>

### What is worth learning

| Mechanism | Why it matters | STKCraft translation |
|---|---|---|
| Formula decomposition | Search over one large expression is expensive and brittle; independent variable groups can be solved separately | A formula record may name its independent sub-expressions; the expert validator checks that each referenced sub-formula exists |
| Dimensional consistency as a hard constraint | A fit that is numerically close but dimensionally wrong is not a physical law | Formula records must carry symbols with units; a dimension checker rejects a candidate whose unit algebra does not close |
| Decompose, then fit | Separating structure from coefficients reduces the search space and makes the result explainable | Offline formula discovery produces candidates per sub-expression, not one opaque model |
| Held-out evaluation | Fitting the data used to propose the formula is not evidence | Every `verified` formula needs a reference value or a held-out case with a stated tolerance |
| Explicit physical meaning | A formula that cannot be read by a domain expert is not yet knowledge | `EXPERT.md` and `limitations.md` must state the assumptions and the regime of validity |

### What not to copy

- Do not put a neural symbolic-regression loop in the runtime path. A mission answer must be
  reproducible from a pinned formula and its inputs, not from a search that may return a different
  expression tomorrow.
- Do not let a language model invent a constant, a unit or a coefficient. A discovered expression
  is a **draft** until a reference or an independent data set verifies it.
- Do not claim "new physics" from a fit. The acceptance criterion remains a published reference,
  a conservation law or a held-out measurement.

### Where it lands

1. `share/experts/*/formulas/*.toml` gains optional `decomposition` and `dimensions` fields, checked
   by `cargo xtask experts check`.
2. A candidate formula stays `extracted` or `classified` until a held-out case and a reference are
   recorded; only then can it become `verified`.
3. Formula discovery is an offline workflow (`cargo xtask` or an expert workspace), not a product
   command.

## CosmoAgent

CosmoAgent is the LLM multi-agent simulation in "What if LLMs Have Different World Views:
Simulating Alien Civilizations with LLM-based Agents" (arXiv:2402.13184). Each civilization has a
`cosmo_agent` that proposes a political system, an action and a state-transition matrix, and a
`secretary_agent` that validates the proposal against rationality constraints and a fixed response
format. A rejected proposal is regenerated; after three consecutive rejections the system falls
back to the previous state. A "stick" stores the resource vector, transition matrix, political
system and actions by round; a relationship map records the state of inter-civilization links.

Sources:

- <https://arxiv.org/abs/2402.13184>
- <https://github.com/MingyuJ666/Simulating-Alien-Civilizations-with-LLM-based-Agents>

The name is ambiguous: a separate 2026 arXiv project also uses "CosmoAgent" for theoretical
cosmology. This note studies the multi-agent paper above, because its verifier/blackboard pattern is
the one that maps to STKCraft. The other project should be reviewed separately if it is in scope.

### What is worth learning

| Mechanism | Why it matters | STKCraft translation |
|---|---|---|
| Proposer and verifier are separate roles | The model that produced a claim is the worst judge of it | A deterministic result verifier checks the structured answer; the model's prose is not the evidence |
| Enumerated action space | A free-form action is hard to validate and impossible to replay | Commands and capability manifests declare their parameters; unknown actions fail before dispatch |
| Rationality constraints | Shape and value constraints catch meaningless output before it changes state | Window durations must match their instants; a result without formula citations or limitations is refused |
| Bounded rejection and fallback | An agent can loop forever trying to satisfy an impossible request | A verification failure is reported as `failed`; a missing capability is `unsupported`; no silent retry loop |
| Blackboard / "stick" | Later steps need the exact inputs and results of earlier steps | A computation ledger records the command, input digest, engine version, result and errors |
| Relationship/state maps | The system needs to see the state it is acting on, not a summary of it | The scenario is the authoritative state; the transcript is not the workflow database |
| Counterfactual evaluation | A single run does not show whether the decision mattered | Fixed cases compare the answer with and without a parameter or a data source |

### What not to copy

- Do not make a model the final authority on a number. The secretary's checks are deterministic;
  STKCraft's equivalent is Rust, not a second model asked to agree.
- Do not add a second JSON state file for each role. `docs/design/storage.md` owns persistence.
- Do not turn every object into a separate model process. A role invocation is a task; many tasks
  share one provider connection.

### Where it lands

The first slice is implemented in `stkcraft-agent-runtime::verify`: `verify_results` checks the
structured access and pass results, and `ResultEnvelope` reports `complete`, `unsupported` or
`failed` with the failed invariants. The model's text is not checked as evidence.

## Gap matrix

| Gap in STKCraft | Reference lesson | Next action | State |
|---|---|---|---|
| A model can describe a number the tool did not return | CosmoAgent's secretary | Deterministic result verifier | Implemented |
| A failed result has no status | Separate approval/rejection | `ResultEnvelope.errors` and `failed` status | Implemented |
| A capability gap looks like a complete answer | CosmoAgent's checked action space | `agent.report` status plus a known-domain guard in `share/capabilities/limits.toml` that runs before the model | Known domains are deterministic; general request-to-capability preflight remains |
| No capability manifest per object | Enumerated action space | Satellite/station/target/scenario/planet manifests, validated against the command registry and exposed as an MCP resource | Declared; role-scoped dispatch is enforced, a session without a role is not |
| No task ledger | "Stick" | `Agent::with_ledger` records command, parameters, scenario digest, engine version, result and verifier errors; the CLI `passes` path stores its structured answer too | Implemented; best-effort when the record opens |
| Formula records have no dimension algebra | PhyE2E dimensional consistency | Known unit/dimension pairs must agree in the expert validator | Pair check implemented; expression-level balance and decomposition remain |
| Formula discovery is not a workflow | PhyE2E data-to-law pipeline | Offline candidate pipeline with held-out evaluation | Later |
| No planner/executor | Proposer/verifier separation | Deterministic `TaskGraph` executor; `Planner` asks a model for a graph, `stkcraft plan` validates and runs it | Executor and one-shot planner implemented; correction, budgets, cancellation and durable state remain |
| Agent evaluation is ad hoc | Prophecy/counterfactual evaluation | Fixed JSONL cases for complete/needs-input/unsupported/failed | In progress |

## Delivery order and acceptance

1. **Result verifier.** Done when a duration/instant mismatch, a missing formula or a missing
   limitation makes the envelope `failed`, and a consistent access result stays `complete`.
2. **Object capability manifests.** Done when a satellite, station and target each declare their
   verbs, parameter schema, allowed tools, evidence and limitations, and an unknown verb fails
   closed.
3. **Computation ledger.** Done when every displayed result can be traced to a command, a fixed input
   digest and an engine version without reading the transcript.
4. **Formula dimensions and decomposition.** Done when a formula with an unbalanced unit expression
   is rejected and a decomposed formula's sub-expressions are checked.
5. **Offline formula discovery.** Done when a candidate is evaluated on a held-out case, carries its
   data and code provenance, and remains a draft until a reference verifies it.
6. **Task graph.** Done when dependencies, budgets, cancellation and outcome classes are tested, and
   a role cannot call a tool outside its allowlist.

The order is deliberate: verify the numbers first, then describe the objects, then let a planner
compose them. A planner over unverified numbers only makes wrong answers faster.
