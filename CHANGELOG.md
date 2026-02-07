# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Fixed

- **Mistral and other providers rejecting requests with HTTP 422**: Session metadata fields
  (`created_at`, `model`, `provider`, `inputTokens`, `outputTokens`) were leaking into
  provider API request bodies. Mistral's strict validation rejected the extra `created_at`
  field. Replaced `Vec<serde_json::Value>` with a typed `ChatMessage` enum in the
  `LlmProvider` trait — metadata can no longer leak because the type only contains
  LLM-relevant fields (`role`, `content`, `tool_calls`). Conversion from persisted JSON
  happens once at the gateway boundary via `values_to_chat_messages()`.

### Added

- **Voice Provider Management UI**: Configure TTS and STT providers from Settings > Voice
  - Auto-detection of API keys from environment variables and LLM provider configs
  - Toggle switches to enable/disable providers without removing configuration
  - Local binary detection for whisper.cpp, piper, and sherpa-onnx
  - Server availability checks for Coqui TTS and Voxtral Local
  - Setup instructions modal for local provider installation
  - Shared Google Cloud API key between TTS and STT

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

### Changed

- **Documentation safety notice**: Added an upfront alpha-software warning on the docs landing page, emphasizing careful deployment, isolation, and strong auth/network controls for self-hosted AI assistants.
- **Release packaging**: Derive release artifact versions from the Git tag (`vX.Y.Z`) in CI, and sync package metadata during release jobs to prevent filename/version drift.
- **Versioning**: Bump workspace and snap baseline version to `0.2.0`.
- **Onboarding auth flow**: Route first-run setup directly into `/onboarding` and remove the separate `/setup` web UI page.
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
