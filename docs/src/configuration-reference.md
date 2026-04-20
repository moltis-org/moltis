# Configuration Reference

> **Manually authored from source:** `crates/config/src/schema/` + `crates/config/src/validate/schema_map.rs`

>

> Every valid `moltis.toml` option, organized by domain.

> Types: `string`, `bool`, `integer`, `float`, `array`, `map`, `optional`, `enum(...)`.

> Defaults shown as TOML values. `—` means the field has no explicit default (uses Rust `Default`).


## Contents


- **Server & Networking**
- **Observability**
- **Identity & User**
- **Chat & Agents**
- **Tools — Execution**
- **Tools — Web & Data**
- **Tools — Policy & Agent Limits**
- **Channels & Integrations**
- **Memory**
- **Scheduling & Webhooks**
- **LLM Providers**
- **Voice — Text-to-Speech**
- **Voice — Speech-to-Text**
- **Environment**
- **Server & Networking**
  - [`server`](#server)
  - [`auth`](#auth)
  - [`tls`](#tls)
  - [`graphql`](#graphql)
  - [`ngrok`](#ngrok)
  - [`tailscale`](#tailscale)
  - [`upstream_proxy`](#upstream-proxy)
  - [`failover`](#failover)
- **Observability**
  - [`metrics`](#metrics)
- **Identity & User**
  - [`identity`](#identity)
  - [`user`](#user)
- **Chat & Agents**
  - [`chat`](#chat)
  - [`chat.compaction`](#chatcompaction)
  - [`agents`](#agents)
  - [`agents.presets.<name>`](#agentspresetsname)
  - [`skills`](#skills)
- **Tools — Execution**
  - [`tools.exec`](#toolsexec)
  - [`tools.exec.sandbox`](#toolsexecsandbox)
  - [`tools.exec.sandbox.resource_limits`](#toolsexecsandboxresource-limits)
  - [`tools.exec.sandbox.tools_policy`](#toolsexecsandboxtools-policy)
  - [`tools.exec.sandbox.wasm_tool_limits`](#toolsexecsandboxwasm-tool-limits)
  - [`tools.browser`](#toolsbrowser)
- **Tools — Web & Data**
  - [`tools.web.search`](#toolswebsearch)
  - [`tools.web.search.perplexity`](#toolswebsearchperplexity)
  - [`tools.web.fetch`](#toolswebfetch)
  - [`tools.web.firecrawl`](#toolswebfirecrawl)
  - [`tools.fs`](#toolsfs)
  - [`tools.maps`](#toolsmaps)
- **Tools — Policy & Agent Limits**
  - [`tools.policy`](#toolspolicy)
  - [`tools` (agent-level scalars)](#tools-agent-level-scalars)
- **Channels & Integrations**
  - [`channels`](#channels)
  - [`channels.*.<account>.tools`](#channels*accounttools)
  - [`channels.*.<account>.tools.groups.<group_id>`](#channels*accounttoolsgroupsgroup-id)
  - [`channels.*.<account>.tools.groups.<group_id>.by_sender.<sender_id>`](#channels*accounttoolsgroupsgroup-idby-sendersender-id)
  - [`hooks`](#hooks)
  - [`hooks.hooks[]`](#hookshooks[])
  - [`mcp`](#mcp)
  - [`mcp.servers.<name>`](#mcpserversname)
  - [`mcp.servers.<name>.oauth`](#mcpserversnameoauth)
- **Memory**
  - [`memory`](#memory)
  - [`memory.qmd`](#memoryqmd)
  - [`memory.qmd.collections.<name>`](#memoryqmdcollectionsname)
- **Scheduling & Webhooks**
  - [`heartbeat`](#heartbeat)
  - [`heartbeat.active_hours`](#heartbeatactive-hours)
  - [`cron`](#cron)
  - [`caldav`](#caldav)
  - [`caldav.accounts.<name>`](#caldavaccountsname)
  - [`webhooks`](#webhooks)
  - [`webhooks.rate_limit`](#webhooksrate-limit)
- **LLM Providers**
  - [`providers`](#providers)
  - [`providers.<name>.policy`](#providersnamepolicy)
- **Voice — Text-to-Speech**
  - [`voice.tts`](#voicetts)
  - [`voice.tts.elevenlabs`](#voicettselevenlabs)
  - [`voice.tts.openai`](#voicettsopenai)
  - [`voice.tts.google`](#voicettsgoogle)
  - [`voice.tts.piper`](#voicettspiper)
  - [`voice.tts.coqui`](#voicettscoqui)
- **Voice — Speech-to-Text**
  - [`voice.stt`](#voicestt)
  - [`voice.stt.whisper`](#voicesttwhisper)
  - [`voice.stt.groq`](#voicesttgroq)
  - [`voice.stt.deepgram`](#voicesttdeepgram)
  - [`voice.stt.google`](#voicesttgoogle)
  - [`voice.stt.mistral`](#voicesttmistral)
  - [`voice.stt.elevenlabs`](#voicesttelevenlabs)
  - [`voice.stt.voxtral_local`](#voicesttvoxtral-local)
  - [`voice.stt.whisper_cli`](#voicesttwhisper-cli)
  - [`voice.stt.sherpa_onnx`](#voicesttsherpa-onnx)
- **Environment**
  - [`env`](#env)


---

## Server & Networking


### `server` — ServerConfig

Gateway server configuration.

| Key | Type | Default | Description |
|---|---|---|---|
| `bind` | string | `"127.0.0.1"` | Address to bind to. |
| `port` | integer | `0` | Port to listen on. `0` is replaced with a random available port when config is created. |
| `http_request_logs` | bool | `false` | Enable verbose Axum/Tower HTTP request logs (`http_request` spans). Useful for debugging redirects and request flow. |
| `ws_request_logs` | bool | `false` | Enable WebSocket request/response logs (`ws:` entries). Useful for debugging RPC calls from the web UI. |
| `log_buffer_size` | integer | `1000` | Maximum number of log entries kept in the in-memory ring buffer. Older entries are persisted to disk. Increase for busy servers, decrease for memory-constrained devices. |
| `update_releases_url` | optional string | — | URL of the releases manifest (`releases.json`) used by the update checker. Defaults to `https://www.moltis.org/releases.json` when unset. |
| `db_pool_max_connections` | integer | `5` | Maximum number of SQLite pool connections. Lower values reduce memory usage for personal gateways. |
| `shiki_cdn_url` | optional string | — | Base URL for the Shiki syntax-highlighting library loaded by the web UI. Defaults to `https://esm.sh/shiki@3.2.1?bundle` when unset. |
| `terminal_enabled` | bool | `true` | Enable or disable the host terminal in the web UI. Set to `false` to prevent an unsandboxed shell. The `MOLTIS_TERMINAL_DISABLED` env var (`1` or `true`) takes precedence. |


### `auth` — AuthConfig

Authentication configuration.

| Key | Type | Default | Description |
|---|---|---|---|
| `disabled` | bool | `false` | When `true`, authentication is explicitly disabled (no login required). |


### `tls` — TlsConfig

TLS configuration for the gateway HTTPS server.

| Key | Type | Default | Description |
|---|---|---|---|
| `enabled` | bool | `true` | Enable HTTPS with auto-generated certificates. |
| `auto_generate` | bool | `true` | Auto-generate a local CA and server certificate on first run. |
| `cert_path` | optional string | — | Path to a custom server certificate (PEM). Overrides auto-generation. |
| `key_path` | optional string | — | Path to a custom server private key (PEM). Overrides auto-generation. |
| `ca_cert_path` | optional string | — | Path to the CA certificate (PEM) used for trust instructions. |
| `http_redirect_port` | optional integer | — | Port for the plain-HTTP redirect/CA-download server. Defaults to the gateway port + 1 when not set. |


### `graphql` — GraphqlConfig

Runtime GraphQL server configuration.

| Key | Type | Default | Description |
|---|---|---|---|
| `enabled` | bool | `true` | Whether GraphQL HTTP/WS handlers accept requests. |


### `ngrok` — NgrokConfig

ngrok public HTTPS tunnel configuration.

| Key | Type | Default | Description |
|---|---|---|---|
| `enabled` | bool | `false` | Whether the ngrok tunnel is enabled. |
| `authtoken` | optional secret string | — | ngrok authtoken. If unset, `NGROK_AUTHTOKEN` env var is used. |
| `domain` | optional string | — | Optional reserved/static domain to request from ngrok. |


### `tailscale` — TailscaleConfig

Tailscale Serve/Funnel configuration.

| Key | Type | Default | Description |
|---|---|---|---|
| `mode` | string | `"off"` | Tailscale mode: `"off"`, `"serve"`, or `"funnel"`. |
| `reset_on_exit` | bool | `true` | Reset tailscale serve/funnel when the gateway shuts down. |


### `upstream_proxy` (top-level scalar)

| Key | Type | Default | Description |
|---|---|---|---|
| `upstream_proxy` | optional secret string | — | Upstream HTTP/SOCKS proxy for all outbound requests. Supports `http://`, `https://`, `socks5://`, and `socks5h://` schemes. Proxy auth via URL: `http://user:pass@host:port`. Overrides `HTTP_PROXY`/`HTTPS_PROXY`/`ALL_PROXY` env vars. Localhost/loopback addresses are automatically excluded (`no_proxy`). |

---


### `failover` — FailoverConfig
Automatic model/provider failover configuration.

| Key | Type | Default | Description |
|---|---|---|---|
| `enabled` | bool | `true` | Whether failover is enabled. |
| `fallback_models` | array | `[]` | Ordered list of fallback model IDs to try when the primary fails. If empty, the chain is built from all registered models. |



---

## Observability


### `metrics` — MetricsConfig

Metrics and observability configuration.

| Key | Type | Default | Description |
|---|---|---|---|
| `enabled` | bool | `true` | Whether metrics collection is enabled. |
| `prometheus_endpoint` | bool | `true` | Whether to expose the `/metrics` Prometheus endpoint. |
| `history_points` | integer | `360` | Maximum number of in-memory history points for time-series charts (sampled every 30 s; 360 ≈ 3 hours). Historical data is persisted to SQLite regardless. |
| `labels` | map | `{}` | Additional labels to add to all metrics. |


---

## Identity & User


### `identity` — AgentIdentity

Agent identity (name, emoji, theme).

| Key | Type | Default | Description |
|---|---|---|---|
| `name` | optional string | — | Agent display name. Falls back to `"moltis"` when unset. |
| `emoji` | optional string | — | Agent emoji icon. |
| `theme` | optional string | — | Agent theme identifier. |


### `user` — UserProfile

User profile collected during onboarding.

| Key | Type | Default | Description |
|---|---|---|---|
| `name` | optional string | — | User's display name. |
| `timezone` | optional string | — | IANA timezone (e.g. `"Europe/Paris"`). |


---

## Chat & Agents


### `chat` — ChatConfig

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `message_queue_mode` | enum: `followup`, `collect` | `"followup"` | How to handle messages that arrive while an agent run is active. `followup` queues each message and replays them one-by-one; `collect` concatenates and processes as a single message. |
| `prompt_memory_mode` | enum: `live-reload`, `frozen-at-session-start` | `"live-reload"` | How `MEMORY.md` is loaded into the prompt for an ongoing session. `live-reload` reloads from disk before each turn; `frozen-at-session-start` freezes the initial content for the session lifetime. |
| `workspace_file_max_chars` | integer | `32000` | Maximum characters from each workspace prompt file (`AGENTS.md`, `TOOLS.md`). |
| `priority_models` | array | `[]` | Preferred model IDs to show first in selectors (full or raw model IDs). |
| `allowed_models` | array | `[]` | ⚠️ **Deprecated.** Legacy model allowlist kept for backward compatibility; currently ignored (model visibility is provider-driven). Will be removed in a future release. |


### `chat.compaction` — CompactionConfig

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `mode` | enum: `deterministic`, `recency-preserving`, `structured`, `llm-replace` | `"deterministic"` | Compaction strategy. `deterministic` extracts a summary from message structure (zero LLM calls). `recency-preserving` keeps head + tail verbatim, collapses middle. `structured` keeps head + tail and LLM-summarises the middle. `llm-replace` replaces entire history with an LLM summary. |
| `threshold_percent` | float | `0.95` | Fraction of the session model's context window at which automatic compaction fires. Clamped to `0.1`–`0.95`. |
| `protect_head` | integer | `3` | Number of head messages preserved verbatim by recency-preserving and structured modes. |
| `protect_tail_min` | integer | `20` | Minimum number of tail messages preserved verbatim (floor under token-budget cut). |
| `tail_budget_ratio` | float | `0.20` | Tail protection window as a fraction of `threshold_percent × context_window`. |
| `tool_prune_char_threshold` | integer | `200` | Tool-result content longer than this is replaced with a placeholder in the collapsed middle region. |
| `summary_model` | optional string | `null` | Provider-qualified model for LLM summary calls (e.g. `"openrouter/google/gemini-2.5-flash"`). ⚠️ **Not yet implemented** — setting this field triggers a warning. |
| `max_summary_tokens` | integer | `4096` | Maximum output tokens for LLM summary calls. `0` accepts provider default. ⚠️ **Not yet implemented** — has no effect. |
| `show_settings_hint` | bool | `true` | Whether the "Change `chat.compaction.mode` in moltis.toml…" hint is included in compaction notifications. |


### `agents` — AgentsConfig

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `default_preset` | optional string | `null` | Default preset name used when `spawn_agent.preset` is omitted. Applies only to sub-agents. |
| `presets` | map of `AgentPreset` | `{}` | Named spawn presets, keyed by name. |


### `agents.presets.<name>` — AgentPreset

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `model` | optional string | `null` | Optional model override for this preset. |
| `tools.allow` | array | `[]` | Tools to allow (whitelist). If empty, all tools are allowed. |
| `tools.deny` | array | `[]` | Tools to deny (blacklist). Applied after `allow`. |
| `delegate_only` | bool | `false` | Restrict sub-agent to delegation/session/task tools only. |
| `system_prompt_suffix` | optional string | `null` | Extra instructions appended to the sub-agent system prompt. |
| `max_iterations` | optional integer | `null` | Maximum iterations for the agent loop. |
| `timeout_secs` | optional integer | `null` | Timeout in seconds for the sub-agent. |
| `reasoning_effort` | optional enum: `low`, `medium`, `high` | `null` | Reasoning/thinking effort level for models that support extended thinking (e.g. Claude Opus, OpenAI o-series). |
| `sessions` | optional `SessionAccessPolicyConfig` | `null` | Session access policy for inter-agent communication. |
| `memory` | optional `PresetMemoryConfig` | `null` | Persistent per-agent memory configuration. |

### `agents.presets.<name>.identity` (`AgentIdentity`)

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `name` | optional string | `null` | Agent display name. |
| `emoji` | optional string | `null` | Agent emoji identifier. |
| `theme` | optional string | `null` | Agent theme identifier. |

### `agents.presets.<name>.sessions` (`SessionAccessPolicyConfig`)

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `key_prefix` | optional string | `null` | Only see sessions with keys matching this prefix. |
| `allowed_keys` | array | `[]` | Explicit session keys the agent can access (in addition to prefix). |
| `can_send` | bool | `true` | Whether the agent can send messages to sessions. |
| `cross_agent` | bool | `false` | Whether the agent can access sessions from other agents. |

### `agents.presets.<name>.memory` (`PresetMemoryConfig`)

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `scope` | enum: `user`, `project`, `local` | `"user"` | Memory scope: `user` stores in `~/.moltis/agent-memory/<preset>/`, `project` in `.moltis/agent-memory/<preset>/`, `local` in `.moltis/agent-memory-local/<preset>/`. |
| `max_lines` | integer | `200` | Maximum lines to load from `MEMORY.md`. |


### `skills` — SkillsConfig

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `enabled` | bool | `true` | Whether the skills system is enabled. |
| `search_paths` | array | `[]` | Extra directories to search for skills. |
| `auto_load` | array | `[]` | Skills to always load (by name) without explicit activation. |
| `enable_agent_sidecar_files` | bool | `false` | Whether agents may write supplementary files inside personal skill directories. |
| `enable_self_improvement` | bool | `true` | Include system prompt guidance encouraging the agent to autonomously create and update skills after complex tasks. |

---


---

## Tools — Execution


### `tools.exec` — ExecConfig

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `default_timeout_secs` | integer | `30` | Default wall-clock timeout in seconds for command execution. |
| `max_output_bytes` | integer | `204800` | Maximum bytes of stdout/stderr captured per command. |
| `approval_mode` | string | `"on-miss"` | When operator approval is required (`"always"`, `"on-miss"`, `"never"`). |
| `security_level` | string | `"allowlist"` | Security enforcement level (`"allowlist"`, `"sandbox"`, etc.). |
| `allowlist` | array | `[]` | List of command globs permitted without sandboxing. |
| `host` | string | `"local"` | Where to run commands: `"local"`, `"node"`, or `"ssh"`. |
| `node` | optional string | `null` | Default node id or display name for remote execution (when `host = "node"`). |
| `ssh_target` | optional string | `null` | Default SSH target for remote execution (when `host = "ssh"`). |


### `tools.exec.sandbox` — SandboxConfig

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `mode` | string | `"all"` | When sandboxing is active (`"all"`, `"auto"`, `"off"`). |
| `scope` | string | `"session"` | Container lifetime (`"session"` or `"per-command"`). |
| `workspace_mount` | string | `"ro"` | Workspace mount mode (`"ro"`, `"rw"`, `"none"`). |
| `host_data_dir` | optional string | `null` | Host-visible path for Moltis `data_dir()` when creating sandbox containers from inside another container. |
| `home_persistence` | enum: `"off"`, `"session"`, `"shared"` | `"shared"` | Persistence strategy for `/home/sandbox` in sandbox containers. |
| `shared_home_dir` | optional string | `null` | Host directory for shared `/home/sandbox` persistence. Relative paths resolved against `data_dir()`. |
| `image` | optional string | `null` | Docker/Podman image for sandbox containers. |
| `container_prefix` | optional string | `null` | Name prefix for created containers. |
| `no_network` | bool | `false` | Disable all network access in sandbox containers. |
| `network` | string | `"trusted"` | Network policy: `"blocked"`, `"trusted"` (proxy-filtered), or `"bypass"` (unrestricted). |
| `trusted_domains` | array | `[]` | Domains allowed through the proxy in `"trusted"` network mode. |
| `backend` | string | `"auto"` | Sandbox backend: `"auto"`, `"docker"`, `"podman"`, `"apple-container"`, `"restricted-host"`, or `"wasm"`. |
| `packages` | array | *(~130 packages)* | Packages to install via `apt-get` in the sandbox image. Empty list to skip. |
| `wasm_fuel_limit` | optional integer | `null` | Fuel limit for WASM sandbox execution (instructions). |
| `wasm_epoch_interval_ms` | optional integer | `null` | Epoch interruption interval in milliseconds for WASM sandbox. |


### `tools.exec.sandbox.resource_limits` — ResourceLimitsConfig

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `memory_limit` | optional string | `null` | Memory limit for sandbox containers (e.g. `"512M"`, `"1G"`). |
| `cpu_quota` | optional float | `null` | CPU quota as a fraction (e.g. `0.5` = half a core, `2.0` = two cores). |
| `pids_max` | optional integer | `null` | Maximum number of PIDs allowed in the sandbox. |


### `tools.exec.sandbox.tools_policy` — ToolPolicyConfig

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `allow` | array | `[]` | Tool names explicitly allowed inside the sandbox. |
| `deny` | array | `[]` | Tool names explicitly denied inside the sandbox. |
| `profile` | optional string | `null` | Named policy profile to apply (e.g. `"restricted"`). |


### `tools.exec.sandbox.wasm_tool_limits` — WasmToolLimitsConfig

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `default_memory` | integer | `16777216` | Default WASM memory limit in bytes (16 MB). |
| `default_fuel` | integer | `1000000` | Default WASM fuel limit (instructions). |
| `tool_overrides` | map | *(see below)* | Per-tool overrides for WASM fuel and memory. |

### tools.exec.sandbox.wasm_tool_limits.tool_overrides.<name> (ToolLimitOverrideConfig)

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `fuel` | optional integer | `null` | Per-tool WASM fuel override (instructions). |
| `memory` | optional integer | `null` | Per-tool WASM memory override (bytes). |

Default `tool_overrides` entries:

| Tool | fuel | memory |
|------|------|--------|
| `calc` | `100000` | `2097152` (2 MB) |
| `web_fetch` | `10000000` | `33554432` (32 MB) |
| `web_search` | `10000000` | `33554432` (32 MB) |
| `show_map` | `10000000` | `67108864` (64 MB) |
| `location` | `5000000` | `16777216` (16 MB) |


### `tools.browser` — BrowserConfig

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `enabled` | bool | `true` | Whether browser support is enabled. |
| `chrome_path` | optional string | `null` | Path to Chrome/Chromium binary (auto-detected if not set). |
| `headless` | bool | `true` | Whether to run in headless mode. |
| `viewport_width` | integer | `2560` | Default viewport width in pixels. |
| `viewport_height` | integer | `1440` | Default viewport height in pixels. |
| `device_scale_factor` | float | `2.0` | Device scale factor for HiDPI/Retina displays (1.0, 2.0, 3.0). |
| `max_instances` | integer | `0` | Maximum concurrent browser instances (0 = unlimited, limited by memory). |
| `memory_limit_percent` | integer | `90` | System memory usage threshold (0–100) above which new instances are blocked. |
| `idle_timeout_secs` | integer | `300` | Instance idle timeout in seconds before closing. |
| `navigation_timeout_ms` | integer | `30000` | Default navigation timeout in milliseconds. |
| `user_agent` | optional string | `null` | Custom user agent string (uses Chrome default if not set). |
| `chrome_args` | array | `[]` | Additional Chrome command-line arguments. |
| `sandbox_image` | string | `"docker.io/browserless/chrome"` | Docker image for sandboxed browser instances. |
| `allowed_domains` | array | `[]` | Allowed navigation domains (empty = all allowed). Supports wildcards (`"*.example.com"`). |
| `low_memory_threshold_mb` | integer | `2048` | System RAM threshold (MB) below which memory-saving Chrome flags are injected (0 to disable). |
| `persist_profile` | bool | `true` | Persist Chrome user profile (cookies, auth, local storage) across sessions. |
| `profile_dir` | optional string | `null` | Custom path for persistent Chrome profile directory. Implies `persist_profile = true`. |
| `container_host` | string | `"127.0.0.1"` | Hostname/IP to connect to the browser container from the host. Use `"host.docker.internal"` when Moltis runs inside Docker. |
| `browserless_api_version` | enum: `"v1"`, `"v2"` | `"v1"` | Browserless API compatibility mode for websocket endpoints. |

---


---

## Tools — Web & Data


### `tools.web.search` — WebSearchConfig

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| enabled | bool | `true` | Enable the web search tool. |
| provider | string (enum) | `"brave"` | Search provider. One of: `brave`, `perplexity`, `firecrawl`. |
| api_key | optional string | — | API key (overrides `BRAVE_API_KEY` env var). |
| max_results | integer | `5` | Maximum number of results to return (1–10). |
| timeout_seconds | integer | `30` | HTTP request timeout in seconds. |
| cache_ttl_minutes | integer | `15` | In-memory cache TTL in minutes (0 to disable). |
| duckduckgo_fallback | bool | `false` | Enable DuckDuckGo HTML fallback when no provider API key is configured. Disabled by default because it may trigger CAPTCHA challenges. |


### `tools.web.search.perplexity` — PerplexityConfig

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| api_key | optional string | — | API key (overrides `PERPLEXITY_API_KEY` / `OPENROUTER_API_KEY` env vars). |
| base_url | optional string | — | Base URL override. Auto-detected from key prefix if empty. |
| model | optional string | — | Model to use. |


### `tools.web.fetch` — WebFetchConfig

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| enabled | bool | `true` | Enable the web fetch tool. |
| max_chars | integer | `50000` | Maximum characters to return from fetched content. |
| timeout_seconds | integer | `30` | HTTP request timeout in seconds. |
| cache_ttl_minutes | integer | `15` | In-memory cache TTL in minutes (0 to disable). |
| max_redirects | integer | `3` | Maximum number of HTTP redirects to follow. |
| readability | bool | `true` | Use readability extraction for HTML pages. |
| ssrf_allowlist | array | `[]` | CIDR ranges exempt from SSRF blocking (e.g. `["172.22.0.0/16"]`). Default: empty (all private IPs blocked). ⚠️ **Security**: ranges added here bypass SSRF protection. Only add specific, trusted CIDR ranges (e.g. a known sidecar subnet), never broad private ranges like `10.0.0.0/8`. |


### `tools.web.firecrawl` — FirecrawlConfig

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| enabled | bool | `false` | Enable Firecrawl integration. |
| api_key | optional string | — | Firecrawl API key (overrides `FIRECRAWL_API_KEY` env var). |
| base_url | string | `"https://api.firecrawl.dev"` | Firecrawl API base URL (for self-hosted instances). |
| only_main_content | bool | `true` | Only extract main content (skip navs, footers, etc.). |
| timeout_seconds | integer | `30` | HTTP request timeout in seconds. |
| cache_ttl_minutes | integer | `15` | In-memory cache TTL in minutes (0 to disable). |
| web_fetch_fallback | bool | `true` | Use Firecrawl as fallback in `web_fetch` when readability extraction produces poor results. |


### `tools.fs` — FsToolsConfig

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| workspace_root | optional string | — | Default search root used by `Glob` and `Grep` when the LLM call omits the `path` argument. Must be an absolute path. When unset, calls without an explicit `path` are rejected. |
| allow_paths | array | `[]` | Absolute path globs the tools are allowed to access. Empty list means all paths allowed. Evaluated after canonicalization. |
| deny_paths | array | `[]` | Absolute path globs the tools must refuse. Deny wins over allow. Evaluated after canonicalization. |
| track_reads | bool | `false` | Whether to track per-session read history (files read, re-read loop detection). Required for `must_read_before_write`. |
| must_read_before_write | bool | `false` | Reject Write/Edit/MultiEdit calls targeting files the session has not previously Read. Requires `track_reads = true`. |
| require_approval | bool | `false` | Whether Write/Edit/MultiEdit must pause for explicit operator approval before mutating a file. |
| max_read_bytes | integer | `10485760` (10 MB) | Maximum bytes a single `Read` call can return before the file is rejected with a typed `too_large` payload. |
| binary_policy | string (enum) | `"reject"` | What to do with binary files encountered by `Read`. One of: `reject` (return typed marker without content), `base64` (return base64-encoded content). |
| respect_gitignore | bool | `true` | Whether `Glob` and `Grep` respect `.gitignore` / `.ignore` files and `.git/info/exclude` while walking. |
| checkpoint_before_mutation | bool | `false` | When true, Write/Edit/MultiEdit create a per-file backup before mutating, so the LLM can restore the pre-edit state via `checkpoint_restore`. |
| context_window_tokens | optional integer | — | Model context window in tokens. When set, `Read`'s per-call byte cap scales adaptively so a single Read call can't consume more than ~20% of the model's working set. Clamped to [50 KB, 512 KB]. When unset, Read uses a fixed 256 KB cap. Typical values: 200000 (Claude 3.5/4 Sonnet), 1000000 (Claude Opus 4.6), 128000 (GPT-4 Turbo). |


### `tools.maps` — MapsConfig

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| provider | string (enum) | `"google_maps"` | Preferred map provider used by `show_map`. One of: `google_maps`, `apple_maps`, `openstreetmap`. |


---

## Tools — Policy & Agent Limits


### `tools.policy` — ToolPolicyConfig

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| allow | array | `[]` | Tool names or glob patterns that are explicitly allowed. |
| deny | array | `[]` | Tool names or glob patterns that are explicitly denied. |
| profile | optional string | — | Named policy profile to apply. |


### `tools` — Agent-level scalars

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| agent_timeout_secs | integer | `600` | Maximum wall-clock seconds for an agent run (0 = no timeout). |
| agent_max_iterations | integer | `25` | Maximum number of agent loop iterations before aborting. |
| agent_max_auto_continues | integer | `2` | Maximum auto-continue nudges when the model stops mid-task (0 = disabled). |
| agent_auto_continue_min_tool_calls | integer | `3` | Minimum tool calls in the current run before auto-continue can trigger. |
| max_tool_result_bytes | integer | `50000` (50 KB) | Maximum bytes for a single tool result before truncation. |
| registry_mode | string (enum) | `"full"` | How tool schemas are presented to the model. One of: `full` (all schemas sent every turn), `lazy` (only `tool_search` sent; model discovers tools on demand). |
| agent_loop_detector_window | integer | `3` | Window size for the tool-call reflex-loop detector. When this many consecutive tool calls share the same tool + (args or error), the runner injects a directive intervention message. Set to 0 to disable. |
| agent_loop_detector_strip_tools_on_second_fire | bool | `true` | When the loop detector fires a second time (stage 2), strip the tool schema list for a single LLM turn so the model is forced to respond in text. |

---


---

## Channels & Integrations


### `channels` — ChannelsConfig

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `offered` | array of string | `["telegram", "msteams", "discord", "slack", "matrix", "nostr"]` | Which channel types are offered in the web UI (onboarding + channels page). Add `"whatsapp"` to opt in. |
| `<channel_type>` | map of `serde_json::Value` | `{}` | Account configs keyed by account name. Known types: `telegram`, `whatsapp`, `msteams`, `discord`, `slack`, `nostr`. Additional types accepted via flatten. |

Each channel account (`channels.<channel_type>.<account_name>`) is an arbitrary JSON object that may contain provider-specific keys plus a `tools` sub-block (see below).


### `channels.*.<account>.tools` — ChannelToolPolicyOverride

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `groups` | map of GroupToolPolicy | `{}` | Per-chat-type policies, keyed by chat type (`"private"`, `"group"`, etc.). |


### `channels.*.<account>.tools.groups.<group_id>` — GroupToolPolicy

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `allow` | array of string | `[]` | Tool names/patterns to allow. |
| `deny` | array of string | `[]` | Tool names/patterns to deny. |
| `by_sender` | map of ToolPolicyConfig | `{}` | Per-sender overrides within this group, keyed by sender/peer ID. |


### `channels.*.<account>.tools.groups.<group_id>.by_sender.<sender_id>` — ToolPolicyConfig

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `allow` | array of string | `[]` | Tool names/patterns to allow for this sender. |
| `deny` | array of string | `[]` | Tool names/patterns to deny for this sender. |
| `profile` | optional string | `None` | Agent profile to use for this sender. |

---


### `hooks` — HooksConfig

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `hooks` | array of ShellHookConfigEntry | `[]` | Shell hooks defined in the config file. |


### `hooks.hooks[]` — ShellHookConfigEntry

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `name` | string | *(required)* | Human-readable hook name. |
| `command` | string | *(required)* | Shell command to execute. |
| `events` | array of string | *(required)* | Event names that trigger this hook. |
| `timeout` | integer | `10` | Timeout in seconds for the hook process. |
| `env` | map of string | `{}` | Environment variables to set for the hook process. |

---


### `mcp` — McpConfig

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `request_timeout_secs` | integer | `30` | Default timeout for MCP requests in seconds. |
| `servers` | map of McpServerEntry | `{}` | Configured MCP servers, keyed by server name. |


### `mcp.servers.<name>` — McpServerEntry

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `command` | string | `""` | Command to spawn the server process (stdio transport). |
| `args` | array of string | `[]` | Arguments to the command. |
| `env` | map of string | `{}` | Environment variables to set for the process. |
| `enabled` | bool | `true` | Whether this server is enabled. |
| `request_timeout_secs` | optional integer | `None` | Optional per-server MCP request timeout override in seconds. |
| `transport` | string | `""` | Transport type: `"stdio"` (default), `"sse"`, or `"streamable-http"`. |
| `url` | optional string | `None` | URL for SSE/Streamable HTTP transport. Required when `transport` is `"sse"` or `"streamable-http"`. |
| `headers` | map of string | `{}` | Custom headers for remote HTTP/SSE transport. |
| `oauth` | optional McpOAuthOverrideEntry | `None` | Manual OAuth override for servers that don't support standard discovery. |
| `display_name` | optional string | `None` | Custom display name for the server (shown in UI instead of technical ID). |


### `mcp.servers.<name>.oauth` — McpOAuthOverrideEntry

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `client_id` | string | *(required)* | The OAuth client ID. |
| `auth_url` | string | *(required)* | The authorization endpoint URL. |
| `token_url` | string | *(required)* | The token endpoint URL. |
| `scopes` | array of string | `[]` | OAuth scopes to request. |

---


---

## Memory


### `memory`

**Struct:** `MemoryEmbeddingConfig`

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `style` | enum (`hybrid`, `prompt-only`, `search-only`, `off`) | `"hybrid"` | High-level memory orchestration style. |
| `agent_write_mode` | enum (`hybrid`, `prompt-only`, `search-only`, `off`) | `"hybrid"` | Where agent-authored memory writes are allowed to land. |
| `user_profile_write_mode` | enum (`explicit-and-auto`, `explicit-only`, `off`) | `"explicit-and-auto"` | How Moltis writes the managed `USER.md` profile surface. |
| `backend` | enum (`builtin`, `qmd`) | `"builtin"` | Memory backend used for search, retrieval, and indexing. |
| `provider` | optional enum (`local`, `ollama`, `openai`, `custom`) | *auto-detect* | Embedding provider. Alias: `embedding_provider`. |
| `disable_rag` | bool | `false` | Disable RAG embeddings and force keyword-only memory search. |
| `base_url` | optional string | — | Base URL for the embedding API. Alias: `embedding_base_url`. |
| `model` | optional string | — | Model name for embeddings. Alias: `embedding_model`. |
| `api_key` | optional string (secret) | — | API key for the embedding endpoint. Alias: `embedding_api_key`. |
| `citations` | enum (`on`, `off`, `auto`) | `"auto"` | Citation mode for memory search results. |
| `llm_reranking` | bool | `false` | Enable LLM reranking for hybrid search results. |
| `search_merge_strategy` | enum (`rrf`, `linear`) | `"rrf"` | Merge strategy for hybrid search results. |
| `session_export` | enum (`off`, `on-new-or-reset`) | `"on-new-or-reset"` | How session transcripts are exported into searchable memory. |
| `qmd` | map (see `memory.qmd`) | `{}` | QMD-specific configuration (only used when backend = `"qmd"`). |


### `memory.qmd`

**Struct:** `QmdConfig`

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `command` | optional string | `"qmd"` | Path to the qmd binary. |
| `collections` | map of name → `QmdCollection` | `{}` | Named collections with paths and glob patterns. |
| `max_results` | optional integer | — | Maximum results to retrieve. |
| `timeout_ms` | optional integer | — | Search timeout in milliseconds. |


### `memory.qmd.collections.<name>`

**Struct:** `QmdCollection`

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `paths` | array of string | `[]` | Paths to include in this collection. |
| `globs` | array of string | `[]` | Glob patterns to filter files. |


---

## Scheduling & Webhooks


### `heartbeat`

**Struct:** `HeartbeatConfig`

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `enabled` | bool | `true` | Whether the heartbeat is enabled. |
| `every` | string | `"30m"` | Interval between heartbeats (e.g. `"30m"`, `"1h"`). |
| `model` | optional string | — | Provider/model override for heartbeat turns. |
| `prompt` | optional string | — | Custom prompt override. Empty uses the built-in default. |
| `ack_max_chars` | integer | `300` | Max characters for an acknowledgment reply before truncation. |
| `active_hours` | map (see `heartbeat.active_hours`) | — | Active hours window — heartbeats only run during this window. |
| `deliver` | bool | `false` | Whether heartbeat replies should be delivered to a channel account. |
| `channel` | optional string | — | Channel account identifier for heartbeat delivery. |
| `to` | optional string | — | Destination chat/recipient id for heartbeat delivery. |
| `sandbox_enabled` | bool | `true` | Whether heartbeat runs inside a sandbox. |
| `sandbox_image` | optional string | — | Override sandbox image for heartbeat. |


### `heartbeat.active_hours`

**Struct:** `ActiveHoursConfig`

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `start` | string | `"08:00"` | Start time in HH:MM format. |
| `end` | string | `"24:00"` | End time in HH:MM format. |
| `timezone` | string | `"local"` | IANA timezone (e.g. `"Europe/Paris"`) or `"local"`. |


### `cron`

**Struct:** `CronConfig`

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `rate_limit_max` | integer | `10` | Maximum number of jobs within the rate limit window. |
| `rate_limit_window_secs` | integer | `60` | Rate limit window in seconds. |
| `session_retention_days` | optional integer | `7` | Days to retain cron session data before auto-cleanup. `None` disables pruning. |
| `auto_prune_cron_containers` | bool | `true` | Whether to auto-prune sandbox containers after cron job completion. |


### `caldav`

**Struct:** `CalDavConfig`

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `enabled` | bool | `false` | Whether CalDAV integration is enabled. |
| `default_account` | optional string | — | Default account name to use when none is specified. |
| `accounts` | map of name → `CalDavAccountConfig` | `{}` | Named CalDAV accounts. |


### `caldav.accounts.<name>`

**Struct:** `CalDavAccountConfig`

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `url` | optional string | — | CalDAV server URL. |
| `username` | optional string | — | Username for authentication. |
| `password` | optional string (secret) | — | Password or app-specific password. |
| `provider` | optional string | — | Provider hint: `"fastmail"`, `"icloud"`, or `"generic"`. |
| `timeout_seconds` | integer | `30` | HTTP request timeout in seconds. |


### `webhooks`

**Struct:** `WebhooksConfig`

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `rate_limit` | map (see `webhooks.rate_limit`) | — | Per-account rate limiting settings. |


### `webhooks.rate_limit`

**Struct:** `WebhookRateLimitConfig`

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `enabled` | bool | `true` | Whether rate limiting is enabled. |
| `requests_per_minute` | optional integer | — | Max requests per minute per account. `None` uses per-channel defaults. |
| `burst` | optional integer | — | Burst allowance per account. |
| `cleanup_interval_secs` | integer | `300` | Interval in seconds between stale bucket cleanup. |


---

## LLM Providers


### `providers`

**Struct:** `ProvidersConfig`

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `offered` | array of string | `[]` | Allowlist of enabled providers (also controls web UI pickers). Empty = all enabled. |
| `show_legacy_models` | bool | `false` | Show models older than one year in the chat model selector. |
| `<name>` | `ProviderEntry` (see below) | — | Provider-specific settings keyed by provider name. |

### `providers.<name>` — ProviderEntry
| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `enabled` | bool | `true` | Whether this provider is enabled. |
| `api_key` | optional string (secret) | — | Override the API key. Env var takes precedence if set. |
| `base_url` | optional string | — | Override the base URL. Alias: `url`. |
| `models` | array of string | `[]` | Preferred model IDs shown first in model pickers. |
| `fetch_models` | bool | `true` | Whether to fetch provider model catalogs dynamically. |
| `stream_transport` | enum (`sse`, `websocket`, `auto`) | `"sse"` | Streaming transport for this provider. |
| `wire_api` | enum (`chat-completions`, `responses`) | `"chat-completions"` | Wire format for this provider's HTTP API. |
| `alias` | optional string | — | Alias used in metrics labels instead of the provider name. |
| `tool_mode` | enum (`auto`, `native`, `text`, `off`) | `"auto"` | How tool calling is handled for this provider. |
| `cache_retention` | enum (`none`, `short`, `long`) | `"short"` | Prompt cache retention policy. |
| `policy` | optional `ToolPolicyConfig` (see below) | — | Tool policy override merged on top of global `[tools.policy]`. |
| `model_overrides` | map of `ModelOverride` | `{}` | Per-model context window overrides. Keys are model IDs. |

### `providers.<name>.model_overrides.<model_id>` — ModelOverride

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `context_window` | optional integer | — | Override the context window size (in tokens) for this model. Must be ≥ 1. Values > 10,000,000 produce a warning. |

### `models` — Global Model Overrides

**Struct:** `HashMap<String, ModelOverride>`

Per-model overrides that apply across all providers. Provider-scoped overrides (`providers.<name>.model_overrides.<id>`) take precedence over these.

```toml
[models.claude-opus-4-6]
context_window = 1_000_000
```


### `providers.<name>.policy`

**Struct:** `ToolPolicyConfig`

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `allow` | array of string | `[]` | Tool names to allow. |
| `deny` | array of string | `[]` | Tool names to deny. |
| `profile` | optional string | — | Named policy profile to apply. |

---


### `voice`
**Struct:** `VoiceConfig`

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `tts` | `VoiceTtsConfig` | (see below) | Text-to-speech settings |
| `stt` | `VoiceSttConfig` | (see below) | Speech-to-text settings |

---

## Voice — Text-to-Speech


### `voice.tts`

**Struct:** `VoiceTtsConfig`

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `enabled` | bool | `true` | Enable TTS globally |
| `provider` | string | `""` | Active provider (`"openai"`, `"elevenlabs"`, `"google"`, `"piper"`, `"coqui"`). Empty string auto-selects the first configured provider |
| `providers` | array of string | `[]` | Provider IDs to list in the UI. Empty means list all |
| `elevenlabs` | `VoiceElevenLabsConfig` | (see below) | ElevenLabs-specific settings |
| `openai` | `VoiceOpenAiConfig` | (see below) | OpenAI TTS settings |
| `google` | `VoiceGoogleTtsConfig` | (see below) | Google Cloud TTS settings |
| `piper` | `VoicePiperTtsConfig` | (see below) | Piper (local) settings |
| `coqui` | `VoiceCoquiTtsConfig` | (see below) | Coqui TTS (local server) settings |


### `voice.tts.elevenlabs`

**Struct:** `VoiceElevenLabsConfig`

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `api_key` | optional secret string | `null` | API key (from `ELEVENLABS_API_KEY` env or config) |
| `voice_id` | optional string | `null` | Default voice ID |
| `model` | optional string | `null` | Model to use (e.g. `"eleven_flash_v2_5"` for lowest latency) |


### `voice.tts.openai`

**Struct:** `VoiceOpenAiConfig`

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `api_key` | optional secret string | `null` | API key (from `OPENAI_API_KEY` env or config) |
| `base_url` | optional string | `null` | Override the OpenAI TTS endpoint for compatible local servers |
| `voice` | optional string | `null` | Voice to use for TTS (`alloy`, `echo`, `fable`, `onyx`, `nova`, `shimmer`) |
| `model` | optional string | `null` | Model to use for TTS (`tts-1`, `tts-1-hd`) |


### `voice.tts.google`

**Struct:** `VoiceGoogleTtsConfig`

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `api_key` | optional secret string | `null` | API key for Google Cloud Text-to-Speech |
| `voice` | optional string | `null` | Voice name (e.g. `"en-US-Neural2-A"`, `"en-US-Wavenet-D"`) |
| `language_code` | optional string | `null` | Language code (e.g. `"en-US"`, `"fr-FR"`) |
| `speaking_rate` | optional float | `null` | Speaking rate (0.25–4.0, default 1.0) |
| `pitch` | optional float | `null` | Pitch (-20.0–20.0, default 0.0) |


### `voice.tts.piper`

**Struct:** `VoicePiperTtsConfig`

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `binary_path` | optional string | `null` | Path to piper binary. If not set, looks in `PATH` |
| `model_path` | optional string | `null` | Path to the voice model file (`.onnx`) |
| `config_path` | optional string | `null` | Path to the model config file (`.onnx.json`). Defaults to `model_path` + `".json"` |
| `speaker_id` | optional integer | `null` | Speaker ID for multi-speaker models |
| `length_scale` | optional float | `null` | Speaking rate multiplier (default 1.0) |


### `voice.tts.coqui`

**Struct:** `VoiceCoquiTtsConfig`

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `endpoint` | string | `"http://localhost:5002"` | Coqui TTS server endpoint |
| `model` | optional string | `null` | Model name to use (if server supports multiple models) |
| `speaker` | optional string | `null` | Speaker name or ID for multi-speaker models |
| `language` | optional string | `null` | Language code for multilingual models |

---


---

## Voice — Speech-to-Text


### `voice.stt`

**Struct:** `VoiceSttConfig`

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `enabled` | bool | `true` | Enable STT globally |
| `provider` | optional enum: `whisper`, `groq`, `deepgram`, `google`, `mistral`, `elevenlabs-stt`, `voxtral-local`, `whisper-cli`, `sherpa-onnx` | `null` | Active provider. `null` auto-selects the first configured provider. Values: `"whisper"`, `"groq"`, `"deepgram"`, `"google"`, `"mistral"`, `"elevenlabs-stt"`, `"voxtral-local"`, `"whisper-cli"`, `"sherpa-onnx"` |
| `providers` | array of string | `[]` | Provider IDs to list in the UI. Empty means list all |
| `whisper` | `VoiceWhisperConfig` | (see below) | OpenAI Whisper settings |
| `groq` | `VoiceGroqSttConfig` | (see below) | Groq (Whisper-compatible) settings |
| `deepgram` | `VoiceDeepgramConfig` | (see below) | Deepgram settings |
| `google` | `VoiceGoogleSttConfig` | (see below) | Google Cloud Speech-to-Text settings |
| `mistral` | `VoiceMistralSttConfig` | (see below) | Mistral AI (Voxtral Transcribe) settings |
| `elevenlabs` | `VoiceElevenLabsSttConfig` | (see below) | ElevenLabs Scribe settings |
| `voxtral_local` | `VoiceVoxtralLocalConfig` | (see below) | Voxtral local (vLLM server) settings |
| `whisper_cli` | `VoiceWhisperCliConfig` | (see below) | whisper-cli (whisper.cpp) settings |
| `sherpa_onnx` | `VoiceSherpaOnnxConfig` | (see below) | sherpa-onnx offline settings |


### `voice.stt.whisper`

**Struct:** `VoiceWhisperConfig`

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `api_key` | optional secret string | `null` | API key (from `OPENAI_API_KEY` env or config) |
| `base_url` | optional string | `null` | Override the Whisper endpoint for compatible local servers |
| `model` | optional string | `null` | Model to use (`whisper-1`) |
| `language` | optional string | `null` | Language hint (ISO 639-1 code) |


### `voice.stt.groq`

**Struct:** `VoiceGroqSttConfig`

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `api_key` | optional secret string | `null` | API key (from `GROQ_API_KEY` env or config) |
| `model` | optional string | `null` | Model to use (e.g. `"whisper-large-v3-turbo"`) |
| `language` | optional string | `null` | Language hint (ISO 639-1 code) |


### `voice.stt.deepgram`

**Struct:** `VoiceDeepgramConfig`

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `api_key` | optional secret string | `null` | API key (from `DEEPGRAM_API_KEY` env or config) |
| `model` | optional string | `null` | Model to use (e.g. `"nova-3"`) |
| `language` | optional string | `null` | Language hint (e.g. `"en-US"`) |
| `smart_format` | bool | `false` | Enable smart formatting (punctuation, capitalization) |


### `voice.stt.google`

**Struct:** `VoiceGoogleSttConfig`

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `api_key` | optional secret string | `null` | API key for Google Cloud Speech-to-Text |
| `service_account_json` | optional string | `null` | Path to service account JSON file (alternative to API key) |
| `language` | optional string | `null` | Language code (e.g. `"en-US"`) |
| `model` | optional string | `null` | Model variant (e.g. `"latest_long"`, `"latest_short"`) |


### `voice.stt.mistral`

**Struct:** `VoiceMistralSttConfig`

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `api_key` | optional secret string | `null` | API key (from `MISTRAL_API_KEY` env or config) |
| `model` | optional string | `null` | Model to use (e.g. `"voxtral-mini-latest"`) |
| `language` | optional string | `null` | Language hint (ISO 639-1 code) |


### `voice.stt.elevenlabs`

**Struct:** `VoiceElevenLabsSttConfig`

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `api_key` | optional secret string | `null` | API key (from `ELEVENLABS_API_KEY` env or config). Shared with TTS if not specified separately |
| `model` | optional string | `null` | Model to use (`scribe_v1` or `scribe_v2`) |
| `language` | optional string | `null` | Language hint (ISO 639-1 code) |


### `voice.stt.voxtral_local`

**Struct:** `VoiceVoxtralLocalConfig`

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `endpoint` | string | `"http://localhost:8000"` | vLLM server endpoint |
| `model` | optional string | `null` | Model to use (optional, server default if not set) |
| `language` | optional string | `null` | Language hint (ISO 639-1 code) |


### `voice.stt.whisper_cli`

**Struct:** `VoiceWhisperCliConfig`

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `binary_path` | optional string | `null` | Path to whisper-cli binary. If not set, looks in `PATH` |
| `model_path` | optional string | `null` | Path to the GGML model file (e.g. `"~/.moltis/models/ggml-base.en.bin"`) |
| `language` | optional string | `null` | Language hint (ISO 639-1 code) |


### `voice.stt.sherpa_onnx`

**Struct:** `VoiceSherpaOnnxConfig`

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `binary_path` | optional string | `null` | Path to sherpa-onnx-offline binary. If not set, looks in `PATH` |
| `model_dir` | optional string | `null` | Path to the ONNX model directory |
| `language` | optional string | `null` | Language hint (ISO 639-1 code) |


---

## Environment


### `env`

**Struct:** top-level `HashMap<String, String>` on `MoltisConfig`

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `*` | string | — | Dynamic map of environment variable names to values. |


---

