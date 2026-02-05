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

# ── Server ─────────────────────────────────────────────────────
[server]
bind = "127.0.0.1"      # Address to bind to
port = {port}                 # Port (auto-generated for this installation)

# ── Authentication ─────────────────────────────────────────────
[auth]
disabled = false        # Set to true to disable authentication entirely

# ── TLS / HTTPS ────────────────────────────────────────────────
[tls]
enabled = true          # Enable HTTPS with auto-generated certificates
auto_generate = true    # Auto-generate local CA and server certificate
# cert_path = "/path/to/cert.pem"   # Custom certificate (overrides auto-gen)
# key_path = "/path/to/key.pem"     # Custom private key
# ca_cert_path = "/path/to/ca.pem"  # CA certificate for trust instructions
http_redirect_port = 18790          # Port for plain HTTP redirect server

# ── Agent Identity ─────────────────────────────────────────────
# Customize your agent's personality. Set during onboarding.
[identity]
# name = "moltis"       # Agent name
# emoji = "🦊"          # Agent emoji
# creature = "fox"      # Creature type
# vibe = "helpful"      # Personality vibe
# soul = "..."          # Freeform personality text for system prompt

# ── User Profile ───────────────────────────────────────────────
[user]
# name = "Your Name"    # Your name (set during onboarding)
# timezone = "America/New_York"  # Your timezone

# ── LLM Providers ──────────────────────────────────────────────
# Configure API keys and settings for each provider.
# API keys can also be set via environment variables (e.g., ANTHROPIC_API_KEY).
[providers]

# [providers.anthropic]
# enabled = true
# api_key = "sk-ant-..."           # Or set ANTHROPIC_API_KEY env var
# model = "claude-sonnet-4-20250514"

# [providers.openai]
# enabled = true
# api_key = "sk-..."               # Or set OPENAI_API_KEY env var
# model = "gpt-4o"
# base_url = "https://api.openai.com/v1"

# [providers.gemini]
# enabled = true
# api_key = "..."                  # Or set GOOGLE_API_KEY env var
# model = "gemini-2.0-flash"

# [providers.groq]
# enabled = true
# api_key = "..."                  # Or set GROQ_API_KEY env var
# model = "llama-3.3-70b-versatile"

# [providers.deepseek]
# enabled = true
# api_key = "..."                  # Or set DEEPSEEK_API_KEY env var
# model = "deepseek-chat"

# [providers.xai]
# enabled = true
# api_key = "..."                  # Or set XAI_API_KEY env var
# model = "grok-3-mini"

# ── Chat Settings ──────────────────────────────────────────────
[chat]
message_queue_mode = "followup"  # "followup" or "collect"
# followup: Queue messages, replay one-by-one after current run
# collect: Buffer messages, concatenate as single message

# ── Tools Configuration ────────────────────────────────────────
[tools]
agent_timeout_secs = 600        # Max wall-clock seconds for an agent run (0 = no timeout)
max_tool_result_bytes = 50000   # Max bytes per tool result before truncation

# Command Execution
[tools.exec]
default_timeout_secs = 30       # Default command timeout
max_output_bytes = 204800       # Max output bytes (200KB)
approval_mode = "on-miss"       # "always", "on-miss", "never"
security_level = "allowlist"    # "permissive", "allowlist", "strict"
allowlist = []                  # Allowed command patterns

# Sandbox Configuration
[tools.exec.sandbox]
mode = "all"                    # "off", "non-main", "all"
scope = "session"               # "command", "session", "global"
workspace_mount = "ro"          # "ro" (read-only), "rw" (read-write), "none"
backend = "auto"                # "auto", "docker", "apple-container"
no_network = true               # Disable network access in sandbox
# image = "custom-image:tag"    # Custom Docker image
# container_prefix = "moltis"   # Container name prefix

# Resource Limits (optional)
[tools.exec.sandbox.resource_limits]
# memory_limit = "512M"         # Memory limit (e.g., "512M", "1G")
# cpu_quota = 0.5               # CPU quota as fraction (0.5 = half a core)
# pids_max = 100                # Max number of PIDs

# Tool Policy (allow/deny specific tools)
[tools.policy]
allow = []                      # Tools to always allow
deny = []                       # Tools to always deny
# profile = "default"           # Policy profile name

# Web Tools
[tools.web.search]
enabled = true
provider = "brave"              # "brave" or "perplexity"
max_results = 5
timeout_seconds = 30
cache_ttl_minutes = 15
# api_key = "..."               # Or set BRAVE_API_KEY env var

[tools.web.search.perplexity]
# api_key = "..."               # Or set PERPLEXITY_API_KEY env var
# base_url = "..."              # API base URL (auto-detected)
# model = "sonar"               # Perplexity model

[tools.web.fetch]
enabled = true
max_chars = 50000               # Max characters from fetched content
timeout_seconds = 30
cache_ttl_minutes = 15
max_redirects = 3
readability = true              # Use readability extraction for HTML

# Browser Automation
[tools.browser]
enabled = true                  # Enable browser tool
headless = true                 # Run without visible window
viewport_width = 1280
viewport_height = 720
max_instances = 3               # Max concurrent browsers
idle_timeout_secs = 300         # Close idle browsers after 5 min
navigation_timeout_ms = 30000   # Page load timeout
sandbox = false                 # Run browser in container (not yet implemented)
# chrome_path = "/path/to/chrome"  # Custom Chrome/Chromium path
# user_agent = "Custom UA"         # Custom user agent
# chrome_args = ["--disable-extensions"]  # Extra Chrome arguments

# Domain restrictions for security (empty = all domains allowed)
# Restricting domains helps prevent prompt injection from untrusted sites.
allowed_domains = []
# allowed_domains = [
#     "docs.example.com",      # Exact match
#     "*.github.com",          # Wildcard: matches any subdomain
#     "localhost",
# ]

# ── Skills ─────────────────────────────────────────────────────
[skills]
enabled = true
search_paths = []               # Extra directories to search for skills
auto_load = []                  # Skills to always load

# ── MCP Servers ────────────────────────────────────────────────
# Configure Model Context Protocol servers for extended capabilities.
[mcp]
# [mcp.servers.filesystem]
# command = "npx"
# args = ["-y", "@anthropic-ai/mcp-filesystem", "/path/to/allow"]
# enabled = true

# ── Metrics ────────────────────────────────────────────────────
[metrics]
enabled = true                  # Enable metrics collection
prometheus_endpoint = true      # Expose /metrics endpoint

# ── Heartbeat ──────────────────────────────────────────────────
# Periodic health-check agent turns.
[heartbeat]
enabled = true
every = "30m"                   # Interval (e.g., "30m", "1h")
# model = "anthropic/claude-sonnet-4-20250514"  # Model override
# prompt = "..."                # Custom prompt override
ack_max_chars = 300             # Max chars for acknowledgment reply
sandbox_enabled = true          # Run heartbeat in sandbox
# sandbox_image = "..."         # Override sandbox image

[heartbeat.active_hours]
start = "08:00"                 # Active hours start (HH:MM)
end = "24:00"                   # Active hours end (HH:MM)
timezone = "local"              # Timezone ("local" or IANA like "Europe/Paris")

# ── Failover ───────────────────────────────────────────────────
[failover]
enabled = true                  # Enable automatic model/provider failover
fallback_models = []            # Ordered fallback models (empty = auto-build)

# ── Tailscale ──────────────────────────────────────────────────
[tailscale]
mode = "off"                    # "off", "serve", or "funnel"
reset_on_exit = true            # Reset serve/funnel when gateway shuts down

# ── Memory / Embeddings ────────────────────────────────────────
[memory]
# provider = "local"            # "local", "ollama", "openai", "custom", or auto-detect
# base_url = "http://localhost:11434/v1"  # Embedding API URL
# model = "nomic-embed-text"    # Embedding model name
# api_key = "..."               # API key (optional for local endpoints)

# ── Channels ───────────────────────────────────────────────────
# Configure messaging channels (Telegram, etc.)
[channels]
# [channels.telegram.my-bot]
# token = "..."                 # Bot token from @BotFather

# ── Hooks ──────────────────────────────────────────────────────
# Shell hooks triggered by events.
# [hooks]
# [[hooks.hooks]]
# name = "notify-on-complete"
# command = "/path/to/script.sh"
# events = ["agent.turn.complete"]
# timeout = 10
# [hooks.hooks.env]
# CUSTOM_VAR = "value"
"##
    )
}
