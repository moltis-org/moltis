# Configuration

Moltis is configured through `moltis.toml`, located in `~/.config/moltis/` by default.

On first run, a complete configuration file is generated with sensible defaults. You can edit it to customize behavior.

## Configuration File Location

| Platform | Default Path |
|----------|--------------|
| macOS/Linux | `~/.config/moltis/moltis.toml` |
| Custom | Set via `--config-dir` or `MOLTIS_CONFIG_DIR` |

## Basic Settings

```toml
[server]
port = 13131                    # HTTP/WebSocket port
bind = "0.0.0.0"               # Listen address

[identity]
name = "Moltis"                 # Agent display name

[tools]
agent_timeout_secs = 600        # Agent run timeout (seconds, 0 = no timeout)
agent_max_iterations = 25       # Max tool call iterations per run
```

## LLM Providers

Provider API keys are stored separately in `~/.config/moltis/provider_keys.json` for security. Configure them through the web UI or directly in the JSON file.

```toml
[providers]
offered = ["openai", "anthropic", "local-llm"]

[providers.openai]
enabled = true
models = ["gpt-5.3", "gpt-5.2"]
stream_transport = "sse"        # "sse", "websocket", or "auto"

[providers.anthropic]
enabled = true

[providers.local-llm]
enabled = true
models = ["qwen2.5-coder-7b-q4_k_m"]

[chat]
priority_models = ["gpt-5.2"]
```

See [Providers](providers.md) for detailed provider configuration.

*More providers are coming soon.*

## Sandbox Configuration

Commands run inside isolated containers for security:

```toml
[tools.exec.sandbox]
mode = "all"                    # "off", "non-main", or "all"
scope = "session"               # "command", "session", or "global"
workspace_mount = "ro"          # "ro", "rw", or "none"
home_persistence = "shared"     # "off", "session", or "shared" (default: "shared")
# shared_home_dir = "/path/to/shared-home"  # Optional path for shared mode
backend = "auto"                # "auto", "docker", or "apple-container"
no_network = true

# Packages installed in the sandbox image
packages = [
    "curl",
    "git",
    "jq",
    "python3",
    "python3-pip",
    "nodejs",
    "npm",
    "golang-go",
]
```

```admonish info
When you modify the packages list and restart, Moltis automatically rebuilds the sandbox image with a new tag.
```

## Web Search

Configure the built-in `web_search` tool:

```toml
[tools.web.search]
enabled = true
provider = "brave"               # "brave" or "perplexity"
max_results = 5                  # 1-10
timeout_seconds = 30
cache_ttl_minutes = 15
duckduckgo_fallback = false      # Default: do not use DuckDuckGo fallback
# api_key = "..."                # Brave key, or use BRAVE_API_KEY

[tools.web.search.perplexity]
# api_key = "..."                # Or use PERPLEXITY_API_KEY / OPENROUTER_API_KEY
# base_url = "..."               # Optional override
# model = "perplexity/sonar-pro" # Optional override
```

If no search API key is configured:

- with `duckduckgo_fallback = false` (default), Moltis returns a clear hint to set `BRAVE_API_KEY` or `PERPLEXITY_API_KEY`
- with `duckduckgo_fallback = true`, Moltis attempts DuckDuckGo HTML search, which may hit CAPTCHA/rate limits

## Chat Message Queue

When a new message arrives while an agent run is already active, Moltis can either
replay queued messages one-by-one or merge them into a single follow-up message.

```toml
[chat]
message_queue_mode = "followup"  # Default: one-by-one replay

# Options:
#   "followup" - Queue each message and run them sequentially
#   "collect"  - Merge queued text and run once after the active run
```

## Memory System

Long-term memory uses embeddings for semantic search:

```toml
[memory]
backend = "builtin"             # Or "qmd"
provider = "openai"             # Or "local", "ollama", "custom"
model = "text-embedding-3-small"
citations = "auto"              # "on", "off", or "auto"
llm_reranking = false
session_export = false
```

## Authentication

Authentication is **only required when accessing Moltis from a non-localhost address**. When running on `localhost` or `127.0.0.1`, no authentication is needed by default.

When you access Moltis from a network address (e.g., `http://192.168.1.100:13131`), a one-time setup code is printed to the terminal. Use it to set up a password or passkey.

```toml
[auth]
disabled = false                # Set true to disable auth entirely
```

```admonish warning
Only set `disabled = true` if Moltis is running on a trusted private network. Never expose an unauthenticated instance to the internet.
```

## Hooks

Configure lifecycle hooks:

```toml
[hooks]
[[hooks.hooks]]
name = "my-hook"
command = "./hooks/my-hook.sh"
events = ["BeforeToolCall", "AfterToolCall"]
timeout = 5                     # Timeout in seconds

[hooks.hooks.env]
MY_VAR = "value"               # Environment variables for the hook
```

See [Hooks](hooks.md) for the full hook system documentation.

## MCP Servers

Connect to Model Context Protocol servers:

```toml
[mcp]

[mcp.servers.filesystem]
command = "npx"
args = ["-y", "@modelcontextprotocol/server-filesystem", "/path/to/allowed"]

[mcp.servers.github]
command = "npx"
args = ["-y", "@modelcontextprotocol/server-github"]
env = { GITHUB_TOKEN = "ghp_..." }
```

## Telegram Integration

```toml
[channels.telegram.my-bot]
token = "123456:ABC..."
allowed_users = [123456789]     # Telegram user IDs allowed to chat
```

## TLS / HTTPS

```toml
[tls]
enabled = true
cert_path = "~/.config/moltis/cert.pem"
key_path = "~/.config/moltis/key.pem"
# If paths don't exist, a self-signed certificate is generated

# Port for the plain-HTTP redirect / CA-download server.
# Defaults to the server port + 1 when not set.
# http_redirect_port = 13132
```

Override via environment variable: `MOLTIS_TLS__HTTP_REDIRECT_PORT=8080`.

## Tailscale Integration

Expose Moltis over your Tailscale network:

```toml
[tailscale]
mode = "serve"                  # "off", "serve", or "funnel"
reset_on_exit = true
```

## Observability

```toml
[metrics]
enabled = true
prometheus_endpoint = true
```

## Process Environment Variables (`[env]`)

The `[env]` section injects variables into the Moltis process at startup.
This is useful in Docker deployments where passing individual `-e` flags is
inconvenient, or when you want API keys stored in the config file rather
than the host environment.

```toml
[env]
BRAVE_API_KEY = "your-brave-key"
OPENROUTER_API_KEY = "sk-or-..."
ELEVENLABS_API_KEY = "..."
```

**Precedence**: existing process environment variables are never overwritten.
If `BRAVE_API_KEY` is already set via `docker -e` or the host shell, the
`[env]` value is skipped. This means `docker -e` always wins.

```admonish info title="Settings UI vs [env]"
Environment variables configured through the Settings UI (Settings >
Environment) are also injected into the Moltis process at startup.
Precedence: host/`docker -e` > config `[env]` > Settings UI.
```

## Environment Variables

All settings can be overridden via environment variables:

| Variable | Description |
|----------|-------------|
| `MOLTIS_CONFIG_DIR` | Configuration directory |
| `MOLTIS_DATA_DIR` | Data directory |
| `MOLTIS_SERVER__PORT` | Server port override |
| `MOLTIS_SERVER__BIND` | Server bind address override |
| `MOLTIS_TOOLS__AGENT_TIMEOUT_SECS` | Agent run timeout override |
| `MOLTIS_TOOLS__AGENT_MAX_ITERATIONS` | Agent loop iteration cap override |

## CLI Flags

```bash
moltis --config-dir /path/to/config --data-dir /path/to/data
```

## Complete Example

```toml
[server]
port = 13131
bind = "0.0.0.0"

[identity]
name = "Atlas"

[tools]
agent_timeout_secs = 600
agent_max_iterations = 25

[providers]
offered = ["openai", "anthropic", "local-llm"]

[tools.exec.sandbox]
mode = "all"
scope = "session"
workspace_mount = "ro"
home_persistence = "session"
# shared_home_dir = "/path/to/shared-home"
backend = "auto"
no_network = true
packages = ["curl", "git", "jq", "python3", "nodejs", "golang-go"]

[memory]
backend = "builtin"
provider = "openai"
model = "text-embedding-3-small"

[auth]
disabled = false

[hooks]
[[hooks.hooks]]
name = "audit-log"
command = "./hooks/audit.sh"
events = ["BeforeToolCall"]
timeout = 5
```
