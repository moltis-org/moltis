---
name: mcp-servers
description: Add, remove, configure, and troubleshoot MCP (Model Context Protocol) servers. Use when the user wants to connect external tools, manage MCP server lifecycle, or debug MCP connectivity issues.
origin:
  source: moltis
  url: https://github.com/moltis-org/moltis
  version: "1.0"
---

# MCP Server Management

Manage MCP (Model Context Protocol) servers that provide external tools to the agent.
MCP servers extend the agent's capabilities by exposing tools over stdio, SSE, or streamable-HTTP transports.

## Agent Tools

The agent has built-in tools for MCP management — no sandbox, network, or CLI needed:

- `mcp_list` — list all configured servers with status
- `mcp_add` — add a new server (params: `name`, `command`, `args`, `transport`, `url`, `env`, `display_name`)
- `mcp_remove` — remove a server (params: `name`)
- `mcp_status` — detailed status for a server (params: `name`)
- `mcp_restart` — restart a running server (params: `name`)

These are the preferred way to manage MCP servers from skills. Use them directly.

## RPC Methods

For advanced use or direct API access, the full RPC namespace is documented below.

## Listing Servers

```json
// RPC: mcp.list
```

Returns all configured servers with their connection status, transport type, and enabled state.

## Adding a Server

### Stdio transport (local process)

```json
// RPC: mcp.add
{
  "name": "filesystem",
  "command": "npx",
  "args": ["-y", "@modelcontextprotocol/server-filesystem", "/home/user/documents"]
}
```

### SSE transport (remote)

```json
// RPC: mcp.add
{
  "name": "remote-tools",
  "transport": "sse",
  "url": "https://mcp.example.com/sse"
}
```

### Streamable HTTP transport

```json
// RPC: mcp.add
{
  "name": "api-tools",
  "transport": "streamable-http",
  "url": "https://mcp.example.com/mcp"
}
```

### With environment variables

```json
// RPC: mcp.add
{
  "name": "github",
  "command": "npx",
  "args": ["-y", "@modelcontextprotocol/server-github"],
  "env": { "GITHUB_PERSONAL_ACCESS_TOKEN": "ghp_..." }
}
```

### With custom display name

```json
// RPC: mcp.add
{
  "name": "gbrain",
  "command": "gbrain",
  "args": ["serve", "--mcp"],
  "display_name": "GBrain Knowledge Base"
}
```

Returns `{ "ok": true, "name": "<final_name>" }`. If a server with the same name already exists, a numeric suffix is appended automatically (e.g. `github-2`).

## Removing a Server

```json
// RPC: mcp.remove
{ "name": "filesystem" }
```

## Updating a Server

Partial update — omitted fields keep their current values.

```json
// RPC: mcp.update
{
  "name": "github",
  "args": ["-y", "@modelcontextprotocol/server-github", "--verbose"],
  "env": { "GITHUB_PERSONAL_ACCESS_TOKEN": "ghp_new_token" }
}
```

## Enable / Disable

Toggle a server without removing its configuration.

```json
// RPC: mcp.enable
{ "name": "filesystem" }

// RPC: mcp.disable
{ "name": "filesystem" }
```

## Restart

Force-restart a running server (useful after config changes).

```json
// RPC: mcp.restart
{ "name": "filesystem" }
```

## Server Status

Detailed status for a single server including connection state, uptime, and error info.

```json
// RPC: mcp.status
{ "name": "filesystem" }
```

## List Tools

Show which tools a specific MCP server exposes.

```json
// RPC: mcp.tools
{ "name": "filesystem" }
```

## Global Config

```json
// RPC: mcp.config.get

// RPC: mcp.config.update
{ "request_timeout_secs": 120 }
```

The global `request_timeout_secs` applies to all servers that don't set their own timeout.
Per-server timeouts can be set via `mcp.update` with `request_timeout_secs`.

## OAuth

For MCP servers that require OAuth authentication:

```json
// RPC: mcp.oauth.start
{ "name": "my-oauth-server" }

// RPC: mcp.oauth.complete
{ "name": "my-oauth-server", "code": "auth_code_here" }

// RPC: mcp.reauth
{ "name": "my-oauth-server" }
```

## Transport Types

| Transport | When to Use | Required Fields |
|-----------|-------------|----------------|
| `stdio` (default) | Local process, fast, no network | `command`, optional `args`, `env` |
| `sse` | Remote server, Server-Sent Events | `url` |
| `streamable-http` | Remote server, HTTP streaming | `url` |

## Common Patterns

### GBrain knowledge base

```json
// RPC: mcp.add
{
  "name": "gbrain",
  "command": "gbrain",
  "args": ["serve", "--mcp"],
  "display_name": "GBrain Knowledge Base"
}
```

Requires: `bun install -g gbrain && gbrain init`

### GitHub tools

```json
// RPC: mcp.add
{
  "name": "github",
  "command": "npx",
  "args": ["-y", "@modelcontextprotocol/server-github"],
  "env": { "GITHUB_PERSONAL_ACCESS_TOKEN": "ghp_..." }
}
```

### Filesystem access

```json
// RPC: mcp.add
{
  "name": "filesystem",
  "command": "npx",
  "args": ["-y", "@modelcontextprotocol/server-filesystem", "/path/to/allowed/directory"]
}
```

### Memory / persistent storage

```json
// RPC: mcp.add
{
  "name": "memory",
  "command": "npx",
  "args": ["-y", "@modelcontextprotocol/server-memory"]
}
```

## Troubleshooting

1. **Server won't connect?** Check `mcp.status` for error details. For stdio servers, verify the command is in PATH.
2. **Tools not appearing?** Use `mcp.tools` to see what the server exposes. Try `mcp.restart` to force reconnection.
3. **Timeout errors?** Increase `request_timeout_secs` via `mcp.config.update` or per-server via `mcp.update`.
4. **OAuth errors?** Run `mcp.reauth` to restart the OAuth flow.
5. **Environment variables?** Sensitive values (API keys, tokens) go in the `env` field of the server config.
