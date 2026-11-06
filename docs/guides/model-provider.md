# Configure the model protocol

The shipped DeepSeek connection uses `deepseek-flash` with thinking explicitly disabled.
Enable thinking when a task needs it:

```sh
stkcraft model --protocol deepseek --thinking enabled
stkcraft model --thinking disabled
```

These commands change local settings; they do not submit a model request. Existing explicitly saved
model names and endpoints are preserved. The credential remains separate from the preferences file
and is not printed by `stkcraft model`.

## Configuration precedence

Each setting resolves saved preferences first, then environment, then the shipped default.

| Setting | CLI | Environment | Preference field |
|---|---|---|---|
| Endpoint | `--endpoint URL` | `DEEPSEEK_BASE_URL` | `model_endpoint` |
| Model | `--name MODEL` | `DEEPSEEK_MODEL` | `model` |
| Protocol | `--protocol deepseek\|compatible` | `DEEPSEEK_PROTOCOL` | `model_protocol` |
| Thinking | `--thinking enabled\|disabled` | `DEEPSEEK_THINKING` | `model_thinking` |
| Output tokens | `--max-output-tokens N` | `DEEPSEEK_MAX_OUTPUT_TOKENS` | `model_max_output_tokens` |

Unknown protocol or thinking values are errors. Thinking enabled requires the DeepSeek protocol.
Output caps must be positive integers. The configured DeepSeek default is 8192 generated tokens,
including thinking tokens; it is an application policy, not a claim about a model's maximum.
Generic connections omit the cap unless explicitly configured. For example:

```sh
stkcraft model --max-output-tokens 16384
```
Changing these options validates the proposed settings before saving them, without needing a key.
Malformed saved JSON or wrong-type fields also fail model operations; repair the saved settings
before continuing. The window can open with default chrome while preserving that invalid file.
Preference files are limited to 64 KiB and must be regular files; UI fallback settings cannot
overwrite a malformed saved connection.

The exact shipped endpoint selects DeepSeek defaults. A custom gateway defaults to the original
compatible request shape; a model name containing "DeepSeek" does not grant protocol capabilities.
Opt in explicitly when a gateway supports the extensions:

```sh
stkcraft model --endpoint https://gateway.example/v1/chat/completions --protocol deepseek --thinking enabled
```

For an ordinary compatible service, select `--protocol compatible --thinking disabled`. Compatible
requests do not receive DeepSeek thinking or reasoning fields. The Rust `Provider::new` constructor
also keeps that generic behaviour; extension-aware callers resolve and attach `ProviderOptions`.

## Tool loops and resumed conversations

DeepSeek thinking tool exchanges carry opaque `reasoning_content`. The runtime preserves it on tool
and final assistant turns, writes it to the local session, and replays it for retained DeepSeek
messages. It is not an answer, a citation, an authoritative fact or summary material; debug/error
reports and user-visible traces do not print that field.

An enabled-thinking tool response missing the required field is rejected before dispatch. An old
conversation whose tool messages never recorded the field cannot reconstruct it: start a new
conversation or explicitly select disabled mode. Existing records without the optional field still
load for compatible/disabled operation. This protocol compatibility does not promise restoration of
an entire workflow or scenario from chat text; task persistence remains a separate milestone.

Reasoning contributes to the context-size heuristic. The current character budget is a heuristic,
not an exact model-token count. Developer translation tasks use the same provider with explicit
ordinary mode for their single JSON completion, retaining their 300-second timeout.

## Verification and limits

Protocol, session and configuration tests use scripted models and serialized requests. They require
no credential and do not establish live account availability or real-task model quality.

Assistant turns retain optional provider-reported usage and completion status in the session. Usage
is telemetry: missing usage remains unknown, and reported zero remains zero. Cache/reasoning
breakdowns are optional; reasoning tokens are already included in completion tokens. Metadata is
not replayed as request messages. `finish_reason=length` fails before any tool dispatch or successful
answer, so a truncated response cannot silently change a scenario.

`Answer::usage` and `Agent::usage()` expose reported sums, unknown requests, failed attempts and
overflow. Compaction requests have a separate aggregate, including failed summary attempts; sums
cover only reported requests and must not be presented as a complete cost when usage is missing.
Supervised bounded/cancellable transport is the next stage in
[the agent programme](../design/agents.md).

Provider behaviour was checked against the official [thinking-mode guide](https://api-docs.deepseek.com/guides/thinking_mode/)
and [chat-completions contract](https://api-docs.deepseek.com/api/create-chat-completion/)
on 2026-11-07. Names and API
requirements can change; these references govern provider extensions, not the numerical kernel.
