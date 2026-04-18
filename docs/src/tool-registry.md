# Tool Registry

The tool registry manages all tools available to the agent during a
conversation. It tracks where each tool comes from and supports filtering by
source.

## Tool Sources

Every registered tool has a `ToolSource` that identifies its origin:

- **`Builtin`** — tools shipped with the binary (exec, web_fetch, etc.)
- **`Mcp { server }`** — tools provided by an MCP server, tagged with the
  server name
- **`Wasm { component_hash }`** — tools provided by a precompiled WASM component

This replaces the previous convention of identifying MCP tools by their
`mcp__` name prefix, providing type-safe filtering instead of string matching.

## Registration

```rust
// Built-in tool
registry.register(Box::new(MyTool::new()));

// MCP tool — tagged with server name
registry.register_mcp(Box::new(adapter), "github".to_string());

// WASM component tool
registry.register_wasm(Box::new(wasm_tool), component_hash);

// Replace existing tool (preserves source metadata)
let replaced = registry.replace(Box::new(UpdatedTool::new()));

// Remove by name
let existed = registry.unregister("some_tool");
```

## Filtering

When MCP tools are disabled for a session, the registry can produce a filtered
copy:

```rust
// Type-safe: filters by ToolSource::Mcp variant
let no_mcp = registry.clone_without_mcp();

// Remove all MCP tools in-place (used during sync)
let removed_count = registry.unregister_mcp();

// Clone excluding tools by name
let filtered = registry.clone_without(&["browser", "sandbox_exec"]);

// Clone excluding tools by name prefix
let no_mcp_prefix = registry.clone_without_prefix("mcp__");

// Clone keeping only tools matching a predicate
let allowed = registry.clone_allowed_by(|name| allowed_set.contains(name));
```

## Schema Output

`list_schemas()` includes source metadata in every tool schema:

```json
{
  "name": "exec",
  "description": "Execute a command",
  "parameters": { ... },
  "source": "builtin"
}
```

```json
{
  "name": "mcp__github__search",
  "description": "Search GitHub",
  "parameters": { ... },
  "source": "mcp",
  "mcpServer": "github"
}
```

```json
{
  "name": "my_wasm_tool",
  "description": "A WASM tool",
  "parameters": { ... },
  "source": "wasm",
  "componentHash": "a1b2c3..."
}
```

The `source` and `mcpServer` fields are available to the UI for rendering
tools grouped by origin.

## Lazy Registry Mode

By default every LLM turn includes full JSON schemas for all registered tools.
With many MCP servers this can burn 15,000+ tokens per turn. **Lazy mode**
replaces all tool schemas with a single `tool_search` meta-tool that the model
uses to discover and activate tools on demand.

### Configuration

```toml
[tools]
registry_mode = "lazy"   # default: "full"
```

### How it works

1. The model receives only `tool_search` in its tool list.
2. `tool_search(query="memory")` returns name + description pairs (max 15), no schemas.
3. `tool_search(name="memory_search")` returns the full schema and **activates** the tool.
4. On the next iteration the model calls `memory_search` directly — standard pipeline, hooks fire normally.

The runner re-computes schemas each iteration, so activated tools appear
immediately. The iteration limit is tripled in lazy mode to account for the
extra discovery round-trips.

### When to use

- Many MCP servers connected (50+ tools)
- Long conversations where input token cost matters
- Sub-agent runs that only need a few specific tools

In **full** mode (default), all schemas are sent every turn — no behavioral change from before this feature.
