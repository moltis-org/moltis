//! Default configuration template with all options documented.
//!
//! This template is used when creating a new config file. It contains only
//! user overrides — built-in defaults live in `defaults.toml` (Moltis-managed)
//! and should not be duplicated here.
//!
//! Uncommenting a built-in default here creates a local override that shadows
//! future built-in updates on upgrade.

/// Generate the default config template with a specific port.
///
/// The template is override-only: only the installation-specific port is set
/// as an active value. All other settings are commented out with documentation
/// so users can see what's available without accidentally freezing defaults.
pub fn default_config_template(port: u16) -> String {
    format!(
        r##"# Moltis User Configuration
# =========================
# This file contains YOUR overrides only. Built-in defaults are in
# defaults.toml (Moltis-managed, regenerated on every startup).
#
# Uncomment and modify settings to override the built-in defaults.
# Changes require a restart to take effect.
#
# ⚠️  Uncommenting a built-in default here creates a local override that
#     shadows future built-in improvements on upgrade. Only uncomment
#     settings you intentionally want to control.
#
# Environment variable substitution is supported: ${{ENV_VAR}}
# Example: api_key = "${{ANTHROPIC_API_KEY}}"

# ══════════════════════════════════════════════════════════════════════════════
# SERVER
# ══════════════════════════════════════════════════════════════════════════════

[server]
port = {port}                           # Port number (auto-generated for this installation)
# bind = "127.0.0.1"                # Address to bind to ("0.0.0.0" for all interfaces)
# http_request_logs = false              # Enable verbose Axum HTTP request/response logs (debugging)
# ws_request_logs = false                # Enable WebSocket RPC request/response logs (debugging)
# terminal_enabled = true                # Enable interactive host terminal in Settings > Terminal
                                         # Set to false to disable the unsandboxed shell in the web UI.
                                         # NOTE: this can be re-enabled via the web UI config editor.
                                         # For hard lockdown, set MOLTIS_TERMINAL_DISABLED=1 (env var
                                         # takes precedence and cannot be changed from the web UI).
# update_releases_url = "https://www.moltis.org/releases.json"  # Override releases manifest URL
# external_url = "https://moltis.example.com"  # Public URL when behind a reverse proxy.
                                                 # Used for WebAuthn passkey origins.
                                                 # Env var MOLTIS_EXTERNAL_URL takes precedence.

# ══════════════════════════════════════════════════════════════════════════════
# UPSTREAM PROXY
# ══════════════════════════════════════════════════════════════════════════════
# Route all outbound traffic (providers, channels, tools, OAuth) through a
# proxy. Supports http://, https://, socks5://, socks5h:// schemes.
# Authentication via URL: "http://user:pass@host:port"
# When unset, reqwest honours HTTP_PROXY / HTTPS_PROXY / ALL_PROXY env vars.

# upstream_proxy = "http://127.0.0.1:1080"

# ══════════════════════════════════════════════════════════════════════════════
# AUTHENTICATION
# ══════════════════════════════════════════════════════════════════════════════

# [auth]
# disabled = false                  # true = disable auth entirely (DANGEROUS if exposed)
                                    # When disabled, anyone with network access can use moltis

# ══════════════════════════════════════════════════════════════════════════════
# GRAPHQL
# ══════════════════════════════════════════════════════════════════════════════

# [graphql]
# enabled = false                   # Enable GraphQL endpoint (/graphql for HTTP + WebSocket)
                                    # Can be toggled at runtime in Settings > GraphQL

# ══════════════════════════════════════════════════════════════════════════════
# TLS / HTTPS
# ══════════════════════════════════════════════════════════════════════════════

# [tls]
# enabled = true                    # Enable HTTPS (recommended)
# auto_generate = true              # Auto-generate local CA and server certificate
# http_redirect_port = 18790        # Optional override (default: server.port + 1)
# cert_path = "/path/to/cert.pem"   # Custom certificate file (overrides auto-gen)
# key_path = "/path/to/key.pem"     # Custom private key file
# ca_cert_path = "/path/to/ca.pem"  # CA certificate for trust instructions

# ══════════════════════════════════════════════════════════════════════════════
# AGENT IDENTITY
# ══════════════════════════════════════════════════════════════════════════════
# Customize your agent's personality. These are typically set during onboarding.

# [identity]
# name = "moltis"                   # Agent's display name
# emoji = "🦊"                      # Agent's emoji/avatar
# theme = "wise owl"                # Theme for agent personality (e.g. wise owl, chill fox)
# soul = ""                         # Freeform personality text injected into system prompt
                                    # Use this for custom instructions, tone, or behavior

# ══════════════════════════════════════════════════════════════════════════════
# USER PROFILE
# ══════════════════════════════════════════════════════════════════════════════
# Information about you. Set during onboarding.

# [user]
# name = "Your Name"                # Your name (used in conversations)
# timezone = "America/New_York"     # Your timezone (IANA format)

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
#   models    - Preferred models shown first (optional)
#   fetch_models - Discover models from provider API when available (default: true)
#   stream_transport - Streaming transport: "sse", "websocket", or "auto" (default: "sse")
#   alias     - Custom name for metrics labels (useful for multiple instances)
#   strict_tools - Force strict/non-strict tool schemas (default: auto-detect per provider)
#   policy    - Per-provider tool policy override (allow/deny lists)
#   model_overrides.<model_id>.context_window - Override context window for a specific model

# [providers]
# offered = ["local-llm", "lmstudio", "github-copilot", "openai-codex", "openai", "anthropic", "openrouter", "ollama", "moonshot", "minimax", "zai"]
                                    # Enabled providers and those shown in onboarding/picker UI ([] = enable/show all)
# show_legacy_models = true         # Show models older than 1 year in the chat model selector (they always appear in Settings)
# All available providers:
#   "anthropic", "openai", "gemini", "groq", "xai", "deepseek",
#   "fireworks", "mistral", "openrouter", "cerebras", "minimax",
#   "moonshot", "zai", "zai-code", "venice", "ollama", "lmstudio", "local-llm", "openai-codex",
#   "github-copilot", "kimi-code"

# ── Anthropic (Claude) ────────────────────────────────────────
# [providers.anthropic]
# enabled = true
# api_key = "sk-ant-..."                      # Or set ANTHROPIC_API_KEY env var
# models = ["claude-sonnet-4-5-20250929"]     # Optional preferred models
# fetch_models = true                          # Set false to skip remote discovery
# base_url = "https://api.anthropic.com"     # API endpoint
# alias = "anthropic"                         # Custom name for metrics
# cache_retention = "short"                    # Prompt caching: "none" | "short" | "long"
# policy.deny = ["exec"]                       # Deny specific tools when using this provider
# policy.allow = []                            # Restrict to only these tools (empty = all allowed)
# [providers.anthropic.model_overrides.claude-opus-4-6]
# context_window = 1_000_000                   # Provider-scoped model override

# ── OpenAI ────────────────────────────────────────────────────
# [providers.openai]
# enabled = true
# api_key = "sk-..."                          # Or set OPENAI_API_KEY env var
# models = ["gpt-5.3", "gpt-5.2"]            # Preferred models shown first
# fetch_models = true
# stream_transport = "sse"                     # "sse" | "websocket" | "auto"
# base_url = "https://api.openai.com/v1"     # API endpoint (change for Azure, etc.)
# alias = "openai"

# ── Google Gemini ─────────────────────────────────────────────
# [providers.gemini]
# enabled = true
# api_key = "..."                             # Or set GEMINI_API_KEY / GOOGLE_API_KEY env var
# models = ["gemini-2.5-flash-preview-05-20", "gemini-2.0-flash"]
# fetch_models = true
# base_url = "https://generativelanguage.googleapis.com/v1beta/openai"
# alias = "gemini"

# ── Groq ──────────────────────────────────────────────────────
# [providers.groq]
# enabled = true
# api_key = "..."                             # Or set GROQ_API_KEY env var
# models = ["llama-3.3-70b-versatile"]
# alias = "groq"

# ── DeepSeek ──────────────────────────────────────────────────
# [providers.deepseek]
# enabled = true
# api_key = "..."                             # Or set DEEPSEEK_API_KEY env var
# models = ["deepseek-chat"]
# base_url = "https://api.deepseek.com"
# alias = "deepseek"

# ── Fireworks ────────────────────────────────────────────────
# [providers.fireworks]
# enabled = true
# api_key = "..."                             # Or set FIREWORKS_API_KEY env var
# models = ["accounts/fireworks/routers/kimi-k2p5-turbo"]
# fetch_models = true                          # Set false to skip remote discovery
# base_url = "https://api.fireworks.ai/inference/v1"
# alias = "fireworks"

# ── xAI (Grok) ────────────────────────────────────────────────
# [providers.xai]
# enabled = true
# api_key = "..."                             # Or set XAI_API_KEY env var
# models = ["grok-3-mini"]
# alias = "xai"

# ── OpenRouter (multi-provider gateway) ───────────────────────
# [providers.openrouter]
# enabled = true
# api_key = "..."                             # Or set OPENROUTER_API_KEY env var
# models = ["anthropic/claude-3.5-sonnet"]    # Any model IDs on OpenRouter
# base_url = "https://openrouter.ai/api/v1"

# ── Moonshot (Kimi) ─────────────────────────────────────────
# [providers.moonshot]
# enabled = true
# api_key = "..."                             # Or set MOONSHOT_API_KEY env var
# models = ["kimi-k2.5"]                      # Preferred models shown first
# base_url = "https://api.moonshot.ai/v1"
# alias = "moonshot"

# ── Ollama ────────────────────────────────────────────────────
# [providers.ollama]
# base_url = "http://localhost:11434"
# models = ["llama3.2", "qwen2.5:7b"]         # Optional preferred models; installed models are discovered dynamically

# ── Local LLM ─────────────────────────────────────────────────
# [providers.local-llm]
# models = ["qwen2.5-coder-7b-q4_k_m"]        # Optional; configure local models in onboarding
# idle_timeout_secs = 300                      # Auto-unload local models after 5 minutes of inactivity (per-model overrides in local-llm.json)

# ══════════════════════════════════════════════════════════════════════════════
# MODEL OVERRIDES (GLOBAL)
# ══════════════════════════════════════════════════════════════════════════════
# Override context window sizes for specific models across all providers.
# Provider-scoped overrides ([providers.<name>.model_overrides.<id>]) take precedence.
#
# [models.claude-opus-4-6]
# context_window = 1_000_000                  # Override the built-in heuristic
#
# [models.glm-5-turbo]
# context_window = 200_000

# ══════════════════════════════════════════════════════════════════════════════
# CHAT SETTINGS
# ══════════════════════════════════════════════════════════════════════════════

# [chat]
# message_queue_mode = "followup"   # How to handle messages during an active agent run:
                                    #   "followup" - Queue messages, replay one-by-one after run
                                    #   "collect"  - Buffer messages, concatenate as single message
# prompt_memory_mode = "live-reload"  # How MEMORY.md reaches the prompt:
                                      #   "live-reload"            - Re-read MEMORY.md before each turn
                                      #   "frozen-at-session-start" - Freeze the first MEMORY.md snapshot per session
# workspace_file_max_chars = 32000  # Optional: per-file prompt cap for AGENTS.md / TOOLS.md before truncation.
# priority_models = ["claude-opus-4-5", "gpt-5.2", "gemini-3-flash"]  # Optional: models to pin first in selectors

# ── Compaction ─────────────────────────────────────────────────────────────
# Strategy used to shrink a session when its context window fills up, or when
# a user invokes `/compact`. Four modes are available — pick the one that
# matches your cost/fidelity trade-off. See docs/src/compaction.md for a full
# comparison table and picking guide.
#
# Modes:
#   "deterministic"        (default) Zero LLM calls. Fast, free, offline.
#   "recency_preserving"   Zero LLM calls. Keeps head + tail, collapses middle.
#   "structured"           Head + LLM summary + tail. Highest fidelity.
#   "llm_replace"          Full LLM summary replacement.
#
# [chat.compaction]
# mode = "deterministic"              # "deterministic" | "recency_preserving" | "structured" | "llm_replace"
# threshold_percent = 0.95            # Auto-compaction threshold (fraction of context window)
# protect_head = 3                    # Leading messages kept verbatim
# protect_tail_min = 20               # Floor for tail messages kept verbatim
# tail_budget_ratio = 0.20            # Tail size as fraction of threshold × context window
# tool_prune_char_threshold = 200     # Prune tool results longer than this in middle region
# show_settings_hint = true           # Append compaction mode hint to notices

# ══════════════════════════════════════════════════════════════════════════════
# SUB-AGENT SPAWN PRESETS
# ══════════════════════════════════════════════════════════════════════════════
# Configure reusable presets for sub-agents spawned via the `spawn_agent` tool.
#
# ⚠️  SCOPE: `[agents.presets.*]` applies ONLY to sub-agents spawned via the
# `spawn_agent` tool. The `tools.allow` / `tools.deny` fields under a preset
# do NOT filter tools for the main agent session. To allow/deny tools for the
# main session, use the `[tools.policy]` section further down this file.
#
# [agents]
# default_preset = "research"      # Sub-agent preset used when spawn_agent.preset is omitted
#
# Built-in agent presets (research, coder, reviewer, qa, ux, docs, coordinator)
# live in defaults.toml. Uncomment and modify below to override a preset,
# or add your own custom presets.
#
# [agents.presets.research]
# identity.name = "Researcher"
# identity.theme = "thorough, skeptical, and evidence-oriented"
# system_prompt_suffix = "..."
# max_iterations = 16

# ══════════════════════════════════════════════════════════════════════════════
# SESSION MODES
# ══════════════════════════════════════════════════════════════════════════════
# Modes are temporary per-session prompt overlays selected with `/mode`.
# They do not create chat agents, do not affect sub-agent presets, and do not
# change an agent's identity or memory. Built-ins include concise, technical,
# creative, teacher, plan, build, review, research, and elevated.
#
# [modes.presets.concise]
# name = "Concise"
# description = "short direct answers"
# prompt = "Keep answers short, concrete, and caveat-light unless the user asks for detail."
#
# [modes.presets.incident]
# name = "Incident"
# description = "production incident response"
# prompt = "Prioritize impact, timeline, mitigation, rollback, logs, and clear status updates."

# ══════════════════════════════════════════════════════════════════════════════
# TOOLS
# ══════════════════════════════════════════════════════════════════════════════

# [tools]
# agent_timeout_secs = 600          # Max seconds for an agent run (0 = no timeout)
# agent_max_iterations = 25         # Max LLM/tool loop iterations before stopping
# agent_max_auto_continues = 2      # Auto-continue nudges when model stops mid-task (0 = off)
# agent_auto_continue_min_tool_calls = 3  # Min tool calls before auto-continue can trigger
# max_tool_result_bytes = 50000     # Max bytes per tool result before truncation (50KB)
# registry_mode = "full"            # "full" = all schemas every turn, "lazy" = tool_search discovery
# agent_loop_detector_window = 2    # Fire intervention after N identical failing tool calls in a row
# tool_result_compaction_ratio = 75 # % of context_window before oldest tool results are compacted
# preemptive_overflow_ratio = 90    # % of context_window before hard ContextWindowExceeded error

# ── Maps ─────────────────────────────────────────────────────────────────────

# [tools.maps]
# provider = "google_maps"          # "google_maps" | "apple_maps" | "openstreetmap"

# ── Native filesystem tools (Read/Write/Edit/MultiEdit/Glob/Grep) ─────────────
# All fields are optional. Defaults are conservative — the fs tools work
# out of the box with no configuration.

# [tools.fs]
# workspace_root = "/home/user/projects/my-app"  # Default search root for Glob/Grep
# allow_paths = []                  # Absolute path globs the fs tools are allowed to access
# deny_paths = []                   # Absolute path globs the fs tools must refuse
# track_reads = false               # Record per-session Read history
# must_read_before_write = false    # Refuse Write/Edit targeting unread files (needs track_reads)
# require_approval = true           # Pause Write/Edit for operator approval
# max_read_bytes = 10485760         # Max bytes per Read (10 MB)
# binary_policy = "reject"          # "reject" or "base64"
# respect_gitignore = true          # Skip .gitignored files in Glob/Grep
# checkpoint_before_mutation = false # Snapshot files before Write/Edit

# ── Command Execution ─────────────────────────────────────────────────────────

# [tools.exec]
# default_timeout_secs = 30         # Default timeout for commands
# max_output_bytes = 204800         # Max command output bytes (200KB)
# approval_mode = "on-miss"         # "always" | "on-miss" | "never"
# security_level = "allowlist"      # "permissive" | "allowlist" | "strict"
# allowlist = []                    # Command patterns to allow. Example: ["git *", "npm *"]
# host = "local"                    # "local" | "node" | "ssh"
# node = "mac-mini"                 # Default node when host = "node"
# ssh_target = "deploy@box"         # SSH target when host = "ssh"

# ── Sandbox Configuration ─────────────────────────────────────────────────────
# Commands run inside isolated containers for security.

# [tools.exec.sandbox]
# mode = "all"                      # "off" | "non-main" | "all" (recommended)
# scope = "session"                 # "command" | "session" (recommended) | "global"
# workspace_mount = "ro"            # "ro" | "rw" | "none"
# home_persistence = "shared"       # "off" | "session" | "shared"
# backend = "auto"                  # "auto" | "docker" | "apple-container"
# no_network = true                 # Disable network access in sandbox
# image = "custom-image:tag"        # Custom Docker image (default: auto-built)
# packages = [...]                  # Packages installed in sandbox containers

# [tools.exec.sandbox.resource_limits]
# memory_limit = "512M"             # Memory limit (e.g., "512M", "1G")
# cpu_quota = 0.5                   # CPU quota as fraction
# pids_max = 100                    # Maximum number of processes

# ── Tool Policy ───────────────────────────────────────────────────────────────
# Control which tools the agent can use. Policies are layered (later wins for
# allow, deny always accumulates across layers):
#
#   1. Global        — [tools.policy]
#   2. Per-provider  — [providers.<name>.policy]
#   3. Per-agent     — [agents.presets.<id>.tools]
#   4. Per-channel   — [channels.<type>.<account>.tools.groups.<chat_type>]
#   5. Per-sender    — [...groups.<chat_type>.by_sender.<sender_id>]
#   6. Sandbox       — [tools.exec.sandbox.tools_policy]

# [tools.policy]
# allow = []                        # Tools to always allow (e.g., ["exec", "web_fetch"])
# deny = []                         # Tools to always deny (e.g., ["browser"])

# ── Web Search ────────────────────────────────────────────────────────────────

# [tools.web.search]
# enabled = true                    # Enable web search tool
# provider = "brave"                # "brave" or "perplexity"
# max_results = 5                   # Number of results to return (1-10)
# timeout_seconds = 30              # HTTP request timeout
# cache_ttl_minutes = 15            # Cache results (0 = no cache)
# duckduckgo_fallback = false       # Enable DDG fallback without API keys
# api_key = "..."                   # Brave API key (or set BRAVE_API_KEY env var)

# [tools.web.search.perplexity]
# api_key = "..."                   # Or set PERPLEXITY_API_KEY env var
# model = "sonar"                   # Perplexity model to use

# ── Web Fetch ─────────────────────────────────────────────────────────────────

# [tools.web.fetch]
# enabled = true                    # Enable web fetch tool
# max_chars = 50000                 # Max characters to return
# timeout_seconds = 30              # HTTP request timeout
# cache_ttl_minutes = 15            # Cache fetched pages (0 = no cache)
# max_redirects = 3                 # Maximum HTTP redirects
# readability = true                # Use readability extraction for HTML
# ssrf_allowlist = ["172.22.0.0/16"] # CIDR ranges exempt from SSRF blocking

# ── Firecrawl (API-based web scraping) ────────────────────────────────────────

# [tools.web.firecrawl]
# enabled = false
# api_key = "fc-..."                # Or set FIRECRAWL_API_KEY env var
# base_url = "https://api.firecrawl.dev"

# ── Browser Automation ────────────────────────────────────────────────────────

# [tools.browser]
# enabled = true                    # Enable browser tool
# headless = true                   # Run without visible window
# viewport_width = 2560             # Default viewport width in pixels
# viewport_height = 1440            # Default viewport height
# device_scale_factor = 2.0         # HiDPI/Retina scaling
# max_instances = 3                 # Maximum concurrent browser instances
# idle_timeout_secs = 300           # Close idle browsers after this many seconds
# sandbox = false                   # Run browser in container for isolation
# allowed_domains = []              # Domain restrictions (empty = all allowed)
# chrome_path = "/path/to/chrome"   # Custom Chrome binary path

# ══════════════════════════════════════════════════════════════════════════════
# SKILLS
# ══════════════════════════════════════════════════════════════════════════════

# [skills]
# enabled = true                    # Enable skills system
# search_paths = []                 # Additional directories to search for skills
# auto_load = []                    # Skills to always load

# ══════════════════════════════════════════════════════════════════════════════
# MCP SERVERS
# ══════════════════════════════════════════════════════════════════════════════
# Model Context Protocol servers provide additional tools and capabilities.
# See https://modelcontextprotocol.io for available servers.

# [mcp]
# request_timeout_secs = 30         # Default timeout for MCP requests

# [mcp.servers.server-name]
# command = "npx"                   # Command to run (for stdio transport)
# args = ["-y", "@package/name"]    # Command arguments
# env = {{ KEY = "value" }}           # Environment variables
# transport = "stdio"               # "stdio" | "sse" | "streamable-http"

# ══════════════════════════════════════════════════════════════════════════════
# METRICS
# ══════════════════════════════════════════════════════════════════════════════

# [metrics]
# enabled = true                    # Enable metrics collection
# prometheus_endpoint = true        # Expose /metrics endpoint

# ══════════════════════════════════════════════════════════════════════════════
# CRON
# ══════════════════════════════════════════════════════════════════════════════

# [cron]
# rate_limit_max = 10
# rate_limit_window_secs = 60
# session_retention_days = 7

# ══════════════════════════════════════════════════════════════════════════════
# HEARTBEAT
# ══════════════════════════════════════════════════════════════════════════════

# [heartbeat]
# enabled = true                    # Enable periodic heartbeats
# every = "30m"                     # Interval (e.g., "30m", "1h", "6h")
# ack_max_chars = 300               # Max characters for acknowledgment reply
# deliver = false                   # Deliver heartbeat replies to a channel
# sandbox_enabled = true            # Run heartbeat commands in sandbox
# wake_cooldown = "5m"              # Min duration between exec-triggered heartbeat wakes (0 to disable)

# [heartbeat.active_hours]
# start = "08:00"
# end = "24:00"
# timezone = "local"                # "local" or IANA name like "Europe/Paris"

# ══════════════════════════════════════════════════════════════════════════════
# FAILOVER
# ══════════════════════════════════════════════════════════════════════════════

# [failover]
# enabled = true                    # Enable automatic failover
# fallback_models = []              # Ordered list of fallback models

# ══════════════════════════════════════════════════════════════════════════════
# VOICE
# ══════════════════════════════════════════════════════════════════════════════

# [voice.tts]
# enabled = true
# providers = ["openai", "elevenlabs"]  # UI allowlist (empty = show all)

# [voice.stt]
# enabled = true
# providers = ["whisper", "mistral", "elevenlabs"]  # UI allowlist (empty = show all)

# ══════════════════════════════════════════════════════════════════════════════
# NGROK
# ══════════════════════════════════════════════════════════════════════════════

# [ngrok]
# enabled = false
# authtoken = "${{NGROK_AUTHTOKEN}}"
# domain = "team-gateway.ngrok.app"

# ══════════════════════════════════════════════════════════════════════════════
# TAILSCALE
# ══════════════════════════════════════════════════════════════════════════════

# [tailscale]
# mode = "off"                      # "off" | "serve" | "funnel"
# reset_on_exit = true

# ══════════════════════════════════════════════════════════════════════════════
# MEMORY / EMBEDDINGS
# ══════════════════════════════════════════════════════════════════════════════

# [memory]
# style = "hybrid"                  # "hybrid" | "prompt-only" | "search-only" | "off"
# agent_write_mode = "hybrid"       # "hybrid" | "prompt-only" | "search-only" | "off"
# backend = "builtin"               # "builtin" | "qmd"
# provider = "auto"                 # "local" | "ollama" | "openai" | "custom"

# ══════════════════════════════════════════════════════════════════════════════
# CHANNELS
# ══════════════════════════════════════════════════════════════════════════════
# External messaging integrations.
# Note: channels added in the web UI are stored in data_dir()/moltis.db,
# not in this file. Keep channel config here only for manual TOML management.

# [channels]
# offered = ["telegram", "whatsapp", "msteams", "discord", "slack", "matrix", "nostr", "signal"]

# See docs or defaults.toml for full channel configuration examples
# (WhatsApp, Telegram, Teams, Discord, Slack, Matrix, Nostr, Signal).

# ══════════════════════════════════════════════════════════════════════════════
# HOOKS
# ══════════════════════════════════════════════════════════════════════════════

# [hooks]
# [[hooks.hooks]]
# name = "my-hook"
# command = "/path/to/handler.sh"
# events = ["BeforeToolCall", "AfterToolCall"]
# timeout = 10

# ══════════════════════════════════════════════════════════════════════════════
# ENVIRONMENT VARIABLES
# ══════════════════════════════════════════════════════════════════════════════
# Variables injected into the Moltis process at startup.

# [env]
# BRAVE_API_KEY = "..."
# OPENROUTER_API_KEY = "sk-or-..."
"##
    )
}
