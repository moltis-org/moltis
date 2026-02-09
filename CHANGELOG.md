# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
