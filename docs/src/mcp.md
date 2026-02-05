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
| **HTTP/SSE** | Remote server via HTTP | Cloud services, shared servers |

## Adding an MCP Server

### Via Web UI

1. Go to **Settings** → **MCP Servers**
2. Click **Add Server**
3. Enter the server configuration
4. Click **Save**

### Via Configuration

Add servers to `moltis.toml`:

```toml
[[mcp.servers]]
name = "filesystem"
command = "npx"
args = ["-y", "@modelcontextprotocol/server-filesystem", "/Users/me/projects"]

[[mcp.servers]]
name = "github"
command = "npx"
args = ["-y", "@modelcontextprotocol/server-github"]
env = { GITHUB_TOKEN = "ghp_..." }

[[mcp.servers]]
name = "remote-api"
url = "https://mcp.example.com/sse"
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
[[mcp.servers]]
name = "my-server"              # Display name
command = "node"                # Command to run
args = ["server.js"]            # Command arguments
cwd = "/path/to/server"         # Working directory

# Environment variables
env = { API_KEY = "secret", DEBUG = "true" }

# Health check settings
health_check_interval = 30      # Seconds between health checks
restart_on_failure = true       # Auto-restart on crash
max_restart_attempts = 5        # Give up after N restarts
restart_backoff = "exponential" # "linear" or "exponential"
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
[[mcp.servers]]
name = "my-server"
command = "node"
args = ["server.js"]
cwd = "/path/to/my-server"
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
tail -f ~/.moltis/logs/gateway.log | grep mcp
```

### Test Locally

Run the server directly to debug:

```bash
echo '{"jsonrpc":"2.0","method":"tools/list","id":1}' | node server.js
```

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
