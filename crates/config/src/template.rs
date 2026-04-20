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
terminal_enabled = true                   # Enable interactive host terminal in Settings > Terminal
                                          # Set to false to disable the unsandboxed shell in the web UI.
                                          # NOTE: this can be re-enabled via the web UI config editor.
                                          # For hard lockdown, set MOLTIS_TERMINAL_DISABLED=1 (env var
                                          # takes precedence and cannot be changed from the web UI).
update_releases_url = "https://www.moltis.org/releases.json"    # Releases manifest URL for update checks (override to use a custom URL)
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

[auth]
disabled = false                  # true = disable auth entirely (DANGEROUS if exposed)
                                  # When disabled, anyone with network access can use moltis

# ══════════════════════════════════════════════════════════════════════════════
# GRAPHQL
# ══════════════════════════════════════════════════════════════════════════════

[graphql]
enabled = false                   # Enable GraphQL endpoint (/graphql for HTTP + WebSocket)
                                  # Can be toggled at runtime in Settings > GraphQL

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
# theme = "wise owl"              # Theme for agent personality (e.g. wise owl, chill fox)
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
#   models    - Preferred models shown first (optional)
#   fetch_models - Discover models from provider API when available (default: true)
#   stream_transport - Streaming transport: "sse", "websocket", or "auto" (default: "sse")
#   alias     - Custom name for metrics labels (useful for multiple instances)
#   strict_tools - Force strict/non-strict tool schemas (default: auto-detect per provider)
#   policy    - Per-provider tool policy override (allow/deny lists)
#   model_overrides.<model_id>.context_window - Override context window for a specific model

[providers]
offered = ["local-llm", "lmstudio", "github-copilot", "openai-codex", "openai", "anthropic", "openrouter", "ollama", "moonshot", "minimax", "zai"] # Enabled providers and those shown in onboarding/picker UI ([] = enable/show all)
# show_legacy_models = true  # Show models older than 1 year in the chat model selector (they always appear in Settings)
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
[providers.openai]
# enabled = true
# api_key = "sk-..."                          # Or set OPENAI_API_KEY env var
models = ["gpt-5.3", "gpt-5.2"]              # Preferred models shown first
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
[providers.moonshot]
# enabled = true
# api_key = "..."                             # Or set MOONSHOT_API_KEY env var
models = ["kimi-k2.5"]                        # Preferred models shown first
# base_url = "https://api.moonshot.ai/v1"
# alias = "moonshot"

[providers.ollama]
# base_url = "http://localhost:11434"
# models = ["llama3.2", "qwen2.5:7b"]         # Optional preferred models; installed models are discovered dynamically

[providers.local-llm]
# models = ["qwen2.5-coder-7b-q4_k_m"]        # Optional; configure local models in onboarding

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

[chat]
message_queue_mode = "followup"   # Default: process queued messages one-by-one after the current run.
                                  # How to handle messages during an active agent run:
                                  #   "followup" - Queue messages, replay one-by-one after run
                                  #   "collect"  - Buffer messages, concatenate as single message
# prompt_memory_mode = "live-reload"  # How MEMORY.md reaches the prompt:
                                      #   "live-reload"            - Re-read MEMORY.md before each turn
                                      #   "frozen-at-session-start" - Freeze the first MEMORY.md snapshot per session
# workspace_file_max_chars = 32000  # Optional: per-file prompt cap for AGENTS.md / TOOLS.md before truncation.
# priority_models = ["claude-opus-4-5", "gpt-5.2", "gemini-3-flash"]  # Optional: models to pin first in selectors
# allowed_models = ["gpt 5.2"]  # Legacy field (currently ignored).

# ── Compaction ─────────────────────────────────────────────────────────────
# Strategy used to shrink a session when its context window fills up, or when
# a user invokes `/compact`. Four modes are available — pick the one that
# matches your cost/fidelity trade-off. See docs/src/compaction.md for a full
# comparison table and picking guide.
#
# Modes:
#   "deterministic"        (default) Zero LLM calls. Replaces the entire
#                          history with a single extracted summary message
#                          (counts, tool names, file paths, recent user
#                          requests, head+tail timeline). Fast, free, offline,
#                          low fidelity. Best for short chat channels and
#                          cost-sensitive deployments.
#
#   "recency_preserving"   Zero LLM calls. Keeps head (system prompt +
#                          first exchange) and recent tail verbatim (sized
#                          by token budget). Collapses the middle into a
#                          short marker message and replaces any bulky
#                          tool-result content in the retained slice with a
#                          placeholder. Repairs orphaned tool_use /
#                          tool_result pairs so strict providers accept the
#                          retry. Mid fidelity, free.
#
#   "structured"           Head + LLM structured summary + tail. Same
#                          boundary logic as recency_preserving, but the
#                          middle is summarised with a single LLM call
#                          using a Goal / Progress / Decisions / Files /
#                          Next Steps template (same convention as
#                          hermes-agent and openclaw safeguard). Iterative
#                          re-compaction preserves prior summary sections.
#                          Highest fidelity, costs a summary LLM call per
#                          compaction. Falls back to recency_preserving on
#                          LLM failure or empty summary.
#
#   "llm_replace"          Replaces the entire history with a single
#                          LLM-generated summary. Pre-PR-#653 behaviour.
#                          Best for maximum token reduction when the session
#                          provider is cheap and the tail isn't worth
#                          preserving.
#
[chat.compaction]
mode = "deterministic"              # "deterministic" | "recency_preserving" | "structured" | "llm_replace"
# threshold_percent = 0.95          # Fires auto-compaction when the next request is estimated to exceed
                                    # this fraction of the model context window. Also multiplied into
                                    # the verbatim tail budget for recency_preserving / structured modes.
                                    # Default matches the pre-PR-#653 hardcoded trigger so upgrades are
                                    # behaviour-neutral. Range: 0.10–0.95. Lower = more aggressive compaction.
# protect_head = 3                  # Number of leading messages kept verbatim by recency/structured modes.
# protect_tail_min = 20             # Floor for tail messages kept verbatim (recency/structured modes).
# tail_budget_ratio = 0.20          # Size of the verbatim tail as a fraction of threshold_percent × context_window.
                                    # Example (defaults): 200K context × 0.95 × 0.20 = 38K tokens of tail preserved.
# tool_prune_char_threshold = 200   # Tool-result content longer than this is replaced with a placeholder
                                    # when recency/structured modes prune the middle region.
# summary_model = "openrouter/google/gemini-2.5-flash"  # RESERVED — auxiliary-model subsystem not yet wired
                                                         # (tracked by beads issue moltis-8me). Setting this
                                                         # today emits a one-shot runtime WARN and has no
                                                         # effect on the structured / llm_replace strategies,
                                                         # which still use the session's primary provider.
# max_summary_tokens = 4096         # RESERVED — see summary_model above. Default is 4096 but the value
                                    # is not yet applied to the streaming summary call.
# show_settings_hint = true         # Append "Change chat.compaction.mode in moltis.toml…" to every
                                    # compaction notice (web UI card + channel messages). Default: true.
                                    # Set to false once you know the setting exists to hide the repetitive
                                    # footer without losing the mode + token metadata.

# ══════════════════════════════════════════════════════════════════════════════
# SUB-AGENT SPAWN PRESETS (OPTIONAL)
# ══════════════════════════════════════════════════════════════════════════════
# Configure reusable presets for sub-agents spawned via the `spawn_agent` tool.
#
# ⚠️  SCOPE: `[agents.presets.*]` applies ONLY to sub-agents spawned via the
# `spawn_agent` tool. The `tools.allow` / `tools.deny` fields under a preset
# do NOT filter tools for the main agent session. To allow/deny tools for the
# main session, use the `[tools.policy]` section further down this file.
# `[agents] default_preset` likewise only selects the sub-agent preset used
# when `spawn_agent.preset` is omitted — it does not apply to the main session.
#
# [agents]
# default_preset = "research"      # Sub-agent preset used when spawn_agent.preset is omitted
#
# [agents.presets.research]
# model = "openai/gpt-5.2"
# allow_tools = ["web_search", "web_fetch", "sessions_send", "task_list"]
# deny_tools = ["exec"]
# delegate_only = false
# system_prompt_suffix = "Focus on gathering and summarizing evidence."

# ══════════════════════════════════════════════════════════════════════════════
# TOOLS
# ══════════════════════════════════════════════════════════════════════════════

[tools]
agent_timeout_secs = 600          # Max seconds for an agent run (0 = no timeout)
agent_max_iterations = 25         # Max LLM/tool loop iterations before stopping
agent_max_auto_continues = 2      # Auto-continue nudges when model stops mid-task (0 = off)
agent_auto_continue_min_tool_calls = 3  # Min tool calls before auto-continue can trigger
max_tool_result_bytes = 50000     # Max bytes per tool result before truncation (50KB)
# registry_mode = "full"          # "full" = all schemas every turn, "lazy" = tool_search discovery
agent_loop_detector_window = 3    # Fire intervention after N identical failing tool calls in a row (0 = disable)
agent_loop_detector_strip_tools_on_second_fire = true  # On second consecutive fire, strip tool schemas for one turn to force a text response
tool_result_compaction_ratio = 75     # % of context_window before oldest tool results are compacted (0 = disable)
preemptive_overflow_ratio = 90        # % of context_window before hard ContextWindowExceeded error
compaction_min_iterations = 3         # Min loop iterations before per-iteration compaction can fire

# ── Maps ─────────────────────────────────────────────────────────────────────

[tools.maps]
provider = "google_maps"          # Map provider used by show_map:
                                  #   "google_maps" (default)
                                  #   "apple_maps"
                                  #   "openstreetmap"

# ── Native filesystem tools (Read/Write/Edit/MultiEdit/Glob/Grep) ─────────────
# All fields are optional. Defaults are conservative — the fs tools work
# out of the box with no configuration.

[tools.fs]
# workspace_root = "/home/user/projects/my-app"
                                  # Absolute path used as the default search root
                                  # for Glob/Grep when the LLM call omits `path`.
                                  # Unset → calls without an explicit `path` are
                                  # rejected with a clear error.
allow_paths = []                  # Absolute path globs the fs tools are allowed
                                  # to access. Empty = no allowlist (all paths OK).
                                  # Evaluated after canonicalization so symlinks
                                  # can't escape the list.
                                  # Example: ["/data/home/*", "/srv/workspace/**"]
deny_paths = []                   # Absolute path globs the fs tools must refuse.
                                  # Deny wins over allow. Also evaluated after
                                  # canonicalization. Example:
                                  #   ["/data/secrets/**", "**/.env*", "/etc/**"]
track_reads = false               # Record per-session Read history for loop
                                  # detection and the must_read_before_write
                                  # invariant. Cheap in-memory map keyed by
                                  # session_key.
must_read_before_write = false    # Refuse Write/Edit/MultiEdit calls targeting
                                  # files the session has not previously Read.
                                  # Requires track_reads = true. Returns a typed
                                  # must_read_before_write payload the LLM can
                                  # branch on.
require_approval = true           # When true, Write/Edit/MultiEdit pause for
                                  # explicit operator approval before mutating.
                                  # Uses the existing approval queue and will
                                  # time out if nobody approves the request.
max_read_bytes = 10485760         # Maximum bytes a single Read can return before
                                  # the file is rejected with a typed "too_large"
                                  # payload (10 MB).
binary_policy = "reject"          # How to handle binary files in Read:
                                  #   "reject" — return typed binary marker (default)
                                  #   "base64" — return base64-encoded bytes in the
                                  #              payload (still capped by max_read_bytes)
respect_gitignore = true          # When true (default), Glob and Grep skip files
                                  # ignored by .gitignore / .ignore / .git/info/exclude
                                  # while walking. Set false to include ignored files.
checkpoint_before_mutation = false  # When true, Write/Edit/MultiEdit call the
                                  # existing CheckpointManager to snapshot the target
                                  # file before mutating, so the LLM (or operator) can
                                  # restore it later via `checkpoint_restore`. Default
                                  # off because checkpoints grow with activity and
                                  # size scales with file size.
# context_window_tokens = 200000    # Model context window in tokens. When set, Read's
                                  # per-call byte cap scales adaptively so a single
                                  # Read uses at most ~20% of the model's working set
                                  # (clamped to [50 KB, 512 KB]). Unset → fixed 256 KB.
                                  # Typical values: 200000 for Claude Sonnet, 1000000
                                  # for Claude Opus 4.6 1M, 128000 for GPT-4 Turbo.

# ── Command Execution ─────────────────────────────────────────────────────────

[tools.exec]
default_timeout_secs = 30         # Default timeout for commands
max_output_bytes = 204800         # Max command output bytes (200KB)
approval_mode = "on-miss"         # When to require approval:
                                  #   "always"  - Always ask before running
                                  #   "on-miss" - Ask if not in allowlist
                                  #   "never"   - Never ask (for headless deployments)
security_level = "allowlist"      # Security mode:
                                  #   "permissive" - Allow most commands
                                  #   "allowlist"  - Only allow listed commands
                                  #   "strict"     - Very restrictive
allowlist = []                    # Command patterns to allow (when security_level = "allowlist").
                                  # With approval_mode = "never", a non-empty allowlist is enforced:
                                  # commands that don't match are denied (safe bins still allowed).
                                  # An empty allowlist in "never" mode is unrestricted.
                                  # Example: ["git *", "npm *", "cargo *"]
host = "local"                    # Where to run commands:
                                  #   "local" - Run on this machine (default)
                                  #   "node"  - Run on a connected Moltis node
                                  #   "ssh"   - Run through the system ssh client
# node = "mac-mini"               # Default node id/display name when host = "node"
# ssh_target = "deploy@box"       # SSH host alias or user@host when host = "ssh"

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
                                  #   "ro"   - Read-only (safe; sandbox commands can read mounted files but cannot modify them)
                                  #   "rw"   - Read-write (can modify files)
                                  #   "none" - No mount
# host_data_dir = "/host/path/data"  # Optional override if auto-detection cannot resolve the host-visible data dir
home_persistence = "shared"       # Persist /home/sandbox across container recreation:
                                  #   "off"     - Ephemeral home
                                  #   "session" - Per-session persisted home
                                  #   "shared"  - One shared persisted home (default)
# shared_home_dir = "/path/to/shared-home"  # Host dir for shared persistence (default: data_dir()/sandbox/home/shared)
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
    "golang-go",
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
    "tmux",
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

# Tool policy overrides applied when commands run inside the sandbox (layer 6).
# These narrow the effective policy for sandboxed execution only.
# [tools.exec.sandbox.tools_policy]
# allow = ["exec"]                # Only allow exec tool inside sandbox
# deny = ["browser"]              # Deny browser tool inside sandbox

# ── Tool Policy ───────────────────────────────────────────────────────────────
# Control which tools the agent can use. Policies are layered (later wins for
# allow, deny always accumulates across layers):
#
#   1. Global        — this section ([tools.policy])
#   2. Per-provider  — [providers.<name>.policy]
#   3. Per-agent     — [agents.presets.<id>.tools]
#   4. Per-channel   — [channels.<type>.<account>.tools.groups.<chat_type>]
#   5. Per-sender    — [...groups.<chat_type>.by_sender.<sender_id>]
#   6. Sandbox       — [tools.exec.sandbox.tools_policy] (only when sandboxed)
#
# This is the base policy for all sessions. Provider and channel layers
# narrow it further based on runtime context.

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
duckduckgo_fallback = false       # Off by default; enable only if you want DDG fallback without API keys
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
# ssrf_allowlist = ["172.22.0.0/16"] # CIDR ranges exempt from SSRF blocking (e.g. Docker networks)

# ── Firecrawl (API-based web scraping) ────────────────────────────────────────
# High-quality markdown extraction from web pages, including JS-heavy and
# bot-protected sites.  Used as a standalone firecrawl_scrape tool, as a
# web_search provider, and as a fallback extractor in web_fetch.
# Get an API key at https://firecrawl.dev or self-host.

# [tools.web.firecrawl]
# enabled = false                        # Enable Firecrawl integration
# api_key = "fc-..."                     # Or set FIRECRAWL_API_KEY env var
# base_url = "https://api.firecrawl.dev" # API endpoint (change for self-hosted)
# only_main_content = true               # Strip navs, footers, sidebars
# timeout_seconds = 30                   # HTTP request timeout
# cache_ttl_minutes = 15                 # Cache scraped pages (0 = no cache)
# web_fetch_fallback = true              # Use as fallback when readability fails

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
# container_host = "127.0.0.1"   # Host/IP to reach browser container (default: localhost)
                                  # Set to "host.docker.internal" when Moltis runs inside Docker
# browserless_api_version = "v1" # Browserless compatibility mode: "v1" (default) or "v2"
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
                                  # Default locations include ~/.moltis/skills/
auto_load = []                    # Skills to always load without explicit activation
                                  # Example: ["code-review", "commit"]
enable_agent_sidecar_files = false # Allow agents to write supplementary text files inside personal skill dirs
enable_self_improvement = true    # Include system prompt guidance encouraging autonomous skill creation/update

# ══════════════════════════════════════════════════════════════════════════════
# MCP SERVERS
# ══════════════════════════════════════════════════════════════════════════════
# Model Context Protocol servers provide additional tools and capabilities.
# See https://modelcontextprotocol.io for available servers.

[mcp]
request_timeout_secs = 30        # Default timeout for MCP requests
# Each server has a name and configuration:
#
# [mcp.servers.server-name]
# command = "npx"                 # Command to run (for stdio transport)
# args = ["-y", "@package/name"]  # Command arguments
# env = {{ KEY = "value" }}         # Environment variables for the process
# enabled = true                  # Whether this server is enabled
# request_timeout_secs = 90       # Optional timeout override for this server
# transport = "stdio"             # Transport: "stdio" (default), "sse", or "streamable-http"
# url = "http://..."              # URL for SSE/Streamable HTTP transport
# headers = {{ Authorization = "Bearer ${{TOKEN}}" }}  # Optional HTTP headers for remote transport

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
# url = "http://localhost:8080/mcp?api_key=$REMOTE_MCP_KEY"
# headers = {{ "x-api-key" = "${{REMOTE_MCP_KEY}}" }}
# enabled = true

# Example: Streamable HTTP server
# [mcp.servers.remote-http]
# transport = "streamable-http"
# url = "https://mcp.example.com/mcp"
# headers = {{ Authorization = "Bearer ${{API_KEY}}" }}
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
# CRON
# ══════════════════════════════════════════════════════════════════════════════
# Settings for the cron scheduler.

# [cron]
# rate_limit_max = 10                # Max jobs created per rate limit window
# rate_limit_window_secs = 60        # Rate limit window in seconds
# session_retention_days = 7         # Auto-clean cron sessions older than N days (0 or omit to disable)
# auto_prune_cron_containers = true  # Auto-remove sandbox containers after cron job completion

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
deliver = false                   # Deliver heartbeat replies to a channel account
# channel = "my-bot"              # Channel account identifier (required when deliver = true)
# to = "123456789"                # Chat/recipient ID (required when deliver = true)
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
# VOICE
# ══════════════════════════════════════════════════════════════════════════════
# Voice provider settings for text-to-speech (TTS) and speech-to-text (STT).
# `providers` controls what appears in the Settings UI provider list.

[voice.tts]
enabled = true                    # Enable text-to-speech
# provider = "openai"             # Active TTS provider (auto-selects first configured if omitted)
providers = ["openai", "elevenlabs"] # UI allowlist (empty = show all TTS providers)
# All available TTS providers:
#   "openai", "elevenlabs", "google", "piper", "coqui"

[voice.stt]
enabled = true                    # Enable speech-to-text
# provider = "whisper"            # Active STT provider (auto-selects first configured if omitted)
providers = ["whisper", "mistral", "elevenlabs"] # UI allowlist (empty = show all STT providers)
# All available STT providers:
#   "whisper", "groq", "deepgram", "google", "mistral",
#   "voxtral-local", "whisper-cli", "sherpa-onnx", "elevenlabs-stt"

# No api_key needed for OpenAI TTS/Whisper when OpenAI is configured as an LLM provider.
# [voice.tts.openai]
# base_url = "https://api.openai.com/v1"     # API endpoint (change for Chatterbox, etc.)
# voice = "alloy"                 # alloy, echo, fable, onyx, nova, shimmer
# model = "tts-1"                 # tts-1 or tts-1-hd

# [voice.stt.whisper]
# model = "whisper-1"             # default; override when using an OpenAI-compatible proxy
# language = "en"                 # Optional ISO 639-1 hint; omit for auto-detect

# ══════════════════════════════════════════════════════════════════════════════
# NGROK
# ══════════════════════════════════════════════════════════════════════════════
# Expose moltis through a public HTTPS tunnel managed by ngrok.
# Requires a build with the `ngrok` feature and an ngrok authtoken.

[ngrok]
enabled = false                   # true = create a public HTTPS tunnel at startup
# authtoken = "${{NGROK_AUTHTOKEN}}" # Optional if NGROK_AUTHTOKEN env var is already set
# domain = "team-gateway.ngrok.app"  # Optional reserved/static ngrok domain

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
# style = "hybrid"               # High-level memory behavior (default = "hybrid"):
                                  #   "hybrid"      - Prompt MEMORY.md + memory_search/memory_get/memory_save
                                  #   "prompt-only" - Prompt MEMORY.md only, no memory tools
                                  #   "search-only" - Memory tools only, no prompt MEMORY.md injection
                                  #   "off"         - Disable prompt memory injection and memory tools
#                                  # Prompt freshness is configured separately in [chat].prompt_memory_mode
# agent_write_mode = "hybrid"    # Where agent-authored memory writes can land (default = "hybrid"):
                                  #   "hybrid"      - Allow MEMORY.md and memory/<name>.md
                                  #   "prompt-only" - Allow MEMORY.md only
                                  #   "search-only" - Allow memory/<name>.md only
                                  #   "off"         - Disable agent-authored memory writes
# user_profile_write_mode = "explicit-and-auto"  # How Moltis writes USER.md (default = "explicit-and-auto"):
                                  #   "explicit-and-auto" - Settings saves plus silent timezone/location capture
                                  #   "explicit-only"     - Settings saves only, no silent timezone/location capture
                                  #   "off"               - Do not write USER.md, keep profile in moltis.toml only
# backend = "builtin"            # Memory backend (default = "builtin"):
                                  #   "builtin" - Built-in SQLite indexer and retriever
                                  #   "qmd"     - External QMD CLI backend
# provider = "auto"               # Embedding provider for the built-in backend (default = auto-detect):
                                  #   "local"   - Built-in local embeddings
                                  #   "ollama"  - Ollama server
                                  #   "openai"  - OpenAI API
                                  #   "custom"  - Custom endpoint
                                  #   (none)    - Auto-detect from available providers
                                  # Ignored while backend = "qmd"
# disable_rag = false             # true => keyword-only search, disables embedding retrieval
# citations = "auto"              # Citation mode (default = "auto"): "on", "off", or "auto"
# llm_reranking = false           # Use the LLM to rerank search results (only meaningful when RAG is enabled)
# search_merge_strategy = "rrf"   # Merge strategy (default = "rrf"): "rrf" or "linear"
# session_export = "on-new-or-reset"  # Session transcript export policy (default = "on-new-or-reset"):
                                  #   "on-new-or-reset" - Export transcripts on /new and /reset
                                  #   "off"             - Disable session transcript export
#                                  # Exported transcripts become searchable memory, not prompt memory
# base_url = "http://localhost:11434/v1"  # Builtin backend only: embedding API base (host, /v1, or /embeddings)
# model = "nomic-embed-text"      # Builtin backend only: embedding model name
# api_key = "..."                 # Builtin backend only: API key (optional for local endpoints like Ollama)
# enable_prefetch = true           # Recall relevant memories before each turn (injected as <recalled_context>)
# prefetch_limit = 3               # Max memories to prefetch per turn (1-10)
# auto_extract_interval = 5        # Run background memory extraction every N turns (0 = disabled)
# enable_session_summary = true    # Write a session summary to memory on /new, /reset, or timeout

# ══════════════════════════════════════════════════════════════════════════════
# CHANNELS
# ══════════════════════════════════════════════════════════════════════════════
# External messaging integrations.
# Note: channels added or edited in the web UI are stored in Moltis's internal
# database at data_dir()/moltis.db. They are not written back into this file.
# Keep channel config here only if you want to manage it manually in TOML.

[channels]
# Which channel types appear in the web UI's "+ Add Channel" menu.
# Default: ["telegram", "msteams", "discord", "slack", "matrix", "nostr"]
# Add "whatsapp" to enable it in the UI.
# offered = ["telegram", "msteams", "discord", "slack", "matrix", "nostr", "whatsapp"]

# WhatsApp linked-device accounts
# [channels.whatsapp.my-bot]
# dm_policy = "open"              # "open", "allowlist", or "disabled"
# group_policy = "disabled"       # "open", "allowlist", or "disabled"
# mention_mode = "always"         # "always", "mention", or "none"
# model = "anthropic/claude-sonnet-4-20250514"
# model_provider = "anthropic"
# agent_id = "support"            # Default agent for this account
# otp_self_approval = true        # OTP self-approval for non-allowlisted DM users
# otp_cooldown_secs = 300         # Cooldown after 3 failed OTP attempts
# [channels.whatsapp.my-bot.channel_overrides."120363456789@g.us"]
# agent_id = "triage"
# [channels.whatsapp.my-bot.user_overrides."15551234567@s.whatsapp.net"]
# agent_id = "research"

# Telegram bots
# [channels.telegram.my-bot]
# token = "..."                   # Bot token from @BotFather
# dm_policy = "allowlist"         # "open", "allowlist", or "disabled"
# group_policy = "open"           # "open", "allowlist", or "disabled"
# mention_mode = "mention"        # "mention", "always", or "none"
# allowlist = []                  # Telegram user IDs or usernames (strings)
# group_allowlist = []            # Telegram group/chat IDs (strings)
# reply_to_message = false        # Send responses as Telegram replies
# agent_id = "research"           # Default agent for this bot
# otp_self_approval = true        # OTP self-approval for non-allowlisted DM users
# otp_cooldown_secs = 300         # Cooldown after 3 failed OTP attempts
# stream_mode = "edit_in_place"   # "edit_in_place" or "off"
# edit_throttle_ms = 300          # Min ms between streaming edits
# [channels.telegram.my-bot.channel_overrides."-1001234567890"]
# agent_id = "triage"
# [channels.telegram.my-bot.user_overrides."123456789"]
# agent_id = "research"
#
# Per-channel tool policy (restrict tools by chat type and sender):
# [channels.telegram.my-bot.tools.groups.group]
# deny = ["exec", "browser"]      # Deny dangerous tools in group chats
# [channels.telegram.my-bot.tools.groups.group.by_sender."123456"]
# allow = ["*"]                    # Override: trusted sender gets all tools
# [channels.telegram.my-bot.tools.groups.private]
# allow = []                       # DMs: allow all tools (default)

# Microsoft Teams bots
# [channels.msteams.my-bot]
# app_id = "..."                  # Azure Bot App ID
# app_password = "..."            # Azure Bot App Password (client secret)
# tenant_id = "botframework.com"  # Azure AD tenant ID (for JWT validation)
# webhook_secret = "..."          # Optional query secret for webhook URL (?secret=...)
# allowlist = []                  # User IDs allowed to DM (empty = all unless dm_policy=allowlist)
# dm_policy = "allowlist"         # "open", "allowlist", or "disabled"
# group_policy = "open"           # "open", "allowlist", or "disabled"
# mention_mode = "mention"        # "mention", "always", or "none"
# stream_mode = "edit_in_place"   # "edit_in_place" or "off"
# edit_throttle_ms = 1500         # Min ms between streaming edits
# text_chunk_limit = 4000         # Max chars per message chunk
# reply_style = "top_level"       # "top_level" or "thread"
# welcome_card = true             # Show welcome card in DMs
# group_welcome_card = false      # Show welcome text in group chats
# bot_name = "Moltis"             # Bot display name for welcome cards
# prompt_starters = []            # Prompt starter buttons on welcome card
# max_retries = 3                 # Max retry attempts for failed sends
# history_limit = 50              # Max messages for thread context (Graph API)

# Discord bots
# [channels.discord.my-bot]
# token = "..."                   # Bot token from Discord Developer Portal
# dm_policy = "allowlist"         # "open", "allowlist", or "disabled"
# group_policy = "open"           # "open", "allowlist", or "disabled"
# mention_mode = "mention"        # "mention", "always", or "none"
# allowlist = []                  # Discord user IDs allowed to DM
# guild_allowlist = []            # Discord guild/server IDs (empty = all)
# agent_id = "research"           # Default agent for this bot
# reply_to_message = false        # Send responses as Discord replies
# ack_reaction = "👀"             # Emoji reaction while processing (omit to disable)
# activity = "with AI"            # Bot activity status text
# activity_type = "custom"        # "playing", "listening", "watching", "competing", or "custom"
# status = "online"               # "online", "idle", "dnd", or "invisible"
# otp_self_approval = true        # OTP self-approval for non-allowlisted DM users
# otp_cooldown_secs = 300         # Cooldown after 3 failed OTP attempts

# Slack bots
# [channels.slack.my-bot]
# bot_token = "xoxb-..."          # Bot user OAuth token
# app_token = "xapp-..."          # App-level token for Socket Mode
# connection_mode = "socket_mode" # "socket_mode" or "events_api"
# signing_secret = "..."          # Required for events_api mode
# dm_policy = "allowlist"         # "open", "allowlist", or "disabled"
# group_policy = "open"           # "open", "allowlist", or "disabled"
# mention_mode = "mention"        # "mention", "always", or "none"
# allowlist = []                  # Slack user IDs (strings)
# channel_allowlist = []          # Slack channel IDs (strings)
# stream_mode = "edit_in_place"   # "edit_in_place", "native", or "off"
# edit_throttle_ms = 500          # Min ms between streaming edits
# thread_replies = true           # Reply in threads

# Matrix bots / appservices using OIDC, access tokens, or password login
# Three authentication modes:
#   oidc         - OAuth 2.0/OIDC via Matrix Authentication Service (MSC3861).
#                  Recommended for modern homeservers (e.g. matrix.org since April 2025).
#                  Authenticates via browser; no password or token needed.
#   password     - UIAA password login. Required for encrypted Matrix chats on
#                  older homeservers. Moltis creates and persists its own device keys.
#   access_token - Reuses an existing Matrix session. Plain traffic only (no E2EE).
# [channels.matrix.my-bot]
# homeserver = "https://matrix.example.com"
# auth_mode = "oidc"              # "oidc", "password", or "access_token" (auto-detected if omitted)
# access_token = "syt_..."        # Required for access_token mode
# password = "..."                # Required for password mode
# user_id = "@bot:example.com"    # Required for password login, auto-detected for token/OIDC auth
# device_id = "MOLTISBOT"         # Optional device ID for session restore
# device_display_name = "Moltis Matrix Bot"  # Optional display name for password logins
# ownership_mode = "moltis_owned" # "moltis_owned" or "user_managed"
# dm_policy = "allowlist"         # "open", "allowlist", or "disabled"
# room_policy = "allowlist"       # "open", "allowlist", or "disabled"
# mention_mode = "mention"        # "mention", "always", or "none"
# room_allowlist = []             # Matrix room IDs or aliases
# user_allowlist = []             # Matrix user IDs
# auto_join = "always"            # "always", "allowlist", or "off"
# model = "gpt-4.1"
# model_provider = "openai"
# stream_mode = "edit_in_place"   # "edit_in_place" or "off"
# edit_throttle_ms = 500          # Min ms between streaming edits
# stream_min_initial_chars = 30   # Delay first streamed send until this many chars
# reply_to_message = true         # Send threaded/rich replies when possible
# ack_reaction = "👀"             # Emoji reaction while processing (omit to disable)
# otp_self_approval = true        # OTP self-approval for non-allowlisted DM users
# otp_cooldown_secs = 300         # Cooldown after 3 failed OTP attempts

# Nostr DM bots (NIP-04 encrypted direct messages)
# [channels.nostr.my-bot]
# secret_key = "nsec1..."          # Secret key (nsec1 bech32 or 64-char hex)
# relays = ["wss://relay.damus.io", "wss://relay.nostr.band", "wss://nos.lol"]
# dm_policy = "allowlist"          # "open", "allowlist", or "disabled"
# allowed_pubkeys = []             # Allowed sender public keys (npub1 or hex)
# model = "anthropic/claude-sonnet-4-20250514"
# model_provider = "anthropic"
# otp_self_approval = true         # OTP self-approval for non-allowlisted DM users
# otp_cooldown_secs = 300          # Cooldown after 3 failed OTP attempts
# [channels.nostr.my-bot.profile]  # NIP-01 profile metadata (optional)
# name = "Moltis Bot"
# about = "AI assistant on Nostr"

# ══════════════════════════════════════════════════════════════════════════════
# HOOKS
# ══════════════════════════════════════════════════════════════════════════════
# Shell commands triggered by events.

# ══════════════════════════════════════════════════════════════════════════════
# ENVIRONMENT VARIABLES
# ══════════════════════════════════════════════════════════════════════════════
# Variables injected into the Moltis process at startup.
# Useful for API keys in Docker where you can't easily pass env vars.
# Process env vars (docker -e, host env) take precedence — existing vars
# are NOT overwritten.
#
# [env]
# BRAVE_API_KEY = "..."
# OPENROUTER_API_KEY = "sk-or-..."

# ══════════════════════════════════════════════════════════════════════════════
# HOOKS
# ══════════════════════════════════════════════════════════════════════════════
# Shell commands triggered by events.

# [hooks]
# [[hooks.hooks]]
# name = "my-hook"                # Hook name (for logging)
# command = "/path/to/handler.sh" # Command to run
# events = [
#     # ── Modifying events (can block or modify payload) ──
#     "BeforeAgentStart",          # Before the agent loop starts
#     "BeforeLLMCall",             # Before prompt is sent to the LLM provider
#     "AfterLLMCall",              # After LLM response, before tool execution
#     "BeforeToolCall",            # Before a tool executes
#     "BeforeCompaction",          # Before context window compaction
#     "MessageSending",            # Before sending a response to the user
#     "ToolResultPersist",         # When a tool result is persisted
#     #
#     # ── Read-only events (observe only, run in parallel) ──
#     "AfterToolCall",             # After a tool completes
#     "AfterCompaction",           # After context is compacted
#     "AgentEnd",                  # When the agent loop finishes
#     "MessageReceived",           # When a user message arrives
#     "MessageSent",               # After a response is delivered
#     "SessionStart",              # When a new session begins
#     "SessionEnd",                # When a session ends
#     "GatewayStart",              # When Moltis starts
#     "GatewayStop",               # When Moltis shuts down
#     "Command",                   # When a slash command is used
# ]
# timeout = 10                    # Command timeout in seconds
# [hooks.hooks.env]               # Environment variables passed to command
# CUSTOM_VAR = "value"
"##
    )
}
