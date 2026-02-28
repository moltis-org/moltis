# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **Podman sandbox backend** — Podman as a first-class sandbox backend. Set `backend = "podman"` or let auto-detection prefer it over Docker (Apple Container → Podman → Docker → restricted-host). Uses the `podman` CLI directly (no socket compatibility needed)
- **Trusted network mode**: sandbox containers now default to `sandbox.network = "trusted"`, routing outbound traffic through an HTTP CONNECT proxy with full audit logging. When `trusted_domains` is empty (the default), all domains are allowed (audit-only mode); when configured, only listed domains pass without approval. Includes real-time network audit log with domain, protocol, and action filtering via Settings > Network Audit. Configurable via `sandbox.trusted_domains` in `moltis.toml`. Proxy env vars (`HTTP_PROXY`, `HTTPS_PROXY`, `NO_PROXY`) are now automatically injected into both Docker and Apple Container sandboxes, and the proxy binds to `0.0.0.0` so it is reachable from container VMs. The proxy rejects connections from non-private IPs (only loopback, RFC 1918, link-local, and CGNAT ranges are accepted)
- **New `moltis-network-filter` crate**: domain filtering, proxy, and audit buffer logic extracted from `moltis-tools` and `moltis-gateway` into a standalone crate with feature flags (`proxy`, `service`, `metrics`). The macOS app can now depend on it directly for network audit log display via `moltis-swift-bridge`
- **macOS Network Audit pane**: new Settings > Network Audit section with real-time log display, action filtering (allowed/denied), search, pause/resume, clipboard export, and JSONL download — matching the web UI pattern. New FFI callback `moltis_set_network_audit_callback` bridges Rust audit entries to Swift
- **Proxy-compliant HTTP tools**: all HTTP tools (`web_fetch`, `web_search`, `location`, `map`) now route through the trusted-network proxy when active, so their traffic appears in the Network Audit log and respects domain filtering. The shared `reqwest` client is initialized with proxy config at gateway startup; `web_fetch` uses a per-tool proxy setting for its custom redirect-following client
- **Network policy rename**: `sandbox.network = "open"` has been renamed to `"bypass"` to make explicit that traffic bypasses the proxy entirely (no audit logging)
- **Real WASM sandbox** (`wasm` feature, default on) — Wasmtime + WASI sandbox with filesystem isolation, fuel metering, epoch-based timeouts, and ~20 built-in coreutils (echo, cat, ls, mkdir, rm, cp, mv, etc.). Two execution tiers: built-in commands operate on a sandboxed directory tree; `.wasm` modules run via Wasmtime with preopened dirs and captured I/O. Backend: `"wasm"` in config
- **Restricted-host sandbox** — new `"restricted-host"` backend (extracted from the old `WasmtimeSandbox`) providing honest naming for what it does: env clearing, restricted PATH, and `ulimit` resource wrappers without containers or WASM. Always compiled (no feature gate)
- **Docker security hardening** — containers now launch with `--cap-drop ALL`, `--security-opt no-new-privileges`, tmpfs mounts for `/tmp` and `/run`, and `--read-only` root filesystem for prebuilt images
- **Generic sandbox failover chain** — auto-detection now tries Apple Container → Docker → Restricted Host. Failover uses restricted-host as the final fallback instead of NoSandbox
- Discord channel integration via new `moltis-discord` crate using serenity Gateway API (persistent WebSocket, no public URL required). Supports DM and group messaging with allowlist/OTP gating, mention mode, guild allowlist, and 2000-char message chunking. Web UI: connect/edit/remove Discord bots in Settings > Channels and onboarding flow
- Discord reply-to-message support: set `reply_to_message = true` to have the bot send responses as Discord threaded replies to the user's message
- Discord ack reactions: set `ack_reaction = "👀"` to add an emoji reaction while processing (removed on completion)
- Discord bot token import from OpenClaw installations during onboarding (both flat and multi-account configs)
- Discord bot presence/activity: configure `activity`, `activity_type` (playing/listening/watching/competing/custom), and `status` (online/idle/dnd/invisible) in bot config
- Discord OTP self-approval for DMs: non-allowlisted users receive a 6-digit challenge code (visible in web UI) to self-approve access, matching Telegram's existing OTP flow
- Discord native slash commands: `/new`, `/clear`, `/compact`, `/context`, `/model`, `/sessions`, `/agent`, `/help` registered as Discord application commands with ephemeral responses
- OTP module moved from `moltis-telegram` to shared `moltis-channels` crate for cross-platform reuse
- Real-time session sync between macOS app and web UI via `SessionEventBus` (`tokio::sync::broadcast`). Sessions created, deleted, or patched in one UI instantly appear in the other. New FFI callback `moltis_set_session_event_callback` and WebSocket `"session"` events for create/delete/fork operations.
- Swift bridge: persistent session storage via FFI — `moltis_list_sessions`, `moltis_switch_session`, `moltis_create_session`, `moltis_session_chat_stream` functions backed by JSONL files and shared SQLite metadata (`moltis.db`) across all UIs (macOS app, web, TUI)
- **Internationalization (i18n)**: web UI now supports runtime language switching via `i18next` with English and French locales. Error codes use structured constants with locale-aware error messages across API handlers, terminal, chat, and environment routes. Onboarding step labels, navigation buttons, and page strings use translation keys (`t()` calls)
- **Vault UI**: recovery key display during onboarding password setup, vault status/unlock controls in Settings > Security, encrypted/plaintext badges on environment variables
- **Encryption-at-rest vault** (`vault` feature, default on) — environment variables are encrypted with XChaCha20-Poly1305 AEAD using Argon2id-derived keys. Vault is initialized on first password setup and auto-unsealed on login. Recovery key provided at initialization for emergency access. API: `/api/auth/vault/status`, `/api/auth/vault/unlock`, `/api/auth/vault/recovery`
- `send_image` tool for sending local image files (PNG, JPEG, GIF, WebP) to channel targets like Telegram, with optional caption support
- GraphQL API at `/graphql` (GET serves GraphiQL playground and WebSocket subscriptions, POST handles queries/mutations) exposing all RPC methods as typed operations
- New `moltis-graphql` crate with queries, mutations, subscriptions, custom `Json` scalar, and `ServiceCaller` trait abstraction
- New `moltis-providers` crate that owns provider integrations and model registry/catalog logic (OpenAI, Anthropic, OpenAI-compatible, OpenAI Codex, GitHub Copilot, Kimi Code, local GGUF, local LLM)
- `graphql` feature flag (default on) in gateway and CLI crates for compile-time opt-out
- Settings > GraphQL page embedding GraphiQL playground at `/settings/graphql`
- Gateway startup now seeds a built-in `dcg-guard` hook in `~/.moltis/hooks/dcg-guard/` (manifest + handler), so destructive command guarding is available out of the box once `dcg` is installed
- Swift embedding POC scaffold with a new `moltis-swift-bridge` static library crate, XcodeGen YAML project (`apps/macos/project.yml`), and SwiftLint wiring for SwiftUI frontend code quality
- New `moltis-openclaw-import` crate for detecting OpenClaw installations and selectively importing identity, providers, skills, memory files, Telegram channels, sessions, and MCP servers
- New onboarding RPC methods: `openclaw.detect`, `openclaw.scan`, and `openclaw.import`
- New `moltis import` CLI commands (`detect`, `all`, `select`) with `--dry-run` and `--json` output options
- Onboarding now includes a conditional OpenClaw Import step with category selection, import execution, and detailed per-category results/TODO reporting
- Settings now includes an OpenClaw Import section (shown only when OpenClaw is detected) for scan-and-import workflows after onboarding
- Microsoft Teams channel integration via new `moltis-msteams` plugin crate with webhook ingress and OAuth client-credentials outbound messaging
- Teams channel management in the web UI (add/edit/remove accounts, sender review, session/channel badges)
- Guided Teams bootstrap tooling via `moltis channels teams bootstrap` plus an in-UI endpoint generator in Settings → Channels
- Multi-agent personas with per-agent workspaces (`data_dir()/agents/<id>/`), `agents.*` RPC methods, and session-level `agent_id` binding/switching across web + Telegram flows
- `chat.peek` RPC method returning real-time session state (active flag, thinking text, active tool calls) for any session key
- Active tool call tracking per-session in `LiveChatService` with camelCase-serialized `ActiveToolCall` structs
- Web UI: inline red "Stop" button inside thinking indicator, `aborted` broadcast handler that cleans up streaming state
- Channel commands: `/peek` (shows thinking text and active tool calls) and `/stop` (aborts active generation)
### Changed

- **Crate restructure**: gateway crate reduced from ~42K to ~29K lines by extracting `moltis-chat` (chat engine, agent orchestration), `moltis-auth` (password + passkey auth), `moltis-tls` (TLS/HTTPS termination), `moltis-service-traits` (shared service interfaces), and moving share rendering into `moltis-web`
- Provider wiring now routes through `moltis-providers` instead of `moltis-agents::providers`, and local LLM feature flags (`local-llm`, `local-llm-cuda`, `local-llm-metal`) now resolve via `moltis-providers`
- Voice now auto-selects the first configured TTS/STT provider when no explicit
  provider is set.
- Default voice template/settings now favor OpenAI TTS and Whisper STT in
  onboarding-ready configs.
- Updated the `dcg-guard` example hook docs and handler behavior to gracefully no-op when `dcg` is missing, instead of hard-failing
- Automatic model/provider selection now prefers subscription-backed providers (OpenAI Codex, GitHub Copilot) ahead of API-key providers, while still honoring explicit model priorities
- GraphQL gateway now builds its schema once at startup and reuses it for HTTP and WebSocket requests
- GraphQL resolvers now share common RPC helper macros and use typed response objects for `node.describe`, `voice.config`, `voice.voxtral_requirements`, `skills.security_status`, `skills.security_scan`, and `memory.config`
- GraphQL `logs.ack` mutation now matches backend behavior and no longer takes an `ids` argument
- Gateway startup diagnostics now report OpenClaw detection status and pass detection state to web gon data for conditional UI rendering
- Gateway and CLI now enable the `openclaw-import` feature in default builds
- Providers now support `stream_transport = "sse" | "websocket" | "auto"` in config. OpenAI can stream via Responses API WebSocket mode, and `auto` falls back to SSE when WebSocket setup is unavailable.
- Agent Identity emoji picker now includes 🐰 🐹 🦀 🦞 🦝 🦭 🧠 🧭 options
- Added architecture docs for a native Swift UI app embedding Moltis Rust core through a C FFI bridge (`docs/src/native-swift-embedding.md`)
- Channel persistence and message-log queries are now channel-type scoped (`channel_type + account_id`) so Telegram and Teams accounts can share the same account IDs safely
- Chat/system prompt resolution is now agent-aware, loading `IDENTITY.md`, `SOUL.md`, `MEMORY.md`, `AGENTS.md`, and `TOOLS.md` from the active session agent workspace with backward-compatible fallbacks
- Memory tool operations and compaction memory writes are now agent-scoped, preventing cross-agent memory leakage during search/read/write flows
- Default sandbox package set now includes `golang-go`, and pre-built sandbox images install the latest `gog` (`steipete/gogcli`) as `gog` and `gogcli`
- Sandbox config now supports `/home/sandbox` persistence strategies (`off`, `session`, `shared`), with `shared` as the default and a shared host folder mounted from `data_dir()/sandbox/home/shared`
- Settings → Sandboxes now includes shared-home controls (enabled + folder path), and sandbox config supports `tools.exec.sandbox.shared_home_dir` for custom shared persistence location

### Deprecated

### Removed

### Fixed

- **OpenAI Codex OAuth in Docker**: the web UI no longer overrides the provider's pre-registered `redirect_uri`, which caused OpenAI to reject the authorization request with `unknown_error`. The OAuth callback server now also respects the gateway bind address (`0.0.0.0` in Docker) so the callback port (1455) is reachable from the host. Docker image now exposes port 1455 for OAuth callbacks (#207)
- **Slow SQLite writes**: `moltis.db` and `memory.db` now use `journal_mode=WAL` and `synchronous=NORMAL` (matching `metrics.db`), eliminating multi-second write contention that caused 3–10 s INSERT times under concurrent access
- Channel image delivery now parses the actual MIME type from data URIs instead of hardcoding `image/png`
- Docker image now installs Docker CLI from Docker’s official Debian repository (`docker-ce-cli`), avoiding API mismatches with newer host daemons during sandbox builds/exec
- Chat UI now shows a first-run sandbox preparation status message before container/image setup begins, so startup delays are visible while sandbox resources are created
- OpenAI TTS and Whisper STT now correctly reuse OpenAI credentials from
  voice config, `OPENAI_API_KEY`, or the LLM OpenAI provider config.
- Voice provider parsing now accepts `openai-tts` and `google-tts` aliases
  sent by the web UI.
- Chat welcome card is now hidden as soon as the thinking indicator appears.
- Onboarding summary loading state now keeps modal sizing stable with a
  centered spinner.
- Onboarding voice provider rows now use a dedicated `needs-key` badge class and styling, with E2E coverage to verify the badge pill rendering
- OpenAI Codex OAuth token handling now preserves account context across refreshes and resolves `ChatGPT-Account-Id` from additional JWT/auth.json shapes to avoid auth failures with Max-style OAuth flows
- Onboarding/provider setup now surfaces subscription OAuth providers (OpenAI Codex, GitHub Copilot) as configured when local OAuth tokens are present, even if they are omitted from `providers.offered`
- GraphQL WebSocket upgrade detection now accepts clients that provide `Upgrade`/`Sec-WebSocket-Key` without `Connection: upgrade`
- GraphQL channel and memory status bridges now return schema-compatible shapes for `channels.status`, `channels.list`, and `memory.status`
- Provider errors with `insufficient_quota` now surface as explicit quota/billing failures (with the upstream message) instead of generic retrying/rate-limit behavior
- Linux `aarch64` builds now skip `jemalloc` to prevent startup aborts on 16 KiB page-size kernels (for example Raspberry Pi 5 Debian images)
- Gateway startup now blocks the common reverse-proxy TLS mismatch (`MOLTIS_BEHIND_PROXY=true` with Moltis TLS enabled) and explains using `--no-tls`; HTTPS-upstream proxy setups can explicitly opt in with `MOLTIS_ALLOW_TLS_BEHIND_PROXY=true`
- WebSocket same-origin checks now accept proxy deployments that rewrite `Host` by using `X-Forwarded-Host` in proxy mode, and treat implicit `:443`/`:80` as equivalent to default ports
### Security

## [0.9.10] - 2026-02-21


### Added

### Changed

### Deprecated

### Removed

### Fixed

### Security

## [0.9.9] - 2026-02-21


### Added

### Changed

### Deprecated

### Removed

### Fixed

### Security

## [0.9.8] - 2026-02-21


### Added

### Changed

### Deprecated

### Removed

### Fixed

### Security

## [0.9.7] - 2026-02-20


### Added

### Changed

### Deprecated

### Removed

### Fixed

### Security

## [0.9.6] - 2026-02-20


### Added

- Cron jobs can now deliver agent turn output to Telegram channels via the `deliver`, `channel`, and `to` payload fields

### Changed

### Deprecated

### Removed

### Fixed

- Accessing `http://` on the HTTPS port now returns a 301 redirect to `https://` instead of a garbled TLS handshake page
- SQLite metrics store now uses WAL journal mode and `synchronous=NORMAL` to fix slow INSERT times (1-3s) on Docker/WSL2

### Security

## [0.9.5] - 2026-02-20


### Added

### Changed

### Deprecated

### Removed

### Fixed

- Skip jemalloc on Windows (platform-specific dependency gate)

### Security

## [0.9.4] - 2026-02-20


### Added

### Changed

### Deprecated

### Removed

### Fixed

### Security

## [0.9.3] - 2026-02-20


### Added

### Changed

### Deprecated

### Removed

### Fixed

### Security

## [0.9.2] - 2026-02-20


### Added

- Event-driven heartbeat wake system: cron jobs can now trigger immediate
  heartbeat runs via a `wakeMode` field (`"now"` or `"nextHeartbeat"`).
- System events queue: in-memory bounded buffer that collects events (exec
  completions, cron triggers) and drains them into the heartbeat prompt so the
  agent sees what happened while it was idle.
- Exec completion callback: command executions automatically enqueue a summary
  event and wake the heartbeat, giving the agent real-time awareness of
  background task results.

### Changed

### Deprecated

### Removed

### Fixed

### Security

## [0.9.1] - 2026-02-19


### Added

- `lightweight` feature profile for memory-constrained devices (Raspberry Pi, etc.)
  with only essential features: `jemalloc`, `tls`, `web-ui`.
- jemalloc allocator behind `jemalloc` feature flag for lower memory fragmentation.
- Configurable `history_points` (metrics) and `log_buffer_size` (server) settings
  to tune in-memory buffer sizes.
- Persistent browser profiles: cookies, auth state, and local storage now persist
  across sessions by default. Disable with `persist_profile = false` in
  `[tools.browser]`, or set a custom path with `profile_dir`. (#162)
- Added `examples/docker-compose.coolify.yml` plus Docker/cloud deploy docs for
  self-hosted Coolify (e.g. Hetzner), including reverse-proxy defaults and
  Docker socket mount guidance for sandboxed exec support.
- Markdown and ANSI table rendering in chat messages.
- Provider-aware `show_map` links for multi-provider map display.
- Session history caching with visual switch loader for faster session
  transitions.

### Changed

- MetricsHistory default reduced from 60,480 to 360 points (~170x less memory).
- LogBuffer default reduced from 10,000 to 1,000 entries.
- Shared `reqwest::Client` singleton in `moltis-agents` and `moltis-tools` replaces
  per-call client creation, saving connection pools and TLS session caches.
- WebSocket client channels changed from unbounded to bounded (512), adding
  backpressure for slow consumers.
- Release profile: `panic = "abort"` and `codegen-units = 1` for smaller binaries.

### Deprecated

### Removed

### Fixed

- Onboarding identity save now captures browser timezone and persists it to
  `USER.md` via `user_timezone`, so first-run profile setup records the user's
  timezone alongside their name.
- Runtime prompt host metadata now prefers user/browser timezone over server
  local fallback and includes an explicit `today=YYYY-MM-DD` field so models
  can reliably reason about the user's current date.
- Skills installation now supports Claude marketplace repos that define skills
  directly via `.claude-plugin/marketplace.json` `plugins[].skills[]` paths
  (for example `anthropics/skills`), including loading `SKILL.md` entries under
  `skills/*` and exposing them through the existing plugin-skill workflow.
- Web search no longer falls back to DuckDuckGo by default when search API keys
  are missing, avoiding repeated CAPTCHA failures; fallback is now opt-in via
  `tools.web.search.duckduckgo_fallback = true`.
- Terminal: force tmux window resize on client viewport change to prevent
  stale dimensions after reconnect.
- Browser: profile persistence now works correctly on Apple Container
  (macOS containerized sandbox).
- Browser: centralized stale CDP connection detection prevents ghost browser
  sessions from accumulating. (#172)
- Gateway: deduplicate voice replies on Telegram channels to prevent echo
  loops. (#173)
- Cron job editor: fix modal default validation and form reset when switching
  schedule type. (#181)
- MCP: strip internal metadata from tool call arguments before forwarding to
  MCP servers.
- Web search: load runtime env keys and improve Brave search response
  parsing robustness.
- Prompt: clarify sandbox vs `data_dir` path semantics in system prompts.
- Gateway: align `show_map` listing ratings to the right for consistent
  layout.

### Security

## [0.9.0] - 2026-02-17


### Added

- Settings > Cron job editor now supports per-job LLM model selection and
  execution target selection (`host` or `sandbox`), including optional
  sandbox image override when sandbox execution is selected.

### Changed

- Configuration documentation examples now match the current schema
  (`[server]`, `[identity]`, `[tools]`, `[hooks.hooks]`,
  `[mcp.servers.<name>]`, and `[channels.telegram.<account>]`), including
  updated provider and local-LLM snippets.

### Deprecated

### Removed

### Fixed

- Agent loop iteration limit is now configurable via
  `tools.agent_max_iterations` in `moltis.toml` (default `25`) instead of
  being hardcoded at runtime.

### Security

## [0.8.38] - 2026-02-17


### Added

- `show_map` now supports multi-point maps via `points[]`, rendering all
  destinations in one screenshot with auto-fit zoom/centering, while keeping
  legacy single-point fields for backward compatibility.
- Telegram channel reply streaming via edit-in-place updates, with per-account
  `stream_mode` gating so `off` keeps the classic final-message delivery path.
- Telegram per-account `stream_notify_on_complete` option to send a final
  non-silent completion message after edit-in-place streaming finishes.
- Telegram per-account `stream_min_initial_chars` option (default `30`) to
  delay the first streamed message until enough text has accumulated.

### Changed

### Deprecated

### Removed

### Fixed

### Security

## [0.8.37] - 2026-02-17


### Added

- Settings > Terminal now includes tmux window tabs for the managed
  `moltis-host-terminal` session, plus a `+ Tab` action to create new tmux
  windows from the UI.
- New terminal window APIs: `GET /api/terminal/windows` and
  `POST /api/terminal/windows` to list and create host tmux windows.
- Host terminal websocket now supports `?window=<id|index>` targeting and
  returns `activeWindowId` in the ready payload.

### Changed

- Web chat now supports `/sh` command mode: entering `/sh` toggles a dedicated
  command input state, command sends are automatically prefixed with `/sh`,
  and the token bar shows effective execution route (`sandboxed` vs `host`)
  plus prompt symbol (`#` for root, `$` for non-root).
- Settings > Terminal now polls tmux windows and updates tabs automatically,
  so windows created inside tmux (for example `Ctrl-b c`) appear in the web UI.
- Host terminal tmux integration now uses a dedicated tmux socket and applies
  a Moltis-friendly profile (status off, mouse off, stable window naming).
- Settings > Terminal subtitle now omits the prompt symbol hint so it does not
  show stale `$`/`#` information after privilege changes inside the shell.

### Deprecated

### Removed

### Fixed

- Apple Container sandbox startup now pins `--workdir /tmp`, bootstraps
  `/home/sandbox` before `sleep infinity`, and uses explicit exec workdirs to
  avoid `WORKDIR` chdir failures when image metadata directories are missing.
- Cron tool job creation/update now accepts common shorthand schedule/payload
  shapes (including cron expression strings) and normalizes them before
  validation, reducing model-side schema mismatch failures.
- Force-exec fallback now triggers only for explicit `/sh ...` input (including
  `/sh@bot ...`), preventing casual chat messages like `hey` from being treated
  as shell commands while still allowing normal model-driven exec tool use.
- Tool-mode system prompt guidance is now conversation-first and documents the
  `/sh` explicit shell prefix.
- Chat auto-compaction now uses estimated next-request prompt tokens (current
  context pressure) instead of cumulative session token totals, and chat context
  UI now separates cumulative usage from current/estimated request context.
- Settings > Terminal tab switching now uses in-band tmux window switching over
  the active websocket, reducing redraw/cursor corruption when switching
  between tmux windows (including fullscreen apps like `vim`).
- Host terminal tmux attach now resets window sizing to auto (`resize-window -A`)
  to prevent stale oversized window dimensions across reconnects.
- Settings > Terminal tmux window polling now continues after tab switches, so
  windows closed with `Ctrl-D` are removed from the tab strip automatically.
- Settings > Terminal now recovers from stale `?window=` reconnect targets
  after a tmux window is closed, attaching to the current window instead of
  getting stuck in a reconnect loop.
- Settings > Terminal host PTY output is now transported as raw bytes
  (base64-encoded over websocket) instead of UTF-8-decoded text, fixing
  rendering/control-sequence corruption in full-screen terminal apps like `vim`.
- Settings > Terminal now force-syncs terminal size on connect/window switch so
  newly created tmux windows attach at full viewport size instead of a smaller
  default geometry.
- Settings > Terminal now ignores OSC color/palette mutation sequences from
  full-screen apps to avoid invisible-text redraw glitches when switching tmux
  tabs (notably seen with `vim`).
- Settings > Terminal now re-sends forced resize updates during a short
  post-connect settle window, fixing initial page-reload cases where tmux
  windows stayed at stale dimensions until a manual tab switch.

### Security

## [0.8.36] - 2026-02-16


### Added

- OAuth 2.1 support for remote MCP servers — automatic discovery (RFC 9728/8414), dynamic client registration (RFC 7591), PKCE authorization code flow, and Bearer token injection with 401 retry
- `McpOAuthOverride` config option for servers that don't implement standard OAuth discovery
- `mcp.reauth` RPC method to manually trigger re-authentication for a server
- Persistent storage of dynamic client registrations at `~/.config/moltis/mcp_oauth_registrations.json`
- **SSRF allowlist**: `tools.web.fetch.ssrf_allowlist` config field to exempt trusted
  CIDR ranges from SSRF blocking, enabling Docker inter-container networking.
- Memory config: add `memory.disable_rag` to force keyword-only memory search while keeping markdown indexing and memory tools enabled
- Generic OpenAI-compatible provider support: connect any OpenAI-compatible endpoint via the provider setup UI, with domain-derived naming (`custom-` prefix), model auto-discovery, and full model selection
### Changed

### Deprecated

### Removed

### Fixed

- **Telegram queued replies**: route channel reply targets per queued message so
  `chat.message_queue_mode = "followup"` delivers replies one-by-one instead of
  collapsing queued channel replies into a single batch delivery.
- **Queue mode default**: make one-by-one replay (`followup`) explicit as the
  `ChatConfig` default, with config-level tests to prevent regressions.
- MCP OAuth dynamic registration now uses the exact loopback callback URI selected for the current auth flow, improving compatibility with providers that require strict redirect URI matching (for example Linear).
- MCP manager now applies `[mcp.servers.<name>.oauth]` override settings when building the OAuth provider for SSE servers.
- Streamable HTTP MCP transport now persists and reuses `Mcp-Session-Id`, parses `text/event-stream` responses, and sends best-effort `DELETE` on shutdown to close server sessions.
- MCP docs/config examples now use the current table-based config shape and `/mcp` endpoint examples for remote servers.
- Memory embeddings endpoint composition now avoids duplicated path segments like `/v1/v1/embeddings` and accepts base URLs ending in host-only, `/v1`, versioned paths (for example `/v4`), or `/embeddings`
### Security

## [0.8.35] - 2026-02-15


### Added

- Add memory target routing guidance to `memory_save` prompt hint — core facts go to MEMORY.md, everything else to `memory/<topic>.md` to keep context lean

### Changed

### Deprecated

### Removed

### Fixed

### Security

## [0.8.34] - 2026-02-15


### Added

- Add explicit `memory_save` hint in system prompt so weaker models (MiniMax, etc.) call the tool when asked to remember something
- Add anchor text after memory content so models don't ignore known facts when `memory_search` returns empty
- Add `zai` to default offered providers in config template

### Changed

### Deprecated

### Removed

### Fixed

### Security

## [0.8.33] - 2026-02-15


### Added

### Changed

### Deprecated

### Removed

### Fixed

- **CI**: remove unnecessary `std::path::` qualification in gateway server flagged
  by nightly clippy.

### Security

## [0.8.32] - 2026-02-15


### Added

### Changed

### Deprecated

### Removed

### Fixed

- **CI**: gate macOS-only sandbox helper functions with `#[cfg]` to fix dead-code
  errors on Linux CI.

### Security

## [0.8.31] - 2026-02-15


### Added

- **Sandbox toggle notification**: when the sandbox is enabled or disabled
  mid-session, a system message is injected into the conversation history so
  the LLM knows the execution environment changed. A chat notice also appears
  in the UI immediately.

- **Config `[env]` section**: environment variables defined in `[env]` in
  `moltis.toml` are injected into the Moltis process at startup. This makes
  API keys (Brave, OpenRouter, etc.) available to features that read from
  `std::env::var()`. Process env vars (`docker -e`, host env) take precedence.
  Closes #107.
- **Browser auto-detection and install**: automatically detect all installed
  Chromium-family browsers (Chrome, Chromium, Edge, Brave, Opera, Vivaldi, Arc)
  and auto-install via the system package manager when none is found. Requests
  can specify a preferred browser (`"browser": "brave"`) or let the system
  pick the first available one.
- **Z.AI provider**: add Z.AI (Zhipu) as an OpenAI-compatible provider with
  static model catalog (GLM-5, GLM-4.7, GLM-4.6, GLM-4.5 series) and dynamic
  model discovery via `/models` endpoint. Supports tool calling, streaming,
  vision (GLM-4.6V/4.5V), and reasoning content.
- **Running Containers panel**: the Settings > Sandboxes page now shows a
  "Running Containers" section listing all moltis-managed containers with
  live state (running/stopped/exited), backend type (Apple Container/Docker),
  resource info, and Stop/Delete actions. Includes disk usage display
  (container/image counts, sizes, reclaimable space) and a "Clean All"
  button to stop and remove all stale containers at once.
- **Startup container GC**: the gateway now automatically removes orphaned
  session containers on startup, preventing disk space accumulation from
  crashed or interrupted sessions.
- **Download full context as JSONL**: the full context panel now has a
  "Download" button that exports the conversation (including raw LLM
  responses) as a timestamped `.jsonl` file.
- **Sandbox images in cached images list**: the Settings > Images page
  now merges sandbox-built images into the cached images list so all
  container images are visible in one place.

### Changed

- **Sandbox image identity**: image tags now use SHA-256 instead of
  `DefaultHasher` for deterministic, cross-run hashing of base image +
  packages.

### Deprecated

### Removed

### Fixed

- **Thinking indicator lost on reload**: reloading the page while the model
  was generating no longer loses the "thinking" dots. The backend now includes
  `replying` state in `sessions.list` and `sessions.switch` RPC responses so
  the frontend can restore the indicator after a full page reload.
- **Thinking text restored after reload**: reloading the page during extended
  thinking (reasoning) now restores the accumulated thinking text instead of
  showing only bouncing dots. The backend tracks thinking text per session and
  returns it in the `sessions.switch` response.
- **Apple Container recovery**: simplify container recovery to a single flat
  retry loop (3 attempts max, down from up to 24). Name rotation now only
  triggers on `AlreadyExists` errors, preventing orphan containers. Added
  `notFound` error matching so exec readiness probes retry correctly.
  Diagnostic info (running container count, service health, container logs)
  is now included in failure messages. Detect stale Virtualization.framework
  state (`NSPOSIXErrorDomain EINVAL`) and automatically restart the daemon
  (`container system stop && container system start`) before retrying; bail
  with a clear remediation message only if automatic restart fails.
  Exec-level recovery retries reduced from 3 to 1.
- **Ghost Apple Containers**: failed container deletions are now tracked
  in a zombie set and filtered from list output, preventing stale entries
  from reappearing in the Running Containers panel.
- **Container action errors preserved**: failed delete/clean/restart
  operations now surface the original error message to the UI instead of
  silently swallowing it.
- **Usage parsing across OpenAI-compatible providers**: token counts now
  handle Anthropic-style (`input_tokens`/`output_tokens`), camelCase
  variants, cache token fields, and multiple response nesting structures
  across diverse providers.
- **Think tag whitespace**: leading whitespace after `</think>` close
  tags is now stripped, preventing extra blank lines in streamed output.
- **Token bar visible at zero**: the token usage bar no longer disappears
  when all counts are zero; it stays visible as a baseline indicator.

### Security

## [0.8.30] - 2026-02-15


### Added

### Changed

- **Assistant reasoning persistence**: conversation reasoning is now persisted
  in assistant messages and shared snapshots so resumed sessions retain
  reasoning context instead of dropping it after refresh/share operations.

### Deprecated

### Removed

### Fixed

### Security

## [0.8.29] - 2026-02-14


### Added

- **Memory bootstrap**: inject `MEMORY.md` content directly into the system
  prompt (truncated at 20,000 chars) so the agent always has core memory
  available without needing to call `memory_search` first. Matches OpenClaw's
  bootstrap behavior
- **Memory save tool**: new `memory_save` tool lets the LLM write to long-term
  memory files (`MEMORY.md` or `memory/<name>.md`) with append/overwrite modes
  and immediate re-indexing for search

### Changed

- **Memory writing**: `MemoryManager` now implements the `MemoryWriter` trait
  directly, unifying read and write paths behind a single manager. The silent
  memory turn and `MemorySaveTool` both delegate to the manager, which handles
  path validation, size limits, and automatic re-indexing after writes

### Deprecated

### Removed

### Fixed

- **Memory file watcher**: the file watcher now covers `MEMORY.md` at the data
  directory root, which was previously excluded because the filter only matched
  directories

### Security

## [0.8.28] - 2026-02-14


### Added

### Changed

- **Browser sandbox resolution**: `BrowserTool` now resolves sandbox mode
  directly from `SandboxRouter` instead of relying on a `_sandbox` flag
  injected via tool call params.

### Deprecated

### Removed

### Fixed

- **E2E onboarding failures**: Fixed missing `saveProviderKey` export in
  `provider-validation.js` that was accidentally left unstaged in the DRY
  refactoring commit.

### Security

## [0.8.27] - 2026-02-14


### Added

### Changed

- **DRY voice/identity/channel utils**: Extracted shared RPC wrappers and
  validation helpers from `onboarding-view.js` and `page-settings.js` /
  `page-channels.js` into dedicated `voice-utils.js`, `identity-utils.js`,
  and `channel-utils.js` modules.

### Deprecated

### Removed

### Fixed

- **Config test env isolation**: Fixed spurious
  `save_config_to_path_removes_stale_keys_when_values_are_cleared` test
  failure caused by `MOLTIS_IDENTITY__NAME` environment variable leaking
  into the test via `apply_env_overrides`.

### Security

## [0.8.26] - 2026-02-14


### Added

- **Rustls/OpenSSL migration roadmap**: Added
  `plans/2026-02-14-rustls-migration-and-openssl-reduction.md` with a staged
  plan to reduce OpenSSL coupling, isolate feature gates, and move default TLS
  networking paths toward rustls.

### Changed

### Deprecated

### Removed

### Fixed

- **Windows release build reliability**: The `.exe` release workflow now forces
  Strawberry Perl (`OPENSSL_SRC_PERL`/`PERL`) so vendored OpenSSL builds do not
  fail due to missing Git Bash Perl modules.
- **OpenAI tool-call ID length**: Remap tool-call IDs that exceed OpenAI's
  40-character limit during message serialization, and generate shorter
  synthetic IDs in the agent runner to prevent API errors.
- **Onboarding credential persistence**: Provider credentials are now saved
  before opening model selection during onboarding, aligning behavior with the
  Settings > LLM flow.

### Security

## [0.8.25] - 2026-02-14


### Added

### Changed

### Deprecated

### Removed

### Fixed

### Security

## [0.8.24] - 2026-02-13


### Added

### Changed

### Deprecated

### Removed

### Fixed

### Security

## [0.8.23] - 2026-02-13


### Added
- **Multi-select preferred models per provider**: The LLMs page now has a
  "Preferred Models" button per provider that opens a multi-select modal.
  Selected models are pinned at the top of the session model dropdown.
  New `providers.save_models` RPC accepts multiple model IDs at once.
- **Multi-select model picker in onboarding**: The onboarding provider step now
  uses a multi-select model picker matching the Settings LLMs page. Toggle
  models on/off, see per-model probe status badges, and batch-save with a
  single Save button. Previously-saved preferred models are pre-selected when
  re-opening the model selector.

### Changed

- **Model discovery uses `DiscoveredModel` struct**: Replaced `(String, String)`
  tuples with a typed `DiscoveredModel` struct across all providers (OpenAI,
  GitHub Copilot, OpenAI Codex). The struct carries an optional `created_at`
  timestamp from the `/v1/models` API, enabling discovered models to be sorted
  newest-first. Preferred/configured models remain pinned at the top.
- **Removed OpenAI-specific model name filtering from discovery**: The
  `/v1/models` response is no longer filtered by OpenAI naming conventions
  (`gpt-*`, `o1`, etc.). All valid model IDs from any provider are now
  accepted. This fixes model discovery for third-party providers like
  Moonshot whose model IDs don't follow OpenAI naming.
- **Disabled automatic model probe at startup**: The background chat
  completion probe that checked which models are supported is now
  triggered on-demand by the web UI instead of running automatically
  2 seconds after startup. With dynamic model discovery, the startup
  probe was expensive and noisy (non-chat models like image, audio,
  and video would log spurious warnings).
- **Model test uses streaming for faster feedback**: The "Testing..."
  probe when selecting a model now uses streaming and returns on the
  first token instead of waiting for a full non-streaming response.
  Timeout reduced from 20s to 10s.
- **Chosen models merge with config-defined priority**: Models selected
  via the UI are prepended to the saved models list and merged with
  config-defined preferred models, so both sources contribute to
  ordering.
- **Dynamic cross-provider priority list**: The model dropdown priority
  is now a shared `Arc<RwLock<Vec<String>>>` updated at runtime when
  models are saved, instead of a static `HashMap` built once at startup.
- **Replaced hardcoded Ollama checks with `keyOptional` metadata**: JS
  files no longer check `provider.name === "ollama"` for behavior.
  Instead, the backend exposes a `keyOptional` field on provider
  metadata, making the UI provider-agnostic.

### Fixed

- **Settings UI env vars now available process-wide**: environment variables
  set via Settings > Environment were previously only injected into sandbox
  commands. They are now also injected into the Moltis process at startup,
  making them available to web search, embeddings, and provider API calls.
## [0.8.14] - 2026-02-11

### Security

- **Disconnect all WS clients on credential change**: WebSocket connections
  opened before auth setup are now disconnected when credentials change
  (password set/changed, passkey registered during setup, auth reset, last
  credential removed). An `auth.credentials_changed` event notifies browsers
  to redirect to `/login`. Existing sessions are also invalidated on password
  change for defense-in-depth.

### Fixed

- **Onboarding test for SOUL.md clear behavior**: Fixed `identity_update_partial`
  test to match the new empty-file behavior from v0.8.13.

## [0.8.13] - 2026-02-11

### Added

- **Auto-create SOUL.md on first run**: `SOUL.md` is now seeded with the
  default soul text when the file doesn't exist, mirroring how `moltis.toml`
  is auto-created. If deleted, it re-seeds on next load.

### Fixed

- **SOUL.md clear via settings**: Clearing the soul textarea in settings no
  longer re-creates the default on the next load. An explicit clear now writes
  an empty file to distinguish "user cleared soul" from "file never existed".
- **Onboarding WS connection timing**: Deferred WebSocket connection until
  authentication completes, preventing connection failures during onboarding.

### Changed

- **Passkey auth preselection**: Onboarding now preselects the passkey
  authentication method when a passkey is already registered.
- **Moonshot provider**: Added Moonshot to the default offered providers list.

## [0.8.12] - 2026-02-11

### Fixed

- **E2E test CI stability**: `NoopChatService::clear()` now returns Ok instead
  of an error when no LLM providers are configured, fixing 5 e2e test failures
  in CI environments. Hardened websocket, chat-input, and onboarding-auth e2e
  tests against startup race conditions and flaky selectors.

## [0.8.8] - 2026-02-11

### Changed

- **Sessions sidebar layout**: Removed the top `Sessions` title row and moved
  the new-session `+` action next to the session search field for a more
  compact list header.
- **Identity autosave UX**: Name fields in Settings > Identity now autosave on
  input blur, matching the quick-save behavior used for emoji selection.
- **Favicon behavior by browser**: Identity emoji changes now update favicon
  live; Safari-specific reload guidance is shown only when Safari is detected.
- **Page title format**: Browser title now uses the configured assistant name
  only, without appending `AI assistant` suffix text.

## [0.8.7] - 2026-02-11

### Added

- **Model allowlist probe output support**: Model allowlist matching now handles
  provider probe output more robustly and applies stricter matching semantics.
- **Ship helper command**: Added a `just ship` task and `scripts/ship-pr.sh`
  helper to streamline commit, push, PR update, and local validation workflows.

### Changed

- **Gateway titles and labels**: Login/onboarding page titles now consistently
  use configured values and identity emoji; UI copy now labels providers as
  `LLM` where appropriate.
- **Release binary profile**: Enabled ThinLTO and binary stripping in the
  release profile to reduce artifact size.
- **SPA route handling**: Centralized SPA route definitions and preserved TOML
  comments during config updates.

### Fixed

- **Auth setup hardening**: Enforced authentication immediately after password
  or passkey setup to prevent unintended post-setup unauthenticated access.
- **Streaming event ordering**: Preserved gateway chat stream event ordering to
  avoid out-of-order UI updates during streaming responses.
- **Sandbox fallback pathing**: Exec fallback now uses the host data directory
  when no container runtime is available.

## [0.8.6] - 2026-02-11

### Changed

- **Release workflow gates E2E tests**: Build Packages workflow now runs E2E
  tests and blocks all package builds (deb, rpm, arch, AppImage, snap,
  Homebrew, Docker) if they fail.

### Added

- **XML tag stripping**: Strip internal XML tags from LLM responses to prevent
  tag leakage in chat (thinking, reasoning, scratchpad, etc.)
- **Runtime model metadata**: Fetch model metadata from provider APIs for
  accurate context window detection during auto-compaction
- **Run detail UI**: Panel showing tool calls and message flow for agent runs,
  accessible via expandable button on assistant messages

### Fixed

- **Docker TLS setup**: All Docker examples now expose port 13132 for CA
  certificate download with instructions to trust the self-signed cert,
  fixing HTTPS access in Safari and other strict browsers.
- **E2E onboarding-auth test**: The `auth` Playwright project's `testMatch`
  regex `/auth\.spec/` also matched `onboarding-auth.spec.js`, causing it to
  run against the default gateway (wrong server) instead of the onboarding-auth
  gateway. Tightened regex to `/\/auth\.spec/`.

## [0.8.5] - 2026-02-11

### Added

- **CLI `--version` flag**: `moltis --version` now prints the version.
- **Askama HTML rendering**: SPA index and social metadata templates use
  Askama instead of string replacement.

### Fixed

- **WebSocket reconnect after remote onboarding auth**: Connection now
  reconnects immediately after auth setup instead of waiting for the backoff
  timer, fixing "WebSocket not connected" errors during identity save.
- **Passkeys on Fly.io**: Auto-detect WebAuthn RP ID from `FLY_APP_NAME`
  environment variable (constructs `{app}.fly.dev`).
- **PaaS proxy detection**: Added explicit `MOLTIS_BEHIND_PROXY=true` to
  `render.yaml` and `fly.toml` so auth middleware reliably detects remote
  connections behind the platform's reverse proxy.
- **WebAuthn origin scheme on PaaS**: Non-localhost RP IDs now default to
  `https://` origin since PaaS platforms terminate TLS at the proxy.

### Security

- **Compaction prompt injection hardening**: Session compaction now passes
  typed `ChatMessage` objects to the summarizer LLM instead of concatenated
  `{role}: {content}` text, preventing role-spoofing prompt injection where
  user content could mimic role prefixes (similar to GHSA-g8p2-7wf7-98mq).

## [0.8.4] - 2026-02-11

### Changed

- **Session delete UX**: Forked sessions with no new messages beyond the fork
  point are deleted immediately without a confirmation dialog.

### Fixed

- **Localhost passkey compatibility**: Gateway startup URLs and TLS redirect
  hints now use `localhost` for loopback hosts, while WebAuthn also allows
  `moltis.localhost` as an additional origin when RP ID is `localhost`.

## [0.8.3] - 2026-02-11

### Fixed

- **Linux clippy `unused_mut` failure**: Fixed a target-specific `unused_mut`
  warning in browser stale-container cleanup that failed release clippy on
  Linux with `-D warnings`.

## [0.8.2] - 2026-02-11

### Fixed

- **Release clippy environment parity**: The release workflow clippy job now
  runs in the same CUDA-capable environment as main CI, includes the llama
  source git bootstrap step, and installs `rustfmt` alongside `clippy`. This
  fixes release clippy failures caused by missing CUDA toolchain/runtime.

## [0.8.1] - 2026-02-11

### Fixed

- **Clippy validation parity**: Unified local validation, CI (main), and
  release workflows to use the same clippy command and flags
  (`--workspace --all-features --all-targets --timings -D warnings`), which
  prevents release-only clippy failures from command drift.

## [0.8.0] - 2026-02-11

### Added

- **Instance-scoped container naming**: Browser and sandbox container/image
  prefixes now derive from the configured instance name, so multiple Moltis
  instances do not collide.

### Changed

- **Stale container cleanup targeting**: Startup cleanup now removes only
  containers that belong to the active instance prefix instead of sweeping
  unrelated containers.
- **Apple container runtime probing**: Browser container backend checks now use
  the modern Apple container CLI flow (`container image pull --help`) without
  legacy fallback behavior.

### Fixed

- **Release workflow artifacts**: Disabled docker build record artifact uploads
  in release CI to avoid release workflow failures from missing artifact paths.
- **Release preflight consistency**: Pinned nightly toolchain and aligned
  release preflight checks with CI formatting/lint gates.

## [0.7.1] - 2026-02-11

### Fixed

- **Release format gate**: Included missing Rust formatting updates in release
  history so the release workflow `cargo fmt --all -- --check` passes for
  tagged builds.

## [0.7.0] - 2026-02-11

### Added

- **HTTP endpoint throttling**: Added gateway-level per-IP rate limits for
  login (`/api/auth/login`), auth API routes, general API routes, and WebSocket
  upgrades, with `429` responses, `Retry-After` headers, and JSON
  `retry_after_seconds`.
- **Login retry UX**: The login page now disables the password Sign In button
  while throttled and shows a live `Retry in Xs` countdown.

### Changed

- **Auth-aware throttling policy**: IP throttling is now bypassed when auth is
  not required for the current request (authenticated requests, auth-disabled
  mode, and local Tier-2 setup mode). This keeps brute-force protection for
  unauthenticated/auth-required traffic while avoiding localhost friction.
- **Login error copy**: During throttled login retries, the error message stays
  static while the retry countdown is shown only on the button.

### Documentation

- Added throttling/security notes to `README.md`, `docs/src/index.md`,
  `docs/src/authentication.md`, and `docs/src/security.md`.

## [0.6.1] - 2026-02-10

### Fixed

- **Release clippy**: Aligned release workflow clippy command with nightly
  flags (`-Z unstable-options`, `--timings`).
- **Test lint attributes**: Fixed useless outer `#[allow]` on test module
  `use` statement; replaced `.unwrap()` with `.expect()` in auth route tests.

## [0.6.0] - 2026-02-10

### Added

- **CalDAV integration**: New `moltis-caldav` crate providing calendar CRUD
  operations (list calendars, list/create/update/delete events) via the CalDAV
  protocol. Supports Fastmail, iCloud, and generic CalDAV servers with
  multi-account configuration under `[caldav.accounts.<name>]`. Enabled by
  default via the `caldav` feature flag.
- **`BeforeLLMCall` / `AfterLLMCall` hooks**: New modifying hook events that fire
  before sending prompts to the LLM provider and after receiving responses
  (before tool execution). Enables prompt injection filtering, PII redaction,
  and response auditing via shell hooks.
- **Config template**: The generated `moltis.toml` template now lists all 17
  hook events with correct PascalCase names and one-line descriptions.
- **Hook event validation**: `moltis config check` now warns on unknown hook
  event names in the config file.
- **Authentication docs**: Comprehensive `docs/src/authentication.md` with
  decision matrix, credential types, API key scopes, session endpoints,
  and WebSocket auth documentation.

### Fixed

- **Browser container lifecycle**: Browser containers (browserless/chrome)
  now have proper lifecycle management — periodic cleanup removes idle
  instances every 30 seconds, graceful shutdown stops all containers on
  Ctrl+C, and `sessions.clear_all` immediately closes all browser sessions.
  A `Drop` safety net ensures containers are stopped even on unexpected exits.

### Changed

- **Unified auth gate**: All auth decision logic is now in a single
  `check_auth()` function called by one `auth_gate` middleware. This fixes
  the split-brain bug where passkey-only setups (no password) were treated
  differently by 4 out of 5 auth code paths — the middleware used
  `is_setup_complete()` (correct) while the others used `has_password()`
  (incorrect for passkey-only setups).
- **Hooks documentation**: Rewritten `docs/src/hooks.md` with complete event
  reference, corrected `ToolResultPersist` classification (modifying, not
  read-only), and new "Prompt Injection Filtering" section with examples.
- **Logs level filter UI**: Settings -> Logs now shows `DEBUG`/`TRACE` level
  options only when they are enabled by the active tracing filter (including
  target-specific directives). Default view remains `INFO` and above.
- **Logs level filter control**: Settings -> Logs now uses the same combo
  dropdown pattern as the chat model selector for consistent UX.
- **Branch favicon contrast**: Non-main branches now use a high-contrast purple
  favicon variant so branch sessions are visually distinct from `main`.

### Security

- **Content-Security-Policy header**: HTML pages now include a nonce-based CSP
  header (`script-src 'self' 'nonce-<UUID>'`), preventing inline script
  injection (XSS defense-in-depth). The OAuth callback page also gets a
  restrictive CSP.
- **Passkey-only auth fix**: Fixed authentication bypass where passkey-only
  setups (without a password) would incorrectly allow unauthenticated access
  on local connections, because the `has_password()` check returned false
  even though `is_setup_complete()` was true.

## [0.5.0] - 2026-02-09

### Added

- **`moltis doctor` command**: Comprehensive health check that validates config,
  audits security (file permissions, API keys in config), checks directory and
  database health, verifies provider readiness (API keys via config or env vars),
  inspects TLS certificates, and validates MCP server commands on PATH.

### Security

- **npm install --ignore-scripts**: Skill dependency installation now passes
  `--ignore-scripts` to npm, preventing supply chain attacks via malicious
  postinstall scripts in npm packages.
- **API key scope enforcement**: API keys with empty/no scopes are now denied
  access instead of silently receiving full admin privileges. Keys must specify
  at least one scope explicitly (least-privilege by default).

## [0.4.1] - 2026-02-09

### Fixed

- **Clippy lint in map test**: Replace `is_some()`/`unwrap()` with `if let Some` to
  fix `clippy::unnecessary_unwrap` that broke the v0.4.0 release build.

## [0.4.0] - 2026-02-09

### Added

- **Auto-import external OAuth tokens**: At startup, auto-detected provider
  tokens (e.g. Codex CLI `~/.codex/auth.json`) are imported into the central
  `oauth_tokens.json` store so users can manage all providers from the UI.
- **Passkey onboarding**: The security setup step now offers passkey registration
  (Touch ID, Face ID, security keys) as the recommended default, with password
  as a fallback option.
- **`providers.validate_key` RPC method**: Test provider credentials without
  saving them — builds a temporary registry, probes with a "ping" message, and
  returns validation status with available models.
- **`providers.save_model` RPC method**: Save the preferred model for any
  configured provider without changing credentials.
- **`models.test` RPC method**: Test a single model from the live registry with
  a real LLM request before committing to it.
- **Model selection for auto-detected providers**: The Providers settings page
  now shows a "Select Model" button for providers that have available models but
  no preferred model set. This lets users pick their favorite model for
  auto-detected providers (e.g. OpenAI Codex detected from `~/.codex/auth.json`).
- **`show_map` tool**: New LLM-callable tool that composes a static map image
  from OSM tiles with red/blue marker pins (destination + user location), plus
  clickable links to Google Maps, Apple Maps, and OpenStreetMap. Supports
  `user_latitude`/`user_longitude` to show both positions with auto-zoom.
  Solves the "I can't share links" problem in voice mode.
- **Location precision modes**: The `get_user_location` tool now accepts a
  `precision` parameter — `"precise"` (GPS-level, default) for nearby places
  and directions, or `"coarse"` (city-level, faster) for flights, weather, and
  time zones. The LLM picks the appropriate mode based on the user's query.

### Changed

- Show "No LLM Providers Connected" card instead of welcome greeting when no
  providers are configured.
- **Onboarding provider setup**: Credentials are now validated before saving.
  After successful validation, a model selector shows available models for the
  provider. The selected model is tested with a real request before completing
  setup. Clear error messages are shown for common failures (invalid API key,
  rate limits, connection issues).
- **Settings provider setup**: The main Providers settings page now uses the
  same validate-first flow as onboarding. Credentials are validated before
  saving (bad keys are never persisted), a model selector appears after
  validation, and OAuth flows show model selection after authentication.

### Fixed

- **Docker RAM detection**: Fall back to `/proc/meminfo` when `sysinfo` returns
  0 bytes for memory inside Docker/cgroup environments.
- **MLX model suggested on Linux**: Use backend-aware model suggestion so MLX
  models are only suggested on Apple Silicon, not on Linux servers.
- **Host package provisioning noise**: Skip `apt-get` when running as non-root
  with no passwordless sudo, instead of failing with permission denied warnings.
- **Browser image pull without runtime**: Guard browser container image pull to
  skip when no usable container runtime is available (backend = "none").
- **OAuth token store logging**: Replace silent `.ok()?` chains with explicit
  `warn!`/`info!` logging in `TokenStore` load/save/delete for diagnosability.
- **Provider warning noise**: Downgrade "tokens not found" log from `warn!` to
  `debug!` for unconfigured providers (GitHub Copilot, OpenAI Codex).
- **models.detect_supported noise**: Downgrade UNAVAILABLE RPC errors from
  `warn!` to `debug!` since they indicate expected "not ready yet" states.
## [0.3.8] - 2026-02-09

### Changed

- **Release CI parallelization**: Split clippy and test into separate parallel
  jobs in the release workflow for faster feedback on GitHub-hosted runners.

### Fixed

- **CodSpeed workflow zizmor audit**: Pinned `CodSpeedHQ/action@v4` to commit
  SHA to satisfy zizmor's `unpinned-uses` audit.

## [0.3.7] - 2026-02-09

### Fixed

- **Clippy warnings**: Fixed `MutexGuard` held across await in telegram
  test, `field assignment outside initializer` in provider setup test, and
  `items after test module` in gateway services.

## [0.3.6] - 2026-02-09

### Fixed

- **Release CI zizmor audit**: Removed `rust-cache` from the release workflow's
  clippy-test job entirely instead of using `save-if: false`, which zizmor does
  not recognize as a cache-poisoning mitigation.

## [0.3.5] - 2026-02-09

### Fixed

- **Release CI cache-poisoning**: Set `save-if: false` on `rust-cache` in the
  release workflow to satisfy zizmor's cache-poisoning audit for tag-triggered
  workflows that publish artifacts.

## [0.3.4] - 2026-02-09

### Fixed

- **Session file lock contention**: Replaced non-blocking `try_write()` with
  blocking `write()` in `SessionStore::append()` and `replace_history()` so
  concurrent tool-result persists wait for the file lock instead of failing
  with `EAGAIN` (OS error 35).

### Changed

- **Release CI quality gates**: The Build Packages workflow now runs biome,
  format, clippy, and test checks before building any packages, ensuring code
  correctness before artifacts are produced.

## [0.3.3] - 2026-02-09

### Fixed

- **OpenAI Codex token refresh panic**: Made `get_valid_token()` async to fix
  `block_on` inside async runtime panic when refreshing expired OAuth tokens.
- **Channel session binding**: Ensure session row exists before setting channel
  binding, fixing `get_user_location` failures on first Telegram message.
- **Cargo.lock sync**: Lock file now matches workspace version.

## [0.3.0] - 2026-02-08

### Added

- **Silent replies**: The system prompt instructs the LLM to return an empty
  response when tool output speaks for itself, suppressing empty chat bubbles,
  push notifications, and channel replies. Empty assistant messages are not
  persisted to session history.

- **Persist TTS audio to session media**: When TTS is enabled and the reply
  medium is `voice`, the server generates TTS audio, saves it to the session
  media directory, and includes the media path in the persisted assistant
  message. On session reload the frontend renders an `<audio>` player from
  the media API instead of re-generating audio via RPC.

- **Per-session media directory**: Screenshots from the browser tool are now
  persisted to `sessions/media/<key>/` and served via
  `GET /api/sessions/:key/media/:filename`. Session history reload renders
  screenshots from the API instead of losing them. Media files are cleaned
  up when a session is deleted.

- **Process tool for interactive terminal sessions**: New `process` tool lets
  the LLM manage interactive/TUI programs (htop, vim, REPLs, etc.) via tmux
  sessions inside the sandbox. Supports start, poll, send_keys, paste, kill,
  and list actions. Includes a built-in `tmux` skill with usage instructions.

- **Runtime host+sandbox prompt context**: Chat system prompts now include a
  `## Runtime` section with host details (hostname, OS, arch, shell, provider,
  model, session, sudo non-interactive capability) and `exec` sandbox details
  (enabled state, mode, backend, scope, image, workspace mount, network policy,
  session override). Tool-mode prompts also add routing guidance so the agent
  asks before requesting host installs or changing sandbox mode.

- **Telegram location sharing**: Telegram channels now support receiving shared
  locations and live location updates. Live locations are tracked until they
  expire or the user stops sharing.

- **Telegram reply threading**: Telegram channel replies now use
  `reply_to_message_id` to thread responses under the original user message,
  keeping conversations visually grouped in the chat.

- **`get_user_location` tool**: New browser-based geolocation tool lets the LLM
  request the user's current coordinates via the Geolocation API, with a
  permission prompt in the UI.

- **`sandbox_packages` tool**: New tool for on-demand package discovery inside
  the sandbox, allowing the LLM to query available and installable packages at
  runtime.

- **Sandbox package expansions**: Pre-built sandbox images now include expanded
  package groups — GIS/OpenStreetMap, document/office/search,
  image/audio/media/data-processing, and communication packages. Mise is also
  available for runtime version management.

- **Queued message UI**: When a message is submitted while the LLM is already
  responding, it is shown in a dedicated bottom tray with cancel support.
  Queued messages are moved into the conversation only after the current
  response finishes rendering.

- **Full context view**: New "Context" button in the chat header opens a panel
  showing the full LLM messages array sent to the provider, with a Copy button
  for easy debugging.

- **Browser timezone auto-detection**: The gateway now auto-detects the user's
  timezone from the browser via `Intl.DateTimeFormat` and includes it in
  session context, removing the need for manual timezone configuration.

- **Logs download**: New Download button on the logs page streams the JSONL log
  file via `GET /api/logs/download` with gzip/zstd compression.

- **Gateway middleware hardening**: Consolidated middleware into
  `apply_middleware_stack()` with security and observability layers:
  - Replace `allow_origin(Any)` with dynamic host-based CORS validation
    reusing the WebSocket CSWSH `is_same_origin` logic, safe for
    Docker/cloud deployments with unknown hostnames
  - `CatchPanicLayer` to convert handler panics to 500 responses
  - `RequestBodyLimitLayer` (16 MiB) to prevent memory exhaustion
  - `SetSensitiveHeadersLayer` to redact Authorization/Cookie in traces
  - Security response headers (`X-Content-Type-Options`, `X-Frame-Options`,
    `Referrer-Policy`)
  - `SetRequestIdLayer` + `PropagateRequestIdLayer` for `x-request-id`
    correlation across HTTP request logs
  - zstd compression alongside gzip for better ratios

- **Message run tracking**: Persisted messages now carry `run_id` and `seq`
  fields for parent/child linking across multi-turn tool runs, plus a
  client-side sequence number for ordering diagnostics.

- **Cache token metrics**: Provider responses now populate cache-hit and
  cache-miss token counters in the metrics subsystem.

### Changed

- **Provider auto-detection observability**: When no explicit provider settings are present in `moltis.toml`, startup now logs each auto-detected provider with its source (`env`, config file key, OAuth token file, provider key file, or Codex auth file). Added `server.http_request_logs` (Axum HTTP traces) and `server.ws_request_logs` (WebSocket RPC request/response traces) config options (both default `false`) for on-demand transport debugging without code changes.
- **Dynamic OpenAI Codex model catalog**: OpenAI Codex providers now load model IDs from `https://chatgpt.com/backend-api/codex/models` at startup (with fallback defaults), and the gateway refreshes Codex models hourly so long-running sessions pick up newly available models (for example `gpt-5.3`) without restart.
- **Model availability probing UX**: Model support probing now runs in parallel with bounded concurrency, starts automatically after provider connect/startup, and streams live progress (`start`/`progress`/`complete`) over WebSocket so the Providers page can render a progress bar.
- **Provider-scoped probing on connect**: Connecting a provider from the Providers UI now probes only that provider's models (instead of all providers), reducing noise and startup load when adding accounts one by one.
- **Configurable model ordering**: Added `chat.priority_models` in `moltis.toml` to pin preferred models at the top of model selectors without rebuilding. Runtime model selectors (`models.list`, chat model dropdown, Telegram `/model`) hide unsupported models, while Providers diagnostics continue to show full catalog entries (including unsupported flags).
- **Configurable provider offerings in UI**: Added `[providers] offered = [...]` allowlist in `moltis.toml` to control which providers are shown in onboarding/provider-picker UI. New config templates default this to `["openai", "github-copilot"]`; setting `offered = []` shows all known providers. Configured providers remain visible for management.

### Fixed

- **Web search DuckDuckGo fallback**: When no search API key (Brave or
  Perplexity) is configured, `web_search` now automatically falls back to
  DuckDuckGo HTML search instead of returning an error and forcing the LLM
  to ask the user about using the browser.

- **Web onboarding flash and redirect timing**: The web server now performs onboarding redirects before rendering the main app shell. When onboarding is incomplete, non-onboarding routes redirect directly to `/onboarding`; once onboarding is complete, `/onboarding` redirects back to `/`. The onboarding route now serves a dedicated onboarding HTML/JS entry instead of the full app bundle, preventing duplicate bootstrap/navigation flashes in Safari.
- **Local model cache path visibility**: Startup logs for local LLM providers now explicitly print the model cache directory and cached model IDs, making `MOLTIS_DATA_DIR` behavior easier to verify without noisy model-catalog output.
- **Kimi device-flow OAuth in web UI**: Kimi OAuth now uses provider-specific headers and prefers `verification_uri_complete` (or synthesizes `?user_code=` fallback) so mobile-device sign-in links no longer fail with missing `user_code`.
- **Kimi Code provider authentication compatibility**: `kimi-code` is now API-key-first in the web UI (`KIMI_API_KEY`, default base URL `https://api.moonshot.ai/v1`), while still honoring previously stored OAuth tokens for backward compatibility. Provider errors now include a targeted hint to switch to API-key auth when Kimi returns `access_terminated_error`.
- **Provider setup success feedback**: API-key provider setup now runs an immediate model probe after saving credentials. The onboarding and Providers modal only show success when at least one model validates, and otherwise display a validation failure message instead of a false-positive "configured" state.
- **Heartbeat/cron duplicate runs**: Skip heartbeat LLM turn when no prompt is
  configured, and fix duplicate cron job executions that could fire the same
  scheduled run twice.
- **Onboarding finish screen removed**: Onboarding now skips the final
  "congratulations" screen and redirects straight to the chat view.
- **User message footer leak**: Model name footer and timestamp are no longer
  incorrectly attached to user messages in the chat UI.
- **TTS counterpart auto-enable on STT save**: Saving an ElevenLabs or Google
  Cloud STT key now automatically enables the matching TTS provider, mirroring
  the onboarding voice-test behavior.
- **Voice-generating indicator removed**: The separate "voice generating"
  spinner during TTS playback has been removed in favor of the unified
  response indicator.
- **Config restart crash loop prevention**: The gateway now validates the
  configuration file before restarting, returning an error to the UI instead
  of entering a crash loop when the config is invalid.
- **Safari dev-mode cache busting**: Development mode now busts the Safari
  asset cache on reload, and fixes a missing border on detected-provider cards.

### Refactored

- **McpManager lock consolidation**: Replaced per-field `RwLock`s in
  `McpManager` with a single `RwLock<McpManagerInner>` to reduce lock
  contention and simplify state management.
- **GatewayState lock consolidation**: Replaced per-field `RwLock`s in
  `GatewayState` with a single `RwLock<GatewayInner>` for the same reasons.
- **Typed chat broadcast payloads**: Chat WebSocket broadcasts now use typed
  Rust structs instead of ad-hoc `serde_json::Value` maps.

### Documentation

- Expanded default `SOUL.md` with the full OpenClaw reference text for agent
  personality bootstrapping.

## [0.2.9] - 2026-02-08

### Added

- **Voice provider policy controls**: Added provider-list allowlists so config templates and runtime voice setup can explicitly limit shown/allowed TTS and STT providers.
- **Typed voice provider metadata**: Expanded voice provider metadata and preference handling to use typed flows across gateway and UI paths.

### Changed

- **Reply medium preference handling**: Chat now prefers the same reply medium when possible and falls back to text when a medium cannot be preserved.

### Fixed

- **Chat UI reply badge visibility**: Assistant footer now reliably shows the selected reply medium badge.
- **Voice UX polish**: Improved microphone timing behavior and preserved settings scroll state in voice configuration views.
## [0.2.8] - 2026-02-07

### Changed

- **Unified plugins and skills into a single system**: Plugins and skills were separate
  systems with duplicate code, manifests, and UI pages. They are now merged into one
  unified "Skills" system. All installed repos (SKILL.md, Claude Code `.claude-plugin/`,
  Codex) are managed through a single `skills-manifest.json` and `installed-skills/`
  directory. The `/plugins` page has been removed — everything is accessible from the
  `/skills` page. A one-time startup migration automatically moves data from the old
  plugins manifest and directory into the new unified location.
- **Default config template voice list narrowed**: New generated configs now include a
  `[voice]` section with provider-list allowlists limited to ElevenLabs for TTS and
  Mistral + ElevenLabs for STT.

### Fixed

- **Update checker repository configuration**: The update checker now reads
  `server.update_repository_url` from `moltis.toml`, defaults new configs to
  `https://github.com/moltis-org/moltis`, and treats an omitted/commented value
  as explicitly disabled.
- **Mistral and other providers rejecting requests with HTTP 422**: Session metadata fields
  (`created_at`, `model`, `provider`, `inputTokens`, `outputTokens`) were leaking into
  provider API request bodies. Mistral's strict validation rejected the extra `created_at`
  field. Replaced `Vec<serde_json::Value>` with a typed `ChatMessage` enum in the
  `LlmProvider` trait — metadata can no longer leak because the type only contains
  LLM-relevant fields (`role`, `content`, `tool_calls`). Conversion from persisted JSON
  happens once at the gateway boundary via `values_to_chat_messages()`.
- **Chat skill creation not persisting new skills**: Runtime tool filtering incorrectly
  applied the union of discovered skill `allowed_tools` to all chat turns, which could
  hide `create_skill`/`update_skill` and leave only a subset (for example `web_fetch`).
  Chat runs now use configured tool policy for runtime filtering without globally
  restricting tools based on discovered skill metadata.

### Added

- **Voice Provider Management UI**: Configure TTS and STT providers from Settings > Voice
  - Auto-detection of API keys from environment variables and LLM provider configs
  - Toggle switches to enable/disable providers without removing configuration
  - Local binary detection for whisper.cpp, piper, and sherpa-onnx
  - Server availability checks for Coqui TTS and Voxtral Local
  - Setup instructions modal for local provider installation
  - Shared Google Cloud API key between TTS and STT
- **Voice provider UI allowlists**: Added `voice.tts.providers` and `voice.stt.providers`
  config lists to control which TTS/STT providers are shown in the Settings UI.
  Empty lists keep current behavior and show all providers.

- **New TTS Providers**:
  - Google Cloud Text-to-Speech (380+ voices, 50+ languages)
  - Piper (fast local neural TTS, runs offline)
  - Coqui TTS (high-quality neural TTS with voice cloning)

- **New STT Providers**:
  - ElevenLabs Scribe (90+ languages, word timestamps, speaker diarization)
  - Mistral AI Voxtral (cloud-based, 13 languages)
  - Voxtral Local via vLLM (self-hosted with OpenAI-compatible API)

- **Browser Sandbox Mode**: Run browser in isolated Docker containers for security
  - Automatic container lifecycle management
  - Uses `browserless/chrome` image by default (configurable via `sandbox_image`)
  - Container readiness detection via HTTP endpoint probing
  - Browser sandbox mode automatically follows the session's sandbox mode
    (no separate `browser.sandbox` config - sandboxed sessions use sandboxed browser)

- **Memory-Based Browser Pool Limits**: Browser instances now limited by system memory
  - `max_instances = 0` (default) allows unlimited instances, limited only by memory
  - `memory_limit_percent = 90` blocks new instances when system memory exceeds threshold
  - Idle browsers cleaned up automatically before blocking
  - Set `max_instances > 0` for hard limit if preferred

- **Automatic Browser Session Tracking**: Browser tool automatically reuses sessions
  - Session ID is tracked internally and injected when LLM doesn't provide one
  - Prevents pool exhaustion from LLMs forgetting to pass session_id
  - Session cleared on explicit "close" action

- **HiDPI Screenshot Support**: Screenshots scale correctly on Retina displays
  - `device_scale_factor` config (default: 2.0) for high-DPI rendering
  - Screenshot display in UI scales according to device pixel ratio
  - Viewport increased to 2560×1440 for sharper captures

- **Enhanced Screenshot Lightbox**:
  - Scrollable container for viewing long/tall screenshots
  - Download button at top of lightbox
  - Visible ✕ close button instead of text hint
  - Proper scaling for HiDPI displays

- **Telegram Screenshot Support**: Browser screenshots sent to Telegram channels
  - Automatic retry as document when image dimensions exceed Telegram limits
  - Error messages sent to channel when screenshot delivery fails
  - Handles `PHOTO_INVALID_DIMENSIONS` and `PHOTO_SAVE_FILE_INVALID` errors

- **Telegram Tool Status Notifications**: See what's happening during long operations
  - Tool execution messages sent to Telegram (e.g., "🌐 Navigating to...",
    "💻 Running: `git status`", "📸 Taking screenshot...")
  - Messages sent silently (no notification sound) to avoid spam
  - Typing indicator automatically re-sent after status messages
  - Supports browser, exec, web_fetch, web_search, and memory tools

- **Log Target Display**: Logs now include the crate/module path for easier debugging
  - Example: `INFO moltis_gateway::chat: tool execution succeeded tool=browser`

- **Contributor docs: local validation**: Added documentation for the `./scripts/local-validate.sh` workflow, including published local status contexts, platform behavior, and CI fallback expectations.
- **Hooks Web UI**: New `/hooks` page to manage lifecycle hooks from the browser
  - View all discovered hooks with eligibility status, source, and events
  - Enable/disable hooks without removing files (persisted across restarts)
  - Edit HOOK.md content in a monospace textarea and save back to disk
  - Reload hooks at runtime to pick up changes without restarting
  - Live stats (call count, failures, avg latency) from the hook registry
  - WebSocket-driven auto-refresh via `hooks.status` event
  - RPC methods: `hooks.list`, `hooks.enable`, `hooks.disable`, `hooks.save`, `hooks.reload`
- **Deploy platform detection**: New `MOLTIS_DEPLOY_PLATFORM` env var hides local-only providers (local-llm, Ollama) on cloud deployments. Pre-configured in Fly.io, DigitalOcean, and Render deploy templates.
- **Telegram OTP self-approval**: Non-allowlisted DM users receive a 6-digit verification code instead of being silently ignored. Correct code entry auto-approves the user to the allowlist. Includes flood protection (non-code messages silently ignored), lockout after 3 failed attempts (configurable cooldown), and 5-minute code expiry. OTP codes visible in web UI Senders tab. Controlled by `otp_self_approval` (default: true) and `otp_cooldown_secs` (default: 300) config fields.
- **Update availability banner**: The web UI now checks GitHub releases hourly and shows a top banner when a newer version of moltis is available, with a direct link to the release page.

### Changed

- **Documentation safety notice**: Added an upfront alpha-software warning on the docs landing page, emphasizing careful deployment, isolation, and strong auth/network controls for self-hosted AI assistants.
- **Release packaging**: Derive release artifact versions from the Git tag (`vX.Y.Z`) in CI, and sync package metadata during release jobs to prevent filename/version drift.
- **Versioning**: Bump workspace and snap baseline version to `0.2.0`.
- **Onboarding auth flow**: Route first-run setup directly into `/onboarding` and remove the separate `/setup` web UI page.
- **Startup observability**: Log each loaded context markdown (`CLAUDE.md` / `AGENTS.md` / `.claude/rules/*.md`), memory markdown (`MEMORY.md` and `memory/*.md`), and discovered `SKILL.md` to make startup/context loading easier to audit.
- **Workspace root pathing**: Standardize workspace-scoped file discovery/loading on `moltis_config::data_dir()` instead of process cwd (affects BOOT.md, hook discovery, skill discovery, and compaction memory output paths).
- **Soul storage**: Move agent personality text out of `moltis.toml` into workspace `SOUL.md`; identity APIs/UI still edit soul, but now persist it as a markdown file.
- **Identity storage**: Persist agent identity fields (`name`, `emoji`, `creature`, `vibe`) to workspace `IDENTITY.md` using YAML frontmatter; settings UI continues to edit these fields through the same RPC/API.
- **User profile storage**: Persist user profile fields (`name`, `timezone`) to workspace `USER.md` using YAML frontmatter; onboarding/settings continue to use the same API/UI while reading/writing the markdown file.
- **Workspace markdown support**: Add `TOOLS.md` prompt injection from workspace root (`data_dir`), and keep startup injection sourced from `BOOT.md`.
- **Heartbeat prompt precedence**: Support workspace `HEARTBEAT.md` as heartbeat prompt source with precedence `heartbeat.prompt` (config override) → `HEARTBEAT.md` → built-in default; log when config prompt overrides `HEARTBEAT.md`.
- **Heartbeat UX**: Expose effective heartbeat prompt source (`config`, `HEARTBEAT.md`, or default) via `heartbeat.status` and display it in the Heartbeat settings UI.
- **BOOT.md onboarding aid**: Seed a default workspace `BOOT.md` with in-file guidance describing startup injection behavior and recommended usage.
- **Workspace context parity**: Treat workspace `TOOLS.md` as general context (not only policy) and add workspace `AGENTS.md` injection support from `data_dir`.
- **Heartbeat token guard**: Skip heartbeat LLM turns when `HEARTBEAT.md` exists but is empty/comment-only and there is no explicit `heartbeat.prompt` override, reducing unnecessary token consumption.
- **Exec approval policy wiring**: Gateway now initializes exec approval mode/security level/allowlist from `moltis.toml` (`tools.exec.*`) instead of always using hardcoded defaults.
- **Runtime tool enforcement**: Chat runs now apply configured tool policy (`tools.policy`) and skill `allowed_tools` constraints when selecting callable tools.
- **Skill trust lifecycle**: Installed marketplace skills/plugins now track a `trusted` state and must be trusted before they can be enabled; the skills UI now surfaces untrusted status and supports trust-before-enable.
- **Git metadata via gitoxide**: Gateway now resolves branch names, repo HEAD SHAs, and commit timestamps using `gix` (gitoxide) instead of shelling out to `git` for those read-only operations.

### Fixed

- **OAuth callback on hosted deployments**: OpenAI Codex OAuth now uses the web app origin callback (`/auth/callback`) in the UI flow instead of hardcoded localhost loopback, allowing DigitalOcean/Fly/Render deployments to complete OAuth successfully.
- **Sandbox startup on hosted Docker environments**: Skip sandbox image pre-build when sandbox mode is off, and require Docker daemon accessibility (not just Docker CLI presence) before selecting the Docker sandbox backend.
- **Homebrew release automation**: Run the tap update in the release workflow after all package/image jobs complete so formula publishing does not race missing tarball assets.
- **Docker runtime**: Install `libgomp1` in the runtime image to satisfy OpenMP-linked binaries and prevent startup failures with `libgomp.so.1` missing.
- **Release CI validation**: Add a Docker smoke test step (`moltis --help`) after image build/push so missing runtime libraries fail in CI before release.
- **Web onboarding clarity**: Add setup-code guidance that points users to the process log (stdout).
- **WebSocket auth (remote deployments)**: Accept existing session/API-key auth from WebSocket upgrade headers so browser connections don't immediately close after `connect` on hosted setups.
- **Sandbox UX on unsupported hosts**: Disable sandbox controls in chat/images when no runtime backend is detected, with a tooltip explaining cloud deploy limitations.
- **Telegram OTP code echoed to LLM**: After OTP self-approval, the verification code message was re-processed as a regular chat message because `sender_approve` restarted the bot polling loop (resetting the Telegram update offset). Sender approve/deny now hot-update the in-memory config without restarting the bot.
- **Empty allowlist bypassed access control**: When `dm_policy = Allowlist` and all entries were removed, the empty list was treated as "allow everyone" instead of "deny everyone". An explicit Allowlist policy with an empty list now correctly denies all access.
- **Browser sandbox timeout**: Sandboxed browsers now use the configured
  `navigation_timeout_ms` (default 30s) instead of a shorter internal timeout.
  Previously, sandboxed browser connections could time out prematurely.
- **Tall screenshot lightbox**: Full-page screenshots now display at proper size
  with vertical scrolling instead of being scaled down to fit the viewport.
- **Telegram typing indicator for long responses**: Channel replies now wait for outbound delivery tasks to finish before chat completion returns, so periodic `typing...` updates continue until the Telegram message is actually sent.
- **Skills dependency install safety**: `skills.install_dep` now requires explicit user confirmation and blocks host installs when sandbox mode is disabled (unless explicitly overridden in the RPC call).

### Security

- **Asset response hardening**: Static assets now set `X-Content-Type-Options: nosniff`, and SVG responses include a restrictive `Content-Security-Policy` (`script-src 'none'`, `object-src 'none'`) to reduce stored-XSS risk if user-controlled SVGs are ever introduced.
- **Archive extraction hardening**: Skills/plugin tarball installs now reject unsafe archive paths (`..`, absolute/path-prefix escapes) and reject symlink/hardlink archive entries to prevent path traversal and link-based escapes.
- **Install provenance**: Installed skill/plugin repo manifests now persist a pinned `commit_sha` (resolved from clone or API fallback) for future trust drift detection.
- **Re-trust on source drift**: If an installed git-backed repo's HEAD commit changes from the pinned `commit_sha`, the gateway now marks its skills untrusted+disabled and requires trust again before re-enabling; the UI surfaces this as `source changed`.
- **Security audit trail**: Skill/plugin install, remove, trust, enable/disable, dependency install, and source-drift events are now appended to `~/.moltis/logs/security-audit.jsonl` for incident review.
- **Emergency kill switch**: Added `skills.emergency_disable` to immediately disable all installed third-party skills and plugins; exposed in the Skills UI as a one-click emergency action.
- **Risky dependency install blocking**: `skills.install_dep` now blocks suspicious install command patterns by default (e.g. piped shell payloads, base64 decode chains, quarantine bypass) unless explicitly overridden with `allow_risky_install=true`.
- **Provenance visibility**: Skills UI now displays pinned install commit SHA in repo and detail views to make source provenance easier to verify.
- **Recent-commit risk warnings**: Skill/plugin detail views now include commit links and commit-age indicators, with a prominent warning banner when the pinned commit is very recent.
- **Installer subprocess reduction**: Skills/plugins install paths now avoid `git` subprocess clone attempts and use GitHub tarball installs with pinned commit metadata.
- **Install resilience for rapid multi-repo installs**: Skills/plugins install now auto-clean stale on-disk directories that are missing from manifest state, and tar extraction skips link entries instead of failing the whole install.
- **Orphaned repo visibility**: Skills/plugins repo listing now surfaces manifest-missing directories found on disk as `orphaned` entries and allows removing them from the UI.
- **Protected seed skills**: Discovered template skills (`template-skill` / `template`) are now marked protected and cannot be deleted from the web UI.
- **License review links**: Skill/plugin license badges now link directly to repository license files when detectable (e.g. `LICENSE.txt`, `LICENSE.md`, `LICENSE`).
- **Example skill seeding**: Gateway now seeds `~/.moltis/skills/template-skill/SKILL.md` on startup when missing, so users always have a starter personal skill template.
- **Memory indexing scope tightened**: Memory sync now indexes only `MEMORY.md` / `memory.md` and `memory/` content by default (instead of scanning the entire data root), reducing irrelevant indexing noise from installed skills/plugins.
- **Ollama embedding bootstrap**: When using Ollama for memory embeddings, gateway now auto-attempts to pull missing embedding models (default `nomic-embed-text`) via Ollama HTTP API.

### Documentation

- Added `docs/src/skills-security.md` with third-party skills/plugin hardening guidance (trust lifecycle, provenance pinning, source-drift re-trust, risky install guards, emergency disable, and security audit logging).

## [0.1.10] - 2026-02-06

### Changed

- **CI builds**: Build Docker images natively per architecture instead of QEMU emulation, then merge into multi-arch manifest

## [0.1.9] - 2026-02-06

### Changed

- **CI builds**: Migrate all release build jobs from self-hosted to GitHub-hosted runners for full parallelism (`ubuntu-latest`, `ubuntu-latest-arm`, `macos-latest`), remove all cross-compilation toolchain steps

## [0.1.8] - 2026-02-06

### Fixed

- **CI builds**: Fix corrupted cargo config on all self-hosted runner jobs, fix macOS runner label, add llama-cpp build deps to Docker and Snap builds

## [0.1.7] - 2026-02-06

### Fixed

- **CI builds**: Use project-local `.cargo/config.toml` for cross-compilation instead of appending to global config (fixes duplicate key errors on self-hosted runners)

## [0.1.6] - 2026-02-06

### Fixed

- **CI builds**: Use macOS GitHub-hosted runners for apple-darwin binary builds instead of cross-compiling from Linux
- **CI performance**: Run lightweight lint jobs (zizmor, biome, fmt) on GitHub-hosted runners to free up self-hosted runners

## [0.1.5] - 2026-02-06

### Fixed

- **CI security**: Use GitHub-hosted runners for PRs to prevent untrusted code from running on self-hosted infrastructure
- **CI security**: Add `persist-credentials: false` to docs workflow checkout (fixes zizmor artipacked warning)

## [0.1.4] - 2026-02-06

### Added

- **`--no-tls` CLI flag**: `--no-tls` flag and `MOLTIS_NO_TLS` environment variable to disable
  TLS for cloud deployments where the provider handles TLS termination
- **One-click cloud deploy**: Deploy configs for Fly.io (`fly.toml`), DigitalOcean
  (`.do/deploy.template.yaml`), Render (`render.yaml`), and Railway (`railway.json`)
  with deploy buttons in the README
- **Config Check Command**: `moltis config check` validates the configuration file, detects unknown/misspelled fields with Levenshtein-based suggestions, warns about security misconfigurations, and checks file references

- **Memory Usage Indicator**: Display process RSS and system free memory in the header bar, updated every 30 seconds via the tick WebSocket broadcast

- **QMD Backend Support**: Optional QMD (Query Memory Daemon) backend for hybrid search with BM25 + vector + LLM reranking
  - Gated behind `qmd` feature flag (enabled by default)
  - Web UI shows installation instructions and QMD status
  - Comparison table between built-in SQLite and QMD backends
- **Citations**: Configurable citation mode (on/off/auto) for memory search results
  - Auto mode includes citations when results span multiple files
- **Session Export**: Option to export session transcripts to memory for future reference
- **LLM Reranking**: Use LLM to rerank search results for improved relevance (requires QMD)
- **Memory Documentation**: Added `docs/src/memory.md` with comprehensive memory system documentation

- **Mobile PWA Support**: Install moltis as a Progressive Web App on iOS, Android, and desktop
  - Standalone mode with full-screen experience
  - Custom app icon (crab mascot)
  - Service worker for offline support and caching
  - Safe area support for notched devices

- **Push Notifications**: Receive alerts when the LLM responds
  - VAPID key generation and storage for Web Push API
  - Subscribe/unsubscribe toggle in Settings > Notifications
  - Subscription management UI showing device name, IP address, and date
  - Remove any subscription from any device
  - Real-time subscription updates via WebSocket
  - Client IP detection from X-Forwarded-For, X-Real-IP, CF-Connecting-IP headers
  - Notifications sent for both streaming and agent (tool-using) chat modes

- **Safari/iOS PWA Detection**: Show "Add to Dock" instructions when push notifications
  require PWA installation (Safari doesn't support push in browser mode)

- **Browser Screenshot Thumbnails**: Screenshots from the browser tool now display as
  clickable thumbnails in the chat UI
  - Click to view fullscreen in a lightbox overlay
  - Press Escape or click anywhere to close
  - Thumbnails are 200×150px max with hover effects

- **Improved Browser Detection**: Better cross-platform browser detection
  - Checks macOS app bundles before PATH (avoids broken Homebrew chromium wrapper)
  - Supports Chrome, Chromium, Edge, Brave, Opera, Vivaldi, Arc
  - Shows platform-specific installation instructions when no browser found
  - Custom path via `chrome_path` config or `CHROME` environment variable

- **Vision Support for Screenshots**: Vision-capable models can now interpret
  browser screenshots instead of having them stripped from context
  - Screenshots sent as multimodal image content blocks for GPT-4o, Claude, Gemini
  - Non-vision models continue to receive `[base64 data removed]` placeholder
  - `supports_vision()` trait method added to `LlmProvider` for capability detection

- **Session state store**: per-session key-value persistence scoped by
  namespace, backed by SQLite (`session_state` tool).
- **Session branching**: `branch_session` tool forks a conversation at any
  message index into an independent copy.
- **Session fork from UI**: Fork button in the chat header and sidebar action
  buttons let users fork sessions without asking the LLM. Forked sessions
  appear indented under their parent with a branch icon.
- **Skill self-extension**: `create_skill`, `update_skill`, `delete_skill`
  tools let the agent manage project-local skills at runtime.
- **Skill hot-reload**: filesystem watcher on skill directories emits
  `skills.changed` events via WebSocket when SKILL.md files change.
- **Typed tool sources**: `ToolSource` enum (`Builtin` / `Mcp { server }`)
  replaces string-prefix identification of MCP tools in the tool registry.
- **Tool registry metadata**: `list_schemas()` now includes `source` and
  `mcpServer` fields so the UI can group tools by origin.
- **Per-session MCP toggle**: sessions store an `mcp_disabled` flag; the chat
  header exposes a toggle button to enable/disable MCP tools per session.
- **Debug panel convergence**: the debug side-panel now renders the same seven
  sections as the `/context` slash command, eliminating duplicated rendering
  logic.
- Documentation pages for session state, session branching, skill
  self-extension, and the tool registry architecture.
### Changed

- Memory settings UI enhanced with backend comparison and feature explanations
- Added `memory.qmd.status` RPC method for checking QMD availability
- Extended `memory.config.get` to include `qmd_feature_enabled` flag

- Push notifications feature is now enabled by default in the CLI

- **TLS HTTP redirect port** now defaults to `gateway_port + 1` instead of
  the hardcoded port `18790`. This makes the Dockerfile simpler (both ports
  are adjacent) and avoids collisions when running multiple instances.
  Override via `[tls] http_redirect_port` in `moltis.toml` or the
  `MOLTIS_TLS__HTTP_REDIRECT_PORT` environment variable.

- **TLS certificates use `moltis.localhost` domain.** Auto-generated server
  certs now include `moltis.localhost`, `*.moltis.localhost`, `localhost`,
  `127.0.0.1`, and `::1` as SANs. Banner and redirect URLs use
  `https://moltis.localhost:<port>` when bound to loopback, so the cert
  matches the displayed URL. Existing certs are automatically regenerated
  on next startup.

- **Certificate validity uses dynamic dates.** Cert `notBefore`/`notAfter`
  are now computed from the current system time instead of being hardcoded.
  CA certs are valid for 10 years, server certs for 1 year from generation.

- `McpToolBridge` now stores and exposes `server_name()` for typed
  registration.
- `mcp_service::sync_mcp_tools()` uses `unregister_mcp()` /
  `register_mcp()` instead of scanning tool names by prefix.
- `chat.rs` uses `clone_without_mcp()` instead of
  `clone_without_prefix("mcp__")` in all three call sites.

### Fixed

- Push notifications not sending when chat uses agent mode (run_with_tools)
- Missing space in Safari install instructions ("usingFile" → "using File")
- **WebSocket origin validation** now treats `.localhost` subdomains
  (e.g. `moltis.localhost`) as loopback equivalents per RFC 6761.
- **Browser tool schema enforcement**: Added `strict: true` and `additionalProperties: false`
  to OpenAI-compatible tool schemas, improving model compliance with required fields
- **Browser tool defaults**: When model sends URL without action, defaults to `navigate`
  instead of erroring
- **Chat message ordering**: Fixed interleaving of text and tool cards when streaming;
  messages now appear in correct chronological order
- **Tool passthrough in ProviderChain**: Fixed tools not being passed to fallback
  providers when using provider chains
- Fork/branch icon in session sidebar now renders cleanly at 16px (replaced
  complex git-branch SVG with simple trunk+branch path).
- Deleting a forked session now navigates to the parent session instead of
  an unrelated sibling.
- **Streaming tool calls for non-Anthropic providers**: `OpenAiProvider`,
  `GitHubCopilotProvider`, `KimiCodeProvider`, `OpenAiCodexProvider`, and
  `ProviderChain` now implement `stream_with_tools()` so tool schemas are
  sent in the streaming API request and tool-call events are properly parsed.
  Previously only `AnthropicProvider` supported streaming tool calls; all
  other providers silently dropped the tools parameter, causing the LLM to
  emit tool invocations as plain text instead of structured function calls.
- **Streaming tool call arguments dropped when index ≠ 0**: When a provider
  (e.g. GitHub Copilot proxying Claude) emits a text content block at
  streaming index 0 and a tool_use block at index 1, the runner's argument
  finalization used the streaming index as the vector position directly.
  Since `tool_calls` has only 1 element at position 0, the condition
  `1 < 1` was false and arguments were silently dropped (empty `{}`).
  Fixed by mapping streaming indices to vector positions via a HashMap.
- **Skill tools wrote to wrong directory**: `create_skill`, `update_skill`, and
  `delete_skill` used `std::env::current_dir()` captured at gateway startup,
  writing skills to `<cwd>/.moltis/skills/` instead of `~/.moltis/skills/`.
  Skills now write to `<data_dir>/skills/` (Personal source), which is always
  discovered regardless of where the gateway was started.
- **Skills page missing personal/project skills**: The `/api/skills` endpoint
  only returned manifest-based registry skills. Personal and project-local
  skills were never shown in the navigation or skills page. The endpoint now
  discovers and includes them alongside registry skills.

### Documentation

- Added voice.md with TTS/STT provider documentation and setup guides
- Added mobile-pwa.md with PWA installation and push notification documentation
- Updated CLAUDE.md with cargo feature policy (features enabled by default)
- Updated browser-automation.md with browser detection, screenshot display, and
  model error handling sections
- Rewrote session-branching.md with accurate fork details, UI methods, RPC
  API, inheritance table, and deletion behavior.
