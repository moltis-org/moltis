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
[gateway]
port = 13131                    # HTTP/WebSocket port
host = "0.0.0.0"               # Listen address

[agent]
name = "Moltis"                 # Agent display name
model = "claude-sonnet-4-20250514"  # Default model
timeout = 600                   # Agent run timeout (seconds)
max_iterations = 25             # Max tool call iterations per run
```

## LLM Providers

Provider API keys are stored separately in `~/.config/moltis/provider_keys.json` for security. Configure them through the web UI or directly in the JSON file.

```toml
[providers]
default = "anthropic"           # Default provider

[providers.anthropic]
enabled = true
models = [
    "claude-sonnet-4-20250514",
    "claude-opus-4-20250514",
    "claude-3-5-haiku-20241022",
]

[providers.openai]
enabled = true
models = [
    "gpt-4o",
    "gpt-4o-mini",
    "o1-preview",
]
```

See [Providers](providers.md) for detailed provider configuration.

## Sandbox Configuration

Commands run inside isolated containers for security:

```toml
[tools.exec.sandbox]
enabled = true
backend = "docker"              # "docker" or "apple" (macOS 15+)
base_image = "ubuntu:25.10"

# Packages installed in the sandbox image
packages = [
    "curl",
    "git",
    "jq",
    "python3",
    "python3-pip",
    "nodejs",
    "npm",
]
```

```admonish info
When you modify the packages list and restart, Moltis automatically rebuilds the sandbox image with a new tag.
```

## Memory System

Long-term memory uses embeddings for semantic search:

```toml
[memory]
enabled = true
embedding_model = "text-embedding-3-small"  # OpenAI embedding model
chunk_size = 512                # Characters per chunk
chunk_overlap = 50              # Overlap between chunks

# Directories to watch for memory files
watch_dirs = [
    "~/.moltis/memory",
]
```

## Authentication

Authentication is **only required when accessing Moltis from a non-localhost address**. When running on `localhost` or `127.0.0.1`, no authentication is needed by default.

When you access Moltis from a network address (e.g., `http://192.168.1.100:13131`), a one-time setup code is printed to the terminal. Use it to set up a password or passkey.

```toml
[auth]
disabled = false                # Set true to disable auth entirely

# Session settings
session_expiry = 604800         # Session lifetime in seconds (7 days)
```

```admonish warning
Only set `disabled = true` if Moltis is running on a trusted private network. Never expose an unauthenticated instance to the internet.
```

## Hooks

Configure lifecycle hooks:

```toml
[[hooks]]
name = "my-hook"
command = "./hooks/my-hook.sh"
events = ["BeforeToolCall", "AfterToolCall"]
timeout = 5                     # Timeout in seconds

[hooks.env]
MY_VAR = "value"               # Environment variables for the hook
```

See [Hooks](hooks.md) for the full hook system documentation.

## MCP Servers

Connect to Model Context Protocol servers:

```toml
[[mcp.servers]]
name = "filesystem"
command = "npx"
args = ["-y", "@modelcontextprotocol/server-filesystem", "/path/to/allowed"]

[[mcp.servers]]
name = "github"
command = "npx"
args = ["-y", "@modelcontextprotocol/server-github"]
env = { GITHUB_TOKEN = "ghp_..." }
```

## Telegram Integration

```toml
[telegram]
enabled = true
# Token is stored in provider_keys.json, not here
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
# Defaults to the gateway port + 1 when not set.
# http_redirect_port = 13132
```

Override via environment variable: `MOLTIS_TLS__HTTP_REDIRECT_PORT=8080`.

## Tailscale Integration

Expose Moltis over your Tailscale network:

```toml
[tailscale]
enabled = true
mode = "serve"                  # "serve" (private) or "funnel" (public)
```

## Observability

```toml
[telemetry]
enabled = true
otlp_endpoint = "http://localhost:4317"  # OpenTelemetry collector
```

## Environment Variables

All settings can be overridden via environment variables:

| Variable | Description |
|----------|-------------|
| `MOLTIS_CONFIG_DIR` | Configuration directory |
| `MOLTIS_DATA_DIR` | Data directory |
| `MOLTIS_PORT` | Gateway port |
| `MOLTIS_HOST` | Listen address |

## CLI Flags

```bash
moltis --config-dir /path/to/config --data-dir /path/to/data
```

## Complete Example

```toml
[gateway]
port = 13131
host = "0.0.0.0"

[agent]
name = "Atlas"
model = "claude-sonnet-4-20250514"
timeout = 600
max_iterations = 25

[providers]
default = "anthropic"

[tools.exec.sandbox]
enabled = true
backend = "docker"
base_image = "ubuntu:25.10"
packages = ["curl", "git", "jq", "python3", "nodejs"]

[memory]
enabled = true

[auth]
disabled = false

[[hooks]]
name = "audit-log"
command = "./hooks/audit.sh"
events = ["BeforeToolCall"]
timeout = 5
```
