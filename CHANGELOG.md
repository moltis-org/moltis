# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

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

### Changed

- **Release packaging**: Derive release artifact versions from the Git tag (`vX.Y.Z`) in CI, and sync package metadata during release jobs to prevent filename/version drift.
- **Versioning**: Bump workspace and snap baseline version to `0.2.0`.
- **Onboarding auth flow**: Route first-run setup directly into `/onboarding` and remove the separate `/setup` web UI page.
- **Startup observability**: Log each loaded context markdown (`CLAUDE.md` / `AGENTS.md` / `.claude/rules/*.md`), memory markdown (`MEMORY.md` and `memory/*.md`), and discovered `SKILL.md` to make startup/context loading easier to audit.
- **Workspace root pathing**: Standardize workspace-scoped file discovery/loading on `moltis_config::data_dir()` instead of process cwd (affects BOOT.md, hook discovery, skill discovery, and compaction memory output paths).
- **Soul storage**: Move agent personality text out of `moltis.toml` into workspace `SOUL.md`; identity APIs/UI still edit soul, but now persist it as a markdown file.
- **Identity storage**: Persist agent identity fields (`name`, `emoji`, `creature`, `vibe`) to workspace `IDENTITY.md` using YAML frontmatter; settings UI continues to edit these fields through the same RPC/API.
- **User profile storage**: Persist user profile fields (`name`, `timezone`) to workspace `USER.md` using YAML frontmatter; onboarding/settings continue to use the same API/UI while reading/writing the markdown file.
- **Workspace markdown support**: Add `TOOLS.md` prompt injection and `BOOTSTRAP.md` startup injection support from workspace root (`data_dir`), with built-in boot hook support for combining `BOOTSTRAP.md` + `BOOT.md`.
- **Heartbeat prompt precedence**: Support workspace `HEARTBEAT.md` as heartbeat prompt source with precedence `heartbeat.prompt` (config override) → `HEARTBEAT.md` → built-in default; log when config prompt overrides `HEARTBEAT.md`.
- **Heartbeat UX**: Expose effective heartbeat prompt source (`config`, `HEARTBEAT.md`, or default) via `heartbeat.status` and display it in the Heartbeat settings UI.

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
- **Telegram typing indicator for long responses**: Channel replies now wait for outbound delivery tasks to finish before chat completion returns, so periodic `typing...` updates continue until the Telegram message is actually sent.

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

- Added mobile-pwa.md with PWA installation and push notification documentation
- Updated CLAUDE.md with cargo feature policy (features enabled by default)
- Rewrote session-branching.md with accurate fork details, UI methods, RPC
  API, inheritance table, and deletion behavior.
