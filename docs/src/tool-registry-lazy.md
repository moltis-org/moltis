# Lazy Tool Registry

## Problem

When dozens of MCP servers are connected, every tool schema is sent to the LLM
on each turn. With hundreds of tools this consumes thousands of tokens in the
system prompt (native tool-calling) or in embedded text-mode schemas, degrading
response quality and increasing latency/cost. The model rarely needs more than
two or three tools per turn, yet it pays the context tax for all of them.

A proxy-based approach (a single `tool_use` meta-tool that forwards calls) was
also attempted but is fundamentally broken: models issue `tool_search` and
`tool_use` as **parallel tool calls** in the same batch, so `tool_use` always
fires with empty arguments before `tool_search` has returned the schema.

## Solution

A new `ToolRegistryMode::Lazy` configuration option exposes only **one**
lightweight meta-tool in the initial prompt:

| Tool | Purpose | Token cost |
|---|---|---|
| `tool_search` | Discover tools by keyword or activate one by exact name | ~120 tokens |

Total prompt overhead: **~120 tokens** regardless of how many real tools exist,
versus potentially 10,000+ tokens in `Full` mode with many MCP servers.

### Workflow

```
Turn 1  model → tool_search(name="exec")
               ↳ returns full schema, marks "exec" as activated
Turn 2  model → exec(command="ls -la")      ← direct call, no proxy
               ↳ runner dispatches hooks normally
```

After `tool_search` activates a tool it is added to the `ToolRegistry`'s
`activated` map. Because `list_schemas()` is called **inside the runner loop**
on every iteration, the activated tool schema appears in the next turn's API
call and the model can invoke it directly — with correct arguments and full hook
enforcement.

## Changed Files

### `crates/agents/src/lazy_tools.rs`

Core implementation of the lazy tool registry pattern.

**`ToolSearchTool`** — implements `AgentTool` with two modes:
- **Keyword search** (`{ "query": "memory" }`): returns up to 15 results as
  `{ name, description }` pairs sorted by relevance score (exact match → 100,
  substring → 50, word overlap → 10). No parameter schemas are included.
- **Exact name lookup** (`{ "name": "exec" }`): returns the full parameter
  schema for a single tool **and inserts it into the shared `activated` map**
  so the lazy registry exposes it natively on the next turn.

**`wrap_registry_lazy(full) → ToolRegistry`** — wraps a filtered registry
behind `tool_search`. Clones the registry's `activated` Arc into
`ToolSearchTool` so activations are shared.

**Tests** (9 test cases):
- `wrap_registry_lazy_produces_one_tool` — verifies only `tool_search` is exposed initially
- `search_keyword_returns_name_and_description_only` — no schema leaks
- `search_scores_exact_match_highest` — ranking correctness
- `search_multiword_query` — word-overlap scoring
- `search_short_query_rejected` — minimum query length guard
- `search_exact_name_returns_full_schema` — schema retrieval
- `search_exact_name_activates_tool` — verifies the `activated` map is populated
- `search_unknown_name` — graceful error
- `search_result_stays_compact_with_many_tools` — 200 tools, ≤15 returned,
  <600 estimated tokens

### `crates/agents/src/tool_registry.rs`

- Added `pub(crate) activated: Arc<Mutex<HashMap<String, Arc<dyn AgentTool>>>>`
  field to `ToolRegistry`.
- `get_arc()` checks the `activated` map as a fallback after `tools`.
- `list_schemas()` appends activated tool schemas (tagged `"source":
  "activated"`) after the static schemas.
- All four `clone_*` methods initialise `activated` as a fresh empty map so
  clones start without inherited activations.

### `crates/agents/src/runner.rs`

Both `run_agent_loop_with_context` and `run_agent_loop_streaming`:
- Apply a ×3 multiplier to `max_iterations` in lazy mode (search → activate →
  call direct = 3 turns vs 1).
- **Moved `tool_schemas`/`schemas_for_api` inside the loop** (recomputed on
  every iteration). This is what makes activated tools visible to the LLM on
  the very next API call after `tool_search` fires.

### `crates/chat/src/lib.rs`

Integration point in `run_with_tools()`:

```rust
let filtered_registry = if tools_enabled
    && config.tools.registry_mode == ToolRegistryMode::Lazy
{
    wrap_registry_lazy(filtered_registry)
} else {
    filtered_registry
};
```

Hooks are no longer wired here — `BeforeToolCall` / `AfterToolCall` fire
naturally through the runner's dispatch path when the model calls the real tool
directly.

### `crates/agents/src/prompt.rs`

`append_memory_section()` detects lazy mode by checking for a `tool_search`
schema and adjusts the memory guidance:
- In lazy mode: instructs the model to call `tool_search` to activate memory
  tools before using them.
- In lazy mode: suppresses the standalone `memory_save` paragraph (redundant
  since the tool is discoverable via search).
- Preserves the "always search memory before claiming ignorance" instruction.

### `crates/web/src/assets/js/helpers.js`

`toolCallSummary()` recognises `tool_search` for the UI tool-call cards:
- `tool_search` with `name` field displays `tool_search <name>`.
- `tool_search` with `query` field displays `tool_search "<query>"`.

## Architecture Decisions

1. **Activation model over proxy model.** A proxy (`tool_use`) that wraps real
   tools breaks when models issue parallel tool calls — the proxy fires with
   empty arguments before the search result arrives. The activation model avoids
   this: once activated, the tool is called directly with its own schema.

2. **Schemas recomputed per iteration.** Placing `list_schemas()` inside the
   loop is the minimal correct fix. The cost is negligible (one HashMap walk per
   turn) but it means activations are immediately visible to the provider API.

3. **Hook enforcement through the runner, not a proxy.** Because the model calls
   the real tool directly, the runner's existing `BeforeToolCall` /
   `AfterToolCall` hook dispatch fires exactly as in `Full` mode. No special
   lazy-mode handling is needed in the runner.

4. **No schema leakage.** Keyword search returns only `{ name, description }`.
   Full schemas are served exclusively via exact-name lookup, one tool at a
   time. This is verified by the `search_keyword_returns_name_and_description_only`
   and `search_result_stays_compact_with_many_tools` tests.

5. **Backward compatible.** `Full` is the default `ToolRegistryMode`. Existing
   configurations require zero changes. The `registry_mode` field is omitted
   from serialised config when set to default.

## Configuration

Add to `moltis.toml`:

```toml
[tools]
registry_mode = "lazy"
```

No other configuration changes are required.
