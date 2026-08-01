# Tool Registry

The tool registry manages all tools available to the agent during a
conversation. It tracks where each tool comes from, which audience may use it,
and supports filtering by both properties.

## Tool Sources

Every registered tool has a `ToolSource` that identifies its origin:

- **`Builtin`** — tools shipped with the binary (exec, web_fetch, etc.)
- **`Mcp { server }`** — tools provided by an MCP server, tagged with the
  server name
- **`Wasm { component_hash }`** — tools provided by a WASM component

This replaces the previous convention of identifying MCP tools by their
`mcp__` name prefix, providing type-safe filtering instead of string matching.

## Registration

Every entry also has a `ToolAudience`:

- **`Trusted`** — authenticated owner and authorized operator turns
- **`Public`** — eligible for explicitly configured untrusted origins such as
  webhook payloads

Registration is fail-closed. Built-in tools are trusted-only by default, and
all MCP and WASM tools are trusted-only. A reviewed built-in must explicitly use
`register_public` to become public. Tool descriptions and third-party metadata
cannot grant public access.

Public registration does not grant a tool to messaging-channel guests or
shared rooms. Those turns receive an additional deny-all request policy. A
webhook also receives no tools unless its configuration explicitly opts in.

```rust
// Trusted-only built-in tool (the default)
registry.register(Box::new(MyTool::new()));

// Built-in reviewed as safe for untrusted input
registry.register_public(Box::new(PublicTool::new()));

// Trusted-only MCP tool, tagged with server name
registry.register_mcp(Box::new(adapter), "github".into());
```

Duplicate names are rejected rather than overwritten. Intentional wrappers use
`replace`, which preserves the original source and audience metadata.

## Filtering

When MCP tools are disabled for a session, the registry can produce a filtered
copy:

```rust
// Type-safe: filters by ToolSource::Mcp variant
let no_mcp = registry.clone_without_mcp();

// Remove all MCP tools in-place (used during sync)
let removed_count = registry.unregister_mcp();

// Keep only tools explicitly reviewed for public use
let public = registry.clone_for_audience(ToolAudience::Public);
```

## Schema Output

`list_schemas()` includes source metadata in every tool schema:

```json
{
  "name": "exec",
  "description": "Execute a command",
  "parameters": { ... },
  "source": "builtin",
  "audience": "trusted"
}
```

```json
{
  "name": "mcp__github__search",
  "description": "Search GitHub",
  "parameters": { ... },
  "source": "mcp",
  "mcpServer": "github",
  "audience": "trusted"
}
```

The source and audience fields are available to the UI for diagnostics. The
audience is enforced by registry filtering before model tool schemas are built;
it is not an instruction that the model is expected to follow.

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
