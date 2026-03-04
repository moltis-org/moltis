# Lazy Tool Registry

Lazy mode reduces the tool context sent to the LLM from thousands of tokens
down to ~120, regardless of how many tools and MCP servers are connected.
Instead of sending every schema on every turn, only a single `tool_search`
meta-tool is exposed. The model discovers and activates tools on demand.

## Quick Start

Add one line to your `moltis.toml`:

```toml
[tools]
registry_mode = "lazy"
```

Restart the server. No other changes are needed — all existing tools, hooks,
and MCP servers continue to work as before.

To switch back to the default mode where all schemas are sent on every turn:

```toml
[tools]
registry_mode = "full"
```

Or simply remove the `registry_mode` line — `full` is the default.

## How It Works

In lazy mode the LLM sees only one tool: `tool_search`. It has two calling
modes:

### 1. Keyword Search

```json
{ "query": "memory" }
```

Returns up to 15 results as `{ name, description }` pairs, sorted by relevance.
No parameter schemas are included — this keeps the response compact.

Scoring: exact name match (100) → substring in name (50) → word overlap with
name or description (10). Minimum query length is 2 characters.

### 2. Exact Name Lookup (Activation)

```json
{ "name": "exec" }
```

Returns the full parameter schema for the named tool **and activates it**.
Once activated, the tool appears in the LLM's tool list on the very next turn
and can be called directly — no proxy, no wrapper.

### Typical Conversation Flow

```
User:   list files in the current directory

Turn 1  model → tool_search(name="exec")
               ↳ returns exec schema, marks it as activated

Turn 2  model → exec(command="ls -la")
               ↳ runner dispatches the call normally (hooks fire as usual)

Turn 3  model → "Here are the files: ..."
```

The extra search turn is transparent to the user. The runner automatically
triples `max_iterations` in lazy mode to account for the search→activate→call
pattern.

## When to Use Lazy Mode

| Scenario | Recommended mode |
|---|---|
| Few built-in tools (< 20) | `full` — the context cost is negligible |
| Many MCP servers, 50+ tools | **`lazy`** — saves thousands of prompt tokens |
| Cost-sensitive deployments | **`lazy`** — fewer input tokens per request |
| Latency-sensitive, simple tasks | `full` — avoids the extra search turn |

## Provider Compatibility

Lazy mode works with all providers (Gemini, Mistral, OpenAI-compatible, etc.).
Provider-specific concerns handled automatically:

- **Gemini**: `thought_signature` metadata is round-tripped through the
  `extra_content` field on `ToolCall`, so thinking-model tool calls work
  correctly.
- **Mistral**: Assistant messages with tool calls always include a `content`
  field (empty string if none), which Mistral's API requires.
- **All providers**: Activated tools are dispatched through the runner's standard
  path — `BeforeToolCall` / `AfterToolCall` hooks fire exactly as in full mode.

## Architecture

### Activation Model

When the model calls `tool_search(name="exec")`:

1. `ToolSearchTool.execute()` looks up `exec` in the full (hidden) registry.
2. Returns the full schema to the model.
3. Inserts the tool into the `ToolRegistry`'s shared `activated` map.
4. On the next loop iteration, `list_schemas()` includes the activated tool.
5. The model calls `exec(...)` directly — the runner's `get_arc()` finds it in
   the `activated` map and dispatches normally.

### Why Not a Proxy?

An earlier approach used a `tool_use` meta-tool that would forward calls to real
tools. This is fundamentally broken: models issue `tool_search` and `tool_use`
as **parallel tool calls** in the same batch. The proxy fires before the search
result arrives, so it always receives empty or wrong arguments.

The activation model avoids this entirely — after activation, the real tool is
called directly with its own schema.

### Schema Recomputation

`list_schemas()` is called inside the runner loop on every iteration. This means
activated tools appear in the next API call immediately. The cost is one
`HashMap` walk per turn — negligible.

### Hook Enforcement

Because activated tools are called directly (not proxied), the runner's existing
hook dispatch (`BeforeToolCall` / `AfterToolCall`) fires automatically. No
special lazy-mode handling is needed.

### Memory Integration

When lazy mode is detected, the system prompt adjusts memory guidance
automatically: it instructs the model to call `tool_search` to activate memory
tools before using them.

## Configuration Reference

```toml
[tools]
# How the tool registry is exposed to the LLM.
# "full"  — send all tool schemas on every turn (default)
# "lazy"  — expose only tool_search; tools are activated on demand
registry_mode = "lazy"

# Maximum agent loop iterations (default: 25).
# In lazy mode this is automatically multiplied by 3 internally.
agent_max_iterations = 25
```
