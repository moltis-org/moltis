# Plan: fix tool-argument serialization regression (`#693`)

## Context

Issue [#693](https://github.com/moltis-org/moltis/issues/693) reports that
`v20260413.01` regressed tool calling for falsy and null values:

- integers like `0` arrive as strings like `"0"`
- booleans like `false` arrive as strings like `"false"`
- nullable fields like `null` are mishandled

The failures span multiple tools (`exec`, `Edit`, `Grep`, `cron`, and others),
which strongly suggests the bug is in a shared tool-call conversion path rather
than inside a single tool implementation.

## Goal

Restore end-to-end preservation of native JSON argument types for tool calls,
especially:

- `0`
- `false`
- `null`

The fix should cover both native tool-calling providers and text/XML fallback
tool parsing so the same regression cannot reappear on another provider path.

## Verified Constraints

- `#693` is specifically about `v20260413.01`
- the same-day compatibility fix PR `#696` improved tool-call handling, but it
  does not clearly prove the broad falsy/null serialization bug is fixed
- between `20260410.01` and `20260413.01`, the most obvious shared regression
  candidate in the agent layer is `crates/agents/src/tool_parsing.rs`
- likely shared boundaries for type corruption are:
  - `crates/agents/src/tool_parsing.rs`
  - `crates/agents/src/model.rs`
  - `crates/providers/src/openai.rs`
  - `crates/providers/src/openai_compat.rs`

## Likely Root Cause Areas

### 1. Text/XML fallback parsing

`crates/agents/src/tool_parsing.rs` is the main parser for non-native
tool-calling output. Any path there that treats raw values as strings too early
can easily convert `false`, `0`, or `null` into string literals.

Relevant code:

- `parse_param_value()`
- Zhipu/Z.AI XML parsing
- invoke/function-style XML parsing
- bare-JSON fallback parsing

### 2. Message history serialization

`crates/agents/src/model.rs` serializes assistant tool calls back into OpenAI
style chat history by storing `arguments` as a JSON string. That can be valid
for wire compatibility, but it is a dangerous boundary if any downstream parser
or repair path treats that string as plain text rather than canonical JSON.

### 3. OpenAI-compatible provider adapters

The issue was reproduced on models like `glm-5.1` and `qwen3.5-plus`, which
likely flow through OpenAI-compatible adapters. The shared adapter code in
`openai.rs` and `openai_compat.rs` should be treated as suspect until proven
otherwise.

## Implementation Strategy

### Phase 1. Reproduce with focused regression coverage

Add a narrow regression matrix that proves the failing shapes before changing
code. Prefer end-to-end tests over mocks.

Minimum cases:

- `exec` with `timeout: 0`
- `Edit` with `replace_all: false`
- `Grep` with `-i: false`, `multiline: false`, `offset: 0`, `type: null`
- `cron` with `force: false`, `limit: 20`
- one success case where `true` still remains `true`

Where to add tests:

- agent/provider-level tests for parsed `ToolCall.arguments`
- shared parser tests in `crates/agents/src/tool_parsing.rs`
- adapter tests in `crates/providers/src/openai.rs` and
  `crates/providers/src/openai_compat.rs`

The test objective is simple: assert the resulting `serde_json::Value` contains
native JSON values, not strings.

## Phase 2. Find the first bad conversion boundary

Trace the exact boundary where types are lost. Do not patch multiple layers at
once until the first corrupting step is identified.

Inspection order:

1. provider response parsing
2. text/XML fallback parsing
3. message history serialization and reparse
4. any tool-call recovery or sanitizer path

For each boundary, inspect:

- whether raw JSON is parsed with `serde_json::from_str`
- whether values are wrapped into `Value::String` too early
- whether malformed fallback logic silently substitutes `{}` or string payloads
- whether prompt or compact-schema rendering causes the model to emit the wrong
  shape in the first place

## Phase 3. Patch only the real coercion site

Once the first bad boundary is confirmed:

- preserve native `serde_json::Value` types through that boundary
- avoid stringifying values unless the external wire format strictly requires it
- if the wire format requires stringified JSON, ensure reparsing is canonical
  and does not route through ad hoc text parsing

Important rule:

- do not add per-tool bandaids for `false`, `0`, or `null`
- do not special-case only `cron`
- fix the shared conversion path so all tools benefit

## Phase 4. Audit sibling adapters

After the primary fix, audit equivalent logic in:

- `crates/providers/src/openai.rs`
- `crates/providers/src/openai_compat.rs`
- `crates/agents/src/model.rs`
- `crates/agents/src/tool_parsing.rs`

The goal is to ensure the same bug is not duplicated in parallel code paths.

## Acceptance Criteria

The work is complete when all of the following are true:

- `timeout: 0` stays numeric end-to-end
- `replace_all: false` stays boolean end-to-end
- `offset: 0` stays numeric end-to-end
- `type: null` is preserved or correctly treated as absent, depending on schema
- no failing path shows `"0"`, `"false"`, or `"null"` where native JSON types
  are expected
- native tool-calling and text/XML fallback tests both pass

## Validation

Run targeted tests first:

```bash
cargo test -p moltis-agents tool_parsing
cargo test -p moltis-agents tool_arg_validator
cargo test -p moltis-agents
cargo test -p moltis-tools cron_tool
```

Then run the relevant provider-focused tests for OpenAI-compatible parsing:

```bash
cargo test -p moltis-providers openai
cargo test -p moltis-providers openai_compat
```

Before handoff, also run the repo-required Rust gates for touched code paths:

```bash
just format
cargo clippy --all --benches --tests --examples --all-features
```

If this work lands in a PR, finish with the normal local validation flow that
matches CI.

## Risks

- Fixing only schema display or prompt text may reduce repro frequency without
  fixing the underlying type corruption
- Fixing only `cron` will miss the shared regression, because `#693` spans
  multiple unrelated tools
- Overeager fallback parsing can hide corruption by converting invalid input
  into `{}` and making the real bug harder to observe

## Suggested PR Scope

Keep the PR narrowly scoped to:

- reproduction coverage
- one shared serialization/parsing fix
- adjacent adapter audit only where needed for correctness

Do not mix in unrelated tool-schema cleanups or compatibility refactors unless
they are required for the regression fix.
