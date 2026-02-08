//! Default configuration template with all options documented.
//!
//! This template is used when creating a new config file. It includes all
//! available options with descriptions, allowing users to see everything
//! that can be configured even if they don't change the defaults.

/// Generate the default config template with a specific port.
pub fn default_config_template(port: u16) -> String {
    format!(
        r##"# Moltis Configuration
# ====================
# This file contains all available configuration options.
# Uncomment and modify settings as needed.
# Changes require a restart to take effect.
#
# Environment variable substitution is supported: ${{ENV_VAR}}
# Example: api_key = "${{ANTHROPIC_API_KEY}}"

# ══════════════════════════════════════════════════════════════════════════════
# SERVER
# ══════════════════════════════════════════════════════════════════════════════

[server]
bind = "127.0.0.1"                # Address to bind to ("0.0.0.0" for all interfaces)
port = {port}                           # Port number (auto-generated for this installation)
http_request_logs = false              # Enable verbose Axum HTTP request/response logs (debugging)
ws_request_logs = false                # Enable WebSocket RPC request/response logs (debugging)
update_repository_url = "https://github.com/moltis-org/moltis"    # GitHub repo used for update checks (comment out to disable)

# ══════════════════════════════════════════════════════════════════════════════
# AUTHENTICATION
# ══════════════════════════════════════════════════════════════════════════════

[auth]
disabled = false                  # true = disable auth entirely (DANGEROUS if exposed)
                                  # When disabled, anyone with network access can use moltis

# ══════════════════════════════════════════════════════════════════════════════
# TLS / HTTPS
# ══════════════════════════════════════════════════════════════════════════════

[tls]
enabled = true                    # Enable HTTPS (recommended)
auto_generate = true              # Auto-generate local CA and server certificate
# http_redirect_port = 18790      # Optional override (default: server.port + 1)
# cert_path = "/path/to/cert.pem"     # Custom certificate file (overrides auto-gen)
# key_path = "/path/to/key.pem"       # Custom private key file
# ca_cert_path = "/path/to/ca.pem"    # CA certificate for trust instructions

# ══════════════════════════════════════════════════════════════════════════════
# AGENT IDENTITY
# ══════════════════════════════════════════════════════════════════════════════
# Customize your agent's personality. These are typically set during onboarding.

[identity]
# name = "moltis"                 # Agent's display name
# emoji = "🦊"                    # Agent's emoji/avatar
# creature = "fox"                # Creature type for personality
# vibe = "helpful"                # Personality vibe/style
# soul = ""                       # Freeform personality text injected into system prompt
                                  # Use this for custom instructions, tone, or behavior

# ══════════════════════════════════════════════════════════════════════════════
# USER PROFILE
# ══════════════════════════════════════════════════════════════════════════════
# Information about you. Set during onboarding.

[user]
# name = "Your Name"              # Your name (used in conversations)
# timezone = "America/New_York"   # Your timezone (IANA format)

# ══════════════════════════════════════════════════════════════════════════════
# LLM PROVIDERS
# ══════════════════════════════════════════════════════════════════════════════
# Configure API keys and settings for each LLM provider.
# API keys can also be set via environment variables (preferred for security).
#
# Each provider supports:
#   enabled   - Whether to use this provider (default: true)
#   api_key   - API key (or use env var like ANTHROPIC_API_KEY)
#   base_url  - Override API endpoint
#   model     - Default model for this provider
#   alias     - Custom name for metrics labels (useful for multiple instances)

[providers]
offered = ["openai", "github-copilot"]      # Providers shown in onboarding/picker UI ([] = show all)

# ── Anthropic (Claude) ────────────────────────────────────────
# [providers.anthropic]
# enabled = true
# api_key = "sk-ant-..."                      # Or set ANTHROPIC_API_KEY env var
# model = "claude-sonnet-4-20250514"          # Default model
# base_url = "https://api.anthropic.com"     # API endpoint
# alias = "anthropic"                         # Custom name for metrics

# ── OpenAI ────────────────────────────────────────────────────
# [providers.openai]
# enabled = true
# api_key = "sk-..."                          # Or set OPENAI_API_KEY env var
# model = "gpt-4o"                            # Default model
# base_url = "https://api.openai.com/v1"     # API endpoint (change for Azure, etc.)
# alias = "openai"

# ── Google Gemini ─────────────────────────────────────────────
# [providers.gemini]
# enabled = true
# api_key = "..."                             # Or set GOOGLE_API_KEY env var
# model = "gemini-2.0-flash"
# alias = "gemini"

# ── Groq ──────────────────────────────────────────────────────
# [providers.groq]
# enabled = true
# api_key = "..."                             # Or set GROQ_API_KEY env var
# model = "llama-3.3-70b-versatile"
# alias = "groq"

# ── DeepSeek ──────────────────────────────────────────────────
# [providers.deepseek]
# enabled = true
# api_key = "..."                             # Or set DEEPSEEK_API_KEY env var
# model = "deepseek-chat"
# base_url = "https://api.deepseek.com"
# alias = "deepseek"

# ── xAI (Grok) ────────────────────────────────────────────────
# [providers.xai]
# enabled = true
# api_key = "..."                             # Or set XAI_API_KEY env var
# model = "grok-3-mini"
# alias = "xai"

# ── OpenRouter (multi-provider gateway) ───────────────────────
# [providers.openrouter]
# enabled = true
# api_key = "..."                             # Or set OPENROUTER_API_KEY env var
# model = "anthropic/claude-3.5-sonnet"       # Any model on OpenRouter
# base_url = "https://openrouter.ai/api/v1"

# ══════════════════════════════════════════════════════════════════════════════
# CHAT SETTINGS
# ══════════════════════════════════════════════════════════════════════════════

[chat]
message_queue_mode = "followup"   # How to handle messages during an active agent run:
                                  #   "followup" - Queue messages, replay one-by-one after run
                                  #   "collect"  - Buffer messages, concatenate as single message
# priority_models = ["claude-opus-4-5", "gpt-5.2", "gemini-3-flash"]  # Optional: models to pin first in selectors

# ══════════════════════════════════════════════════════════════════════════════
# TOOLS
# ══════════════════════════════════════════════════════════════════════════════

[tools]
agent_timeout_secs = 600          # Max seconds for an agent run (0 = no timeout)
max_tool_result_bytes = 50000     # Max bytes per tool result before truncation (50KB)

# ── Command Execution ─────────────────────────────────────────────────────────

[tools.exec]
default_timeout_secs = 30         # Default timeout for commands
max_output_bytes = 204800         # Max command output bytes (200KB)
approval_mode = "on-miss"         # When to require approval:
                                  #   "always"  - Always ask before running
                                  #   "on-miss" - Ask if not in allowlist
                                  #   "never"   - Never ask (dangerous)
security_level = "allowlist"      # Security mode:
                                  #   "permissive" - Allow most commands
                                  #   "allowlist"  - Only allow listed commands
                                  #   "strict"     - Very restrictive
allowlist = []                    # Command patterns to allow (when security_level = "allowlist")
                                  # Example: ["git *", "npm *", "cargo *"]

# ── Sandbox Configuration ─────────────────────────────────────────────────────
# Commands run inside isolated containers for security.

[tools.exec.sandbox]
mode = "all"                      # Which commands to sandbox:
                                  #   "off"      - No sandboxing (commands run on host)
                                  #   "non-main" - Sandbox all except main session
                                  #   "all"      - Sandbox everything (recommended)
scope = "session"                 # Container lifecycle:
                                  #   "command" - New container per command
                                  #   "session" - Container per session (recommended)
                                  #   "global"  - Single shared container
workspace_mount = "ro"            # How to mount workspace in sandbox:
                                  #   "ro"   - Read-only (safe)
                                  #   "rw"   - Read-write (can modify files)
                                  #   "none" - No mount
backend = "auto"                  # Container backend:
                                  #   "auto"            - Auto-detect (prefers Apple Container on macOS)
                                  #   "docker"          - Use Docker
                                  #   "apple-container" - Use Apple Container (macOS only)
no_network = true                 # Disable network access in sandbox (recommended)
# image = "custom-image:tag"      # Custom Docker image (default: auto-built)
# container_prefix = "moltis"     # Prefix for container names

# Packages installed in sandbox containers via apt-get.
# This list is used to build the sandbox image. Customize as needed.
packages = [
    # Networking & HTTP
    "curl",
    "wget",
    "ca-certificates",
    "dnsutils",
    "netcat-openbsd",
    "openssh-client",
    "iproute2",
    "net-tools",
    # Language runtimes
    "python3",
    "python3-dev",
    "python3-pip",
    "python3-venv",
    "python-is-python3",
    "nodejs",
    "npm",
    "ruby",
    "ruby-dev",
    # Build toolchain & native deps
    "build-essential",
    "clang",
    "libclang-dev",
    "llvm-dev",
    "pkg-config",
    "libssl-dev",
    "libsqlite3-dev",
    "libyaml-dev",
    "liblzma-dev",
    "autoconf",
    "automake",
    "libtool",
    "bison",
    "flex",
    "dpkg-dev",
    "fakeroot",
    # Compression & archiving
    "zip",
    "unzip",
    "bzip2",
    "xz-utils",
    "p7zip-full",
    "tar",
    "zstd",
    "lz4",
    "pigz",
    # Common CLI utilities
    "git",
    "gnupg2",
    "jq",
    "rsync",
    "file",
    "tree",
    "sqlite3",
    "sudo",
    "locales",
    "tzdata",
    "shellcheck",
    "patchelf",
    # Text processing & search
    "ripgrep",
    # Browser automation dependencies
    "chromium",
    "libxss1",
    "libnss3",
    "libnspr4",
    "libasound2t64",
    "libatk1.0-0t64",
    "libatk-bridge2.0-0t64",
    "libcups2t64",
    "libdrm2",
    "libgbm1",
    "libgtk-3-0t64",
    "libxcomposite1",
    "libxdamage1",
    "libxfixes3",
    "libxrandr2",
    "libxkbcommon0",
    "fonts-liberation",
]

# Resource limits for sandboxed execution (optional)
[tools.exec.sandbox.resource_limits]
# memory_limit = "512M"           # Memory limit (e.g., "512M", "1G", "2G")
# cpu_quota = 0.5                 # CPU quota as fraction (0.5 = half a core, 2.0 = two cores)
# pids_max = 100                  # Maximum number of processes

# ── Tool Policy ───────────────────────────────────────────────────────────────
# Control which tools agents can use.

[tools.policy]
allow = []                        # Tools to always allow (e.g., ["exec", "web_fetch"])
deny = []                         # Tools to always deny (e.g., ["browser"])
# profile = "default"             # Named policy profile

# ── Web Search ────────────────────────────────────────────────────────────────

[tools.web.search]
enabled = true                    # Enable web search tool
provider = "brave"                # Search provider: "brave" or "perplexity"
max_results = 5                   # Number of results to return (1-10)
timeout_seconds = 30              # HTTP request timeout
cache_ttl_minutes = 15            # Cache results for this many minutes (0 = no cache)
# api_key = "..."                 # Brave API key (or set BRAVE_API_KEY env var)

# Perplexity-specific settings (when provider = "perplexity")
[tools.web.search.perplexity]
# api_key = "..."                 # Or set PERPLEXITY_API_KEY env var
# base_url = "..."                # API base URL (auto-detected from key prefix)
# model = "sonar"                 # Perplexity model to use

# ── Web Fetch ─────────────────────────────────────────────────────────────────

[tools.web.fetch]
enabled = true                    # Enable web fetch tool
max_chars = 50000                 # Max characters to return from fetched content
timeout_seconds = 30              # HTTP request timeout
cache_ttl_minutes = 15            # Cache fetched pages for this many minutes (0 = no cache)
max_redirects = 3                 # Maximum HTTP redirects to follow
readability = true                # Use readability extraction for HTML (cleaner output)

# ── Browser Automation ────────────────────────────────────────────────────────
# Full browser control via Chrome DevTools Protocol (CDP).
# Use for JavaScript-heavy sites, form filling, screenshots.

[tools.browser]
enabled = true                    # Enable browser tool
headless = true                   # Run without visible window (true = background)
viewport_width = 2560             # Default viewport width in pixels (QHD for tech users)
viewport_height = 1440            # Default viewport height in pixels
device_scale_factor = 2.0         # HiDPI/Retina scaling (2.0 = Retina, 1.0 = standard)
max_instances = 3                 # Maximum concurrent browser instances
idle_timeout_secs = 300           # Close idle browsers after this many seconds (5 min)
navigation_timeout_ms = 30000     # Page load timeout in milliseconds (30 sec)
sandbox = false                   # Run browser in Docker/Apple Container for isolation
# chrome_path = "/path/to/chrome" # Custom Chrome/Chromium binary path (auto-detected)
# user_agent = "Custom UA"        # Custom user agent string
# chrome_args = []                # Extra Chrome command-line arguments
                                  # Example: ["--disable-extensions", "--disable-gpu"]

# Domain restrictions for security.
# When set, browser will refuse to navigate to domains not in this list.
# This helps prevent prompt injection from untrusted websites.
allowed_domains = []              # Empty = all domains allowed
# allowed_domains = [
#     "docs.example.com",         # Exact match
#     "*.github.com",             # Wildcard: matches any subdomain of github.com
#     "localhost",                # Allow localhost
#     "127.0.0.1",
# ]

# ══════════════════════════════════════════════════════════════════════════════
# SKILLS
# ══════════════════════════════════════════════════════════════════════════════
# Reusable prompt templates and workflows.

[skills]
enabled = true                    # Enable skills system
search_paths = []                 # Additional directories to search for skills
                                  # Default locations: ~/.config/moltis/skills/, ./skills/
auto_load = []                    # Skills to always load without explicit activation
                                  # Example: ["code-review", "commit"]

# ══════════════════════════════════════════════════════════════════════════════
# MCP SERVERS
# ══════════════════════════════════════════════════════════════════════════════
# Model Context Protocol servers provide additional tools and capabilities.
# See https://modelcontextprotocol.io for available servers.

[mcp]
# Each server has a name and configuration:
#
# [mcp.servers.server-name]
# command = "npx"                 # Command to run (for stdio transport)
# args = ["-y", "@package/name"]  # Command arguments
# env = {{ KEY = "value" }}         # Environment variables for the process
# enabled = true                  # Whether this server is enabled
# transport = "stdio"             # Transport: "stdio" (default) or "sse"
# url = "http://..."              # URL for SSE transport

# Example: Filesystem access
# [mcp.servers.filesystem]
# command = "npx"
# args = ["-y", "@modelcontextprotocol/server-filesystem", "/path/to/allow"]
# enabled = true

# Example: GitHub integration
# [mcp.servers.github]
# command = "npx"
# args = ["-y", "@modelcontextprotocol/server-github"]
# env = {{ GITHUB_TOKEN = "${{GITHUB_TOKEN}}" }}
# enabled = true

# Example: SSE server
# [mcp.servers.remote]
# transport = "sse"
# url = "http://localhost:8080/sse"
# enabled = true

# ══════════════════════════════════════════════════════════════════════════════
# METRICS
# ══════════════════════════════════════════════════════════════════════════════
# Prometheus metrics for observability.

[metrics]
enabled = true                    # Enable metrics collection
prometheus_endpoint = true        # Expose /metrics endpoint for Prometheus scraping
# labels = {{ environment = "production", instance = "main" }}
                                  # Additional labels to add to all metrics

# ══════════════════════════════════════════════════════════════════════════════
# HEARTBEAT
# ══════════════════════════════════════════════════════════════════════════════
# Periodic health-check agent turns to keep the agent "alive" and responsive.

[heartbeat]
enabled = true                    # Enable periodic heartbeats
every = "30m"                     # Interval between heartbeats (e.g., "30m", "1h", "6h")
# model = "anthropic/claude-sonnet-4-20250514"  # Override model for heartbeats
# prompt = "..."                  # Custom heartbeat prompt (default: built-in)
ack_max_chars = 300               # Max characters for acknowledgment reply
sandbox_enabled = true            # Run heartbeat commands in sandbox
# sandbox_image = "..."           # Override sandbox image for heartbeats

# Active hours window - heartbeats only run during this time
[heartbeat.active_hours]
start = "08:00"                   # Start time (HH:MM, 24-hour format)
end = "24:00"                     # End time (HH:MM, "24:00" = end of day)
timezone = "local"                # Timezone: "local" or IANA name like "Europe/Paris"

# ══════════════════════════════════════════════════════════════════════════════
# FAILOVER
# ══════════════════════════════════════════════════════════════════════════════
# Automatic fallback to alternative models/providers on failure.

[failover]
enabled = true                    # Enable automatic failover
fallback_models = []              # Ordered list of fallback models
                                  # Empty = auto-build chain from all registered models
                                  # Example: ["openai/gpt-4o", "anthropic/claude-3-haiku"]

# ══════════════════════════════════════════════════════════════════════════════
# TAILSCALE
# ══════════════════════════════════════════════════════════════════════════════
# Expose moltis via Tailscale Serve (private) or Funnel (public).

[tailscale]
mode = "off"                      # Tailscale mode:
                                  #   "off"    - Disabled
                                  #   "serve"  - Tailnet-only HTTPS (private)
                                  #   "funnel" - Public HTTPS via Tailscale
reset_on_exit = true              # Reset serve/funnel when gateway shuts down

# ══════════════════════════════════════════════════════════════════════════════
# MEMORY / EMBEDDINGS
# ══════════════════════════════════════════════════════════════════════════════
# Configure the embedding provider for memory/RAG features.

[memory]
# provider = "local"              # Embedding provider:
                                  #   "local"   - Built-in local embeddings
                                  #   "ollama"  - Ollama server
                                  #   "openai"  - OpenAI API
                                  #   "custom"  - Custom endpoint
                                  #   (none)    - Auto-detect from available providers
# base_url = "http://localhost:11434/v1"  # API endpoint for embeddings
# model = "nomic-embed-text"      # Embedding model name
# api_key = "..."                 # API key (optional for local endpoints like Ollama)

# ══════════════════════════════════════════════════════════════════════════════
# CHANNELS
# ══════════════════════════════════════════════════════════════════════════════
# External messaging integrations.

[channels]
# Telegram bots
# [channels.telegram.my-bot]
# token = "..."                   # Bot token from @BotFather
# allowed_users = []              # Telegram user IDs allowed to chat (empty = all)

# ══════════════════════════════════════════════════════════════════════════════
# HOOKS
# ══════════════════════════════════════════════════════════════════════════════
# Shell commands triggered by events.

# [hooks]
# [[hooks.hooks]]
# name = "notify-on-complete"     # Hook name (for logging)
# command = "/path/to/script.sh"  # Command to run
# events = [                      # Events that trigger this hook:
#     "agent.turn.start",         #   Agent turn started
#     "agent.turn.complete",      #   Agent turn completed
#     "tool.call.start",          #   Tool call started
#     "tool.call.complete",       #   Tool call completed
#     "session.create",           #   Session created
#     "session.close",            #   Session closed
# ]
# timeout = 10                    # Command timeout in seconds
# [hooks.hooks.env]               # Environment variables passed to command
# CUSTOM_VAR = "value"
# SESSION_ID = "${{SESSION_ID}}"    # Variables are substituted
"##
    )
}
