# MCP Servers

Moltis supports the [Model Context Protocol (MCP)](https://modelcontextprotocol.io) for connecting to external tool servers. MCP servers extend your agent's capabilities without modifying Moltis itself.

## What is MCP?

MCP is an open protocol that lets AI assistants connect to external tools and data sources. Think of MCP servers as plugins that provide:

- **Tools** — Functions the agent can call (e.g., search, file operations, API calls)
- **Resources** — Data the agent can read (e.g., files, database records)
- **Prompts** — Pre-defined prompt templates

## Supported Transports

| Transport | Description | Use Case |
|-----------|-------------|----------|
| **stdio** | Local process via stdin/stdout | npm packages, local scripts |
| **Streamable HTTP** | Remote server via HTTP | Cloud services, shared servers |

## Adding an MCP Server

### Via Web UI

1. Go to **Settings** → **MCP Servers**
2. Click **Add Server**
3. Enter the server configuration
4. Click **Save**

### Via Configuration

Add servers to `moltis.toml`:

```toml
[mcp.servers.filesystem]
command = "npx"
args = ["-y", "@modelcontextprotocol/server-filesystem", "/Users/me/projects"]

[mcp.servers.github]
command = "npx"
args = ["-y", "@modelcontextprotocol/server-github"]
env = { GITHUB_TOKEN = "ghp_..." }

[mcp.servers.remote_api]
url = "https://mcp.example.com/mcp"
transport = "sse"
```

## Popular MCP Servers

### Official Servers

| Server | Description | Install |
|--------|-------------|---------|
| **filesystem** | Read/write local files | `npx @modelcontextprotocol/server-filesystem` |
| **github** | GitHub API access | `npx @modelcontextprotocol/server-github` |
| **postgres** | PostgreSQL queries | `npx @modelcontextprotocol/server-postgres` |
| **sqlite** | SQLite database | `npx @modelcontextprotocol/server-sqlite` |
| **puppeteer** | Browser automation | `npx @modelcontextprotocol/server-puppeteer` |
| **brave-search** | Web search | `npx @modelcontextprotocol/server-brave-search` |

### Community Servers

Explore more at [mcp.so](https://mcp.so) and [GitHub MCP Servers](https://github.com/modelcontextprotocol/servers).

## Configuration Options

```toml
[mcp.servers.my_server]
command = "node"                # Required for stdio transport
args = ["server.js"]            # Optional arguments

# Optional environment variables
env = { API_KEY = "secret", DEBUG = "true" }

# Optional: remote transport
transport = "sse"               # "stdio" (default) or "sse"
url = "https://mcp.example.com/mcp"  # Required when transport = "sse"
```

## Server Lifecycle

```
┌─────────────────────────────────────────────────────┐
│                   MCP Server                         │
│                                                      │
│  Start → Initialize → Ready → [Tool Calls] → Stop   │
│            │                       │                 │
│            ▼                       ▼                 │
│     Health Check ◄─────────── Heartbeat             │
│            │                       │                 │
│            ▼                       ▼                 │
│    Crash Detected ───────────► Restart              │
│                                    │                 │
│                              Backoff Wait            │
└─────────────────────────────────────────────────────┘
```

### Health Monitoring

Moltis monitors MCP servers and automatically:

- Detects crashes via process exit
- Restarts with exponential backoff
- Disables after max restart attempts
- Re-enables after cooldown period

## Using MCP Tools

Once connected, MCP tools appear alongside built-in tools. The agent can use them naturally:

```
User: Search GitHub for Rust async runtime projects

Agent: I'll search GitHub for you.
[Calling github.search_repositories with query="rust async runtime"]

Found 15 repositories:
1. tokio-rs/tokio - A runtime for writing reliable async applications
2. async-std/async-std - Async version of the Rust standard library
...
```

## Creating an MCP Server

### Simple Node.js Server

```javascript
// server.js
import { Server } from "@modelcontextprotocol/sdk/server/index.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";

const server = new Server(
  { name: "my-server", version: "1.0.0" },
  { capabilities: { tools: {} } }
);

server.setRequestHandler("tools/list", async () => ({
  tools: [{
    name: "hello",
    description: "Says hello",
    inputSchema: {
      type: "object",
      properties: {
        name: { type: "string", description: "Name to greet" }
      },
      required: ["name"]
    }
  }]
}));

server.setRequestHandler("tools/call", async (request) => {
  if (request.params.name === "hello") {
    const name = request.params.arguments.name;
    return { content: [{ type: "text", text: `Hello, ${name}!` }] };
  }
});

const transport = new StdioServerTransport();
await server.connect(transport);
```

### Configure in Moltis

```toml
[mcp.servers.my_server]
command = "node"
args = ["server.js"]
```

## Debugging

### Check Server Status

In the web UI, go to **Settings** → **MCP Servers** to see:

- Connection status (connected/disconnected/error)
- Available tools
- Recent errors

### View Logs

MCP server stderr is captured in Moltis logs:

```bash
# View gateway logs
tail -f ~/.moltis/logs.jsonl | grep -i mcp
```

### Test Locally

Run the server directly to debug:

```bash
echo '{"jsonrpc":"2.0","method":"tools/list","id":1}' | node server.js
```

## OAuth Authentication

Remote MCP servers can require OAuth 2.1 authentication. Moltis handles this automatically — when a server returns `401 Unauthorized`, the OAuth flow starts without any manual configuration.

### How It Works

1. Moltis connects to the remote MCP server
2. The server returns `401 Unauthorized` with a `WWW-Authenticate` header
3. Moltis discovers the authorization server via [RFC 9728](https://www.rfc-editor.org/rfc/rfc9728) (Protected Resource Metadata)
4. Moltis performs [dynamic client registration](https://www.rfc-editor.org/rfc/rfc7591) (RFC 7591)
5. A PKCE authorization code flow opens your browser for login
6. After login, tokens are stored and used for all subsequent requests

Client registrations and tokens are cached locally, so you only need to log in once per server.

### Manual OAuth Configuration

If a server doesn't support standard OAuth discovery, you can configure credentials manually:

```toml
[mcp.servers.private_api]
url = "https://mcp.example.com/mcp"
transport = "sse"

[mcp.servers.private_api.oauth]
client_id = "your-client-id"
auth_url = "https://auth.example.com/authorize"
token_url = "https://auth.example.com/token"
scopes = ["mcp:read", "mcp:write"]
```

### Re-authentication

If your session expires or tokens are revoked, Moltis automatically re-authenticates on the next `401` response. You can also trigger re-authentication manually via the `mcp.reauth` RPC method.

## Security Considerations

```admonish warning
MCP servers run with the same permissions as Moltis. Only use servers from trusted sources.
```

- **Review server code** before running
- **Limit file access** — use specific paths, not `/`
- **Use environment variables** for secrets
- **Network isolation** — run untrusted servers in containers

## Troubleshooting

### Server won't start

- Check the command exists: `which npx`
- Verify the package: `npx @modelcontextprotocol/server-filesystem --help`
- Check for port conflicts

### Tools not appearing

- Server may still be initializing (wait a few seconds)
- Check server logs for errors
- Verify the server implements `tools/list`

### Server keeps restarting

- Check stderr for crash messages
- Increase `max_restart_attempts` for debugging
- Verify environment variables are set correctly
