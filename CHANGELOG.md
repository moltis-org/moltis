# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

### Changed

### Deprecated

### Removed

### Fixed

### Security

## [20260426.05] - 2026-04-26
### Added
- [cron] Add heartbeat wake cooldown to prevent exec re-fire loop (#871)
- [web-ui] Show chat status badges in visible toolbar row (#886)
- [local-llm] On-demand model loading/unloading with idle timeout (#884)


### Changed
- [config] Extract initialize_config() from discover_and_load()
- Derive nightly toolchain from rust-toolchain.toml everywhere


### Removed
- Remove needless return in skills toggle_bundled_skill


### Fixed
- [tests] Stabilize flaky memory_config_get test
- [ci] Pin NCCL version to match CUDA 12.4 container
- [ci] Match NCCL dev headers to pre-installed runtime version
- [ci] Skip libnccl-dev to work around llama-cpp-sys-2 linking bug
- [ci] Remove pre-installed libnccl-dev from CUDA container
- [providers] Link NCCL when llama-cpp-sys-2 compiles with GGML_USE_NCCL
- Resolve clippy lints for nightly-2026-04-24 and matrix-sdk compat
- [mcp] Prefer native MCP tools over mcporter (#874)
- [ci] Use keychain-profile for notarization and log failures
- [gateway] Handle bundled skill disable/enable via config (#877)
- [gateway] Derive bundled skill enabled state from config (#878)
- [tests] Increase watcher test timeout to accommodate debounce + FSEvents latency
- [web-ui] Stop auto-enabling all skills on repository import (#882)
- [skills] Store per-skill relative paths for marketplace repos (#883)
- [tests] Mark flaky FSEvents watcher test as ignored
- [tests] Use PollWatcher in watcher tests for reliable cross-platform behavior
- Use if-let instead of single-arm match in poll watcher test
- [security] Store voice API keys in credential store, not moltis.toml (#885)
- [e2e] Update session tests for inline action buttons (#886 follow-up)
- [tests] Add missing LocalLlmService trait methods to MockLocalLlm
- Collapse nested if-let in key_store timestamp check
- Derive Default for ModelLifecycleManager


### Security
- [ci] Use correct security list-keychains syntax for macOS codesign
- [ci] Align macOS certificate import with working arbor pattern
- [ci] Add --timestamp and strip get-task-allow for macOS notarization

## [20260426.04] - 2026-04-26
### Added
- [cron] Add heartbeat wake cooldown to prevent exec re-fire loop (#871)


### Changed
- [config] Extract initialize_config() from discover_and_load()
- Derive nightly toolchain from rust-toolchain.toml everywhere


### Fixed
- [tests] Stabilize flaky memory_config_get test
- [ci] Pin NCCL version to match CUDA 12.4 container
- [ci] Match NCCL dev headers to pre-installed runtime version
- [ci] Skip libnccl-dev to work around llama-cpp-sys-2 linking bug
- [ci] Remove pre-installed libnccl-dev from CUDA container
- [providers] Link NCCL when llama-cpp-sys-2 compiles with GGML_USE_NCCL
- Resolve clippy lints for nightly-2026-04-24 and matrix-sdk compat
- [mcp] Prefer native MCP tools over mcporter (#874)
- [ci] Use keychain-profile for notarization and log failures


### Security
- [ci] Use correct security list-keychains syntax for macOS codesign
- [ci] Align macOS certificate import with working arbor pattern
- [ci] Add --timestamp and strip get-task-allow for macOS notarization

## [20260426.03] - 2026-04-26
### Added
- [cron] Add heartbeat wake cooldown to prevent exec re-fire loop (#871)


### Changed
- [config] Extract initialize_config() from discover_and_load()
- Derive nightly toolchain from rust-toolchain.toml everywhere


### Fixed
- [tests] Stabilize flaky memory_config_get test
- [ci] Pin NCCL version to match CUDA 12.4 container
- [ci] Match NCCL dev headers to pre-installed runtime version
- [ci] Skip libnccl-dev to work around llama-cpp-sys-2 linking bug
- [ci] Remove pre-installed libnccl-dev from CUDA container
- [providers] Link NCCL when llama-cpp-sys-2 compiles with GGML_USE_NCCL
- Resolve clippy lints for nightly-2026-04-24 and matrix-sdk compat
- [mcp] Prefer native MCP tools over mcporter (#874)
- [ci] Use keychain-profile for notarization and log failures


### Security
- [ci] Use correct security list-keychains syntax for macOS codesign
- [ci] Align macOS certificate import with working arbor pattern

## [20260426.02] - 2026-04-26
### Added
- [cron] Add heartbeat wake cooldown to prevent exec re-fire loop (#871)


### Changed
- [config] Extract initialize_config() from discover_and_load()
- Derive nightly toolchain from rust-toolchain.toml everywhere


### Fixed
- [tests] Stabilize flaky memory_config_get test
- [ci] Pin NCCL version to match CUDA 12.4 container
- [ci] Match NCCL dev headers to pre-installed runtime version
- [ci] Skip libnccl-dev to work around llama-cpp-sys-2 linking bug
- [ci] Remove pre-installed libnccl-dev from CUDA container
- [providers] Link NCCL when llama-cpp-sys-2 compiles with GGML_USE_NCCL
- Resolve clippy lints for nightly-2026-04-24 and matrix-sdk compat
- [mcp] Prefer native MCP tools over mcporter (#874)
- [ci] Use keychain-profile for notarization and log failures


### Security
- [ci] Use correct security list-keychains syntax for macOS codesign
- [ci] Align macOS certificate import with working arbor pattern

## [20260426.01] - 2026-04-26
### Added
- [cron] Add heartbeat wake cooldown to prevent exec re-fire loop (#871)


### Changed
- [config] Extract initialize_config() from discover_and_load()
- Derive nightly toolchain from rust-toolchain.toml everywhere


### Fixed
- [tests] Stabilize flaky memory_config_get test
- [ci] Pin NCCL version to match CUDA 12.4 container
- [ci] Match NCCL dev headers to pre-installed runtime version
- [ci] Skip libnccl-dev to work around llama-cpp-sys-2 linking bug
- [ci] Remove pre-installed libnccl-dev from CUDA container
- [providers] Link NCCL when llama-cpp-sys-2 compiles with GGML_USE_NCCL


### Security
- [ci] Use correct security list-keychains syntax for macOS codesign
- [ci] Align macOS certificate import with working arbor pattern

## [20260425.09] - 2026-04-25
### Added
- [cron] Add heartbeat wake cooldown to prevent exec re-fire loop (#871)


### Changed
- [config] Extract initialize_config() from discover_and_load()


### Fixed
- [tests] Stabilize flaky memory_config_get test
- [ci] Pin NCCL version to match CUDA 12.4 container
- [ci] Match NCCL dev headers to pre-installed runtime version
- [ci] Skip libnccl-dev to work around llama-cpp-sys-2 linking bug
- [ci] Remove pre-installed libnccl-dev from CUDA container
- [providers] Link NCCL when llama-cpp-sys-2 compiles with GGML_USE_NCCL


### Security
- [ci] Use correct security list-keychains syntax for macOS codesign
- [ci] Align macOS certificate import with working arbor pattern

## [20260425.08] - 2026-04-25
### Added
- [cron] Add heartbeat wake cooldown to prevent exec re-fire loop (#871)


### Changed
- [config] Extract initialize_config() from discover_and_load()


### Fixed
- [tests] Stabilize flaky memory_config_get test
- [ci] Pin NCCL version to match CUDA 12.4 container
- [ci] Match NCCL dev headers to pre-installed runtime version
- [ci] Skip libnccl-dev to work around llama-cpp-sys-2 linking bug
- [ci] Remove pre-installed libnccl-dev from CUDA container
- [providers] Link NCCL when llama-cpp-sys-2 compiles with GGML_USE_NCCL

## [20260425.07] - 2026-04-25
### Added
- [cron] Add heartbeat wake cooldown to prevent exec re-fire loop (#871)


### Changed
- [config] Extract initialize_config() from discover_and_load()


### Fixed
- [tests] Stabilize flaky memory_config_get test
- [ci] Pin NCCL version to match CUDA 12.4 container
- [ci] Match NCCL dev headers to pre-installed runtime version

## [20260425.06] - 2026-04-25
### Added
- [cron] Add heartbeat wake cooldown to prevent exec re-fire loop (#871)


### Changed
- [config] Extract initialize_config() from discover_and_load()


### Fixed
- [tests] Stabilize flaky memory_config_get test
- [ci] Pin NCCL version to match CUDA 12.4 container

## [20260425.05] - 2026-04-25
### Added
- [cron] Add heartbeat wake cooldown to prevent exec re-fire loop (#871)


### Changed
- [config] Extract initialize_config() from discover_and_load()


### Fixed
- [tests] Stabilize flaky memory_config_get test

## [20260425.04] - 2026-04-25
### Changed
- [config] Extract initialize_config() from discover_and_load()


### Fixed
- [tests] Stabilize flaky memory_config_get test

## [20260425.03] - 2026-04-25
### Fixed
- [e2e] Stabilize flaky gemini, autoscroll, and settings-nav tests

## [20260425.02] - 2026-04-25
### Fixed
- [ci] Allow held NCCL packages to be upgraded

## [20260425.01] - 2026-04-25
### Fixed
- [ci] Add NCCL to build deps and fix flaky tests

## [20260424.09] - 2026-04-24
### Fixed
- Replace expect() with let-else in splitter tests
- [benchmarks] Update code_index bench to use free-function chunker API

## [20260424.08] - 2026-04-24

## [20260424.07] - 2026-04-24
### Fixed
- Replace unwrap() with expect() in splitter tests

## [20260424.06] - 2026-04-24
### Fixed
- Remove needless Ok wrapper in refresh_qmd_index

## [20260424.05] - 2026-04-24

## [20260424.04] - 2026-04-24
### Changed
- [splitter] Extract moltis-splitter crate for AST-aware chunking (#791)


### Fixed
- [ci] Prevent releases.json from updating before GitHub release exists
- Revert releases.json to latest published release (20260421.05)

## [20260424.03] - 2026-04-24
### Added
- [skills] Add bundled skill category management to onboarding and settings (#829)
- [signal] Add signal-cli channel (#841)
- [home-assistant] Native Home Assistant integration crate (#827)
- [web-ui] Smart auto-scroll for chat messages (#846)
- [projects] Add code_index_enabled toggle to project settings (#837)
- [web] Add push-to-talk, VAD continuous listening, and voice settings (#303)
- [config] Layered config with defaults.toml and override-only user config (#864)
- [discord] Channel name pattern filtering and per-pattern overrides (#865)
- [config] Add default sub-agent presets (#844)
- [config] Add config compact command and auto-compact on startup
- [skills] MCP server management skill and post-install recipes (#840)


### Changed
- [providers] Split openai_compat tests into submodules
- [config] Split loader tests into submodules (file-size limit)
- [gateway] Extract NoopSkillsService into services/skills.rs (file-size limit)


### Removed
- Remove set -e from codesign-debug recipe that breaks just test
- [web] Remove stale accountId assertion from signal channel E2E test
- [ui] Wire up project combo dropdown in chat header (#847)
- [config] Remove unused imports from test submodule


### Fixed
- [tools] Block exec approval bypass via env-var prefix injection (#822)
- [browser] Add diagnostic logging for container readiness failures (#820)
- [gateway] Downgrade broadcast log from debug to trace (#830)
- [chat] Correct push notification click-through URL (#831)
- [tools] Skip sysfs tmpfs mounts on WSL2 (#835)
- [chat] Preserve Gemini tool call metadata (#836)
- [providers] Normalize non-strict tool schema unions (#833)
- [providers] Apply Kimi router overrides in Fireworks integration tests (#832)
- [config] Resolve ${VAR} placeholders against [env] section and DB env vars (#834)
- [web] Repair gemini tool-signature and reasoning-toggle E2E tests
- [ci] Move release test job to self-hosted runner
- [skills] Repair embedded bundled skill discovery in release builds
- [mcp] Show re-auth button when OAuth server needs re-authentication (#852)
- [sandbox] Skip sysfs tmpfs mounts for missing paths (ARM/Raspberry Pi) (#853)
- [prompt] Move datetime from system message to user content for KV cache stability (#855)
- [providers] Deep-merge properties in schema union collapse, strip redundant boolean enum (#856)
- [channels] Update ALL variant count for Signal channel type
- [web] Align signal channel E2E test with actual modal text
- [web] Stabilize flaky E2E tests
- Enforce correct date in silent memory turn daily log filenames (#859)
- [macos] Configure release signing and notarization (#842)
- [web] Add vault-sealed banner to main app (#839)
- [skills] Materialize bundled skill scripts to disk for execution (#861)
- [web] Biome formatting and i18n parity for projects page
- [config] Derive Default for HomeAssistantConfig and collapse nested if
- [config] Allow unwrap in code_index tests
- [home-assistant] Clippy collapsible-if and field-reassign-with-default
- [benchmarks] Add missing deps for code_index bench
- [code-index] Replace redundant closure with PathBuf::from
- Repair code-index compilation errors in gateway and benchmarks
- Resolve remaining clippy errors from code-index feature
- [providers] Strip null from enum arrays for Fireworks AI (#862)
- [web] Repair chat-autoscroll and projects E2E tests
- [benchmarks] Wire qmd feature to moltis-qmd dep for --all-features CI
- [ci] Explicitly set draft: false on release upload to prevent orphaned drafts
- [agents] Add missing mode field to PromptRuntimeContext in tests
- [gateway] Collapse nested if for clippy
- [ctl] Clippy unnecessary_lazy_evaluations
- [graphql] Add missing SkillsService trait methods to MockSkills
- [gateway] Collapse nested if for clippy

## [20260424.02] - 2026-04-24
### Added
- [skills] Add bundled skill category management to onboarding and settings (#829)
- [signal] Add signal-cli channel (#841)
- [home-assistant] Native Home Assistant integration crate (#827)
- [web-ui] Smart auto-scroll for chat messages (#846)
- [projects] Add code_index_enabled toggle to project settings (#837)


### Changed
- [providers] Split openai_compat tests into submodules


### Removed
- Remove set -e from codesign-debug recipe that breaks just test
- [web] Remove stale accountId assertion from signal channel E2E test
- [ui] Wire up project combo dropdown in chat header (#847)


### Fixed
- [tools] Block exec approval bypass via env-var prefix injection (#822)
- [browser] Add diagnostic logging for container readiness failures (#820)
- [gateway] Downgrade broadcast log from debug to trace (#830)
- [chat] Correct push notification click-through URL (#831)
- [tools] Skip sysfs tmpfs mounts on WSL2 (#835)
- [chat] Preserve Gemini tool call metadata (#836)
- [providers] Normalize non-strict tool schema unions (#833)
- [providers] Apply Kimi router overrides in Fireworks integration tests (#832)
- [config] Resolve ${VAR} placeholders against [env] section and DB env vars (#834)
- [web] Repair gemini tool-signature and reasoning-toggle E2E tests
- [ci] Move release test job to self-hosted runner
- [skills] Repair embedded bundled skill discovery in release builds
- [mcp] Show re-auth button when OAuth server needs re-authentication (#852)
- [sandbox] Skip sysfs tmpfs mounts for missing paths (ARM/Raspberry Pi) (#853)
- [prompt] Move datetime from system message to user content for KV cache stability (#855)
- [providers] Deep-merge properties in schema union collapse, strip redundant boolean enum (#856)
- [channels] Update ALL variant count for Signal channel type
- [web] Align signal channel E2E test with actual modal text
- [web] Stabilize flaky E2E tests
- Enforce correct date in silent memory turn daily log filenames (#859)
- [macos] Configure release signing and notarization (#842)
- [web] Add vault-sealed banner to main app (#839)
- [skills] Materialize bundled skill scripts to disk for execution (#861)
- [web] Biome formatting and i18n parity for projects page
- [config] Derive Default for HomeAssistantConfig and collapse nested if
- [config] Allow unwrap in code_index tests
- [home-assistant] Clippy collapsible-if and field-reassign-with-default
- [benchmarks] Add missing deps for code_index bench
- [code-index] Replace redundant closure with PathBuf::from
- Repair code-index compilation errors in gateway and benchmarks
- Resolve remaining clippy errors from code-index feature
- [providers] Strip null from enum arrays for Fireworks AI (#862)
- [web] Repair chat-autoscroll and projects E2E tests
- [benchmarks] Wire qmd feature to moltis-qmd dep for --all-features CI

## [20260424.01] - 2026-04-24
### Added
- [skills] Add bundled skill category management to onboarding and settings (#829)
- [signal] Add signal-cli channel (#841)


### Changed
- [providers] Split openai_compat tests into submodules


### Removed
- Remove set -e from codesign-debug recipe that breaks just test
- [web] Remove stale accountId assertion from signal channel E2E test


### Fixed
- [tools] Block exec approval bypass via env-var prefix injection (#822)
- [browser] Add diagnostic logging for container readiness failures (#820)
- [gateway] Downgrade broadcast log from debug to trace (#830)
- [chat] Correct push notification click-through URL (#831)
- [tools] Skip sysfs tmpfs mounts on WSL2 (#835)
- [chat] Preserve Gemini tool call metadata (#836)
- [providers] Normalize non-strict tool schema unions (#833)
- [providers] Apply Kimi router overrides in Fireworks integration tests (#832)
- [config] Resolve ${VAR} placeholders against [env] section and DB env vars (#834)
- [web] Repair gemini tool-signature and reasoning-toggle E2E tests
- [ci] Move release test job to self-hosted runner
- [skills] Repair embedded bundled skill discovery in release builds
- [mcp] Show re-auth button when OAuth server needs re-authentication (#852)
- [sandbox] Skip sysfs tmpfs mounts for missing paths (ARM/Raspberry Pi) (#853)
- [prompt] Move datetime from system message to user content for KV cache stability (#855)
- [providers] Deep-merge properties in schema union collapse, strip redundant boolean enum (#856)
- [channels] Update ALL variant count for Signal channel type

## [20260423.01] - 2026-04-23
### Added
- [skills] Add bundled skill category management to onboarding and settings (#829)


### Fixed
- [tools] Block exec approval bypass via env-var prefix injection (#822)
- [browser] Add diagnostic logging for container readiness failures (#820)
- [gateway] Downgrade broadcast log from debug to trace (#830)
- [chat] Correct push notification click-through URL (#831)
- [tools] Skip sysfs tmpfs mounts on WSL2 (#835)
- [chat] Preserve Gemini tool call metadata (#836)
- [providers] Normalize non-strict tool schema unions (#833)
- [providers] Apply Kimi router overrides in Fireworks integration tests (#832)
- [config] Resolve ${VAR} placeholders against [env] section and DB env vars (#834)
- [web] Repair gemini tool-signature and reasoning-toggle E2E tests
- [ci] Move release test job to self-hosted runner
- [skills] Repair embedded bundled skill discovery in release builds

## [20260422.01] - 2026-04-22
### Added
- [skills] Add bundled skill category management to onboarding and settings (#829)


### Fixed
- [tools] Block exec approval bypass via env-var prefix injection (#822)
- [browser] Add diagnostic logging for container readiness failures (#820)
- [gateway] Downgrade broadcast log from debug to trace (#830)
- [chat] Correct push notification click-through URL (#831)
- [tools] Skip sysfs tmpfs mounts on WSL2 (#835)
- [chat] Preserve Gemini tool call metadata (#836)
- [providers] Normalize non-strict tool schema unions (#833)
- [providers] Apply Kimi router overrides in Fireworks integration tests (#832)
- [config] Resolve ${VAR} placeholders against [env] section and DB env vars (#834)
- [web] Repair gemini tool-signature and reasoning-toggle E2E tests

## [20260421.05] - 2026-04-21
### Fixed
- [code-index] Use discover_opts with permissive trust for gix
- [web] Render inline markdown in table cells
- [code-index] Set required_trust to Reduced for gix discovery
- [code-index] Use struct init for clippy field_reassign_with_default
- [code-index] Use open_opts with forced Trust::Full for gix
- [code-index] Skip git-dependent tests when .git is absent
- [telegram] Infer MIME from filename when Telegram sends octet-stream (#819)
- [ci] Install git before checkout in CUDA containers

## [20260421.04] - 2026-04-21
### Fixed
- [ci] Make safe.directory step non-fatal when git is absent

## [20260421.03] - 2026-04-21
### Fixed
- [ci] Restore original step order, fix gix open, revert npm flag

## [20260421.02] - 2026-04-21
### Fixed
- [ci] Gix safe.directory, read_ops file_path regression, npm --ignore-scripts
- [tools] Empty file_path falls through to read_primary, whitespace rejected

## [20260421.01] - 2026-04-21
### Added
- [channels] Centralized command registry for all channels (#794)
- Gemini thought_signature round-tripping and schema validation fixes (#795)
- Self-improving agent loop — skills, memory lifecycle, deployment (#803)
- [web] Add GitHub Issues and Discussions links with count badges (#806)
- [sandbox] Install Node.js 22 LTS via NodeSource (#807)
- [web] Render markdown as HTML in chat messages (#808)
- [skills] Bundle 101 default skills with category UI and format fallback (#797)


### Changed
- [tools] Split skill_tools.rs into submodules


### Fixed
- [ci] Add safe.directory for container jobs (gix ownership check)
- [slack] Strip leading slash from commands before gateway dispatch (#804)
- [mcp] Implement legacy SSE transport for endpoint discovery (#805)
- [ci] Biome formatting and file size allowlist
- [sandbox] Verify image in Podman store after BuildKit build (#811)
- [providers] Disable strict tools for Fireworks Kimi router (#812)
- [ci] Npm --ignore-scripts and git install ordering
- [ci] Pass safe.directory to gix via env vars in rust-ci container

## [20260420.02] - 2026-04-20
### Fixed
- [ci] Address main rust failures

## [20260420.01] - 2026-04-20
### Fixed
- [ci] Handle .ts locale files in i18n-check on Node <22

## [20260419.01] - 2026-04-19
### Added
- [code-index] Add code indexing crate with builtin SQLite+FTS5 backend (#771)
- [config] Add server.external_url for reverse proxy WebAuthn (#785)


### Changed
- Add thiserror Error types to 8 library crates (#792)


### Fixed
- [task_list] List all tasks by default and add list_lists action (#779)
- [sandbox] Add observability and prevent repeated package provisioning (#784)

## [20260417.02] - 2026-04-17
### Added
- [nostr] Add NIP-59 Gift Wrap support for private DMs (#763)
- [matrix] Add OIDC authentication via Matrix Authentication Service (#730)


### Removed
- [tests] Remove hardcoded secret fixtures (#768)


### Fixed
- [providers] Strip $schema recursively and downgrade fallback log level (#762)
- [matrix] Retry sync loop on transient connection failures (#761)
- [e2e] Stabilize archived-sessions and nostr channel tests
- [sandbox] Skip sysfs tmpfs overlays on Podman (#765)
- [slack] Register /commands HTTP endpoint for slash commands (#767)
- [matrix] Restore main CI after OIDC changes

## [20260417.01] - 2026-04-17
### Added
- [web] Add reasoning effort toggle to chat toolbar (#750)


### Fixed
- [providers] Sanitize MCP tool schemas regardless of JSON Schema draft (#746)
- [node-host] Install rustls CryptoProvider before wss:// connections (#749)
- [providers] Prune orphaned required entries from tool schemas (#751)
- [web] Satisfy biome and format gates
- [testing] Avoid CUDA features in Darwin test recipes
- [web] Stabilize session delete flow
- [ci] Harden linux dependency installer

## [20260416.02] - 2026-04-16
### Added
- [providers] Configurable context windows + oldest-first compaction (#737)


### Changed
- [tests] Split oversized Rust test modules


### Fixed
- [ci] Repair MCP e2e and iOS chat event handling
- [providers] Detect Grok 3/4 as reasoning-capable models (#741)
- [nostr] Replace tokio RwLock with std RwLock to prevent panic (#742)
- [providers] Restore type annotations stripped by schema canonicalization (#740)
- [ci] Retry Linux dependency installs and align Darwin validation
- [ci] Install openssh client in Linux build container

## [20260415.01] - 2026-04-15
### Added
- Allow GUIDELINES.md file override for hardcoded tool guidelines
- [channels] Add WhatsApp to default offered channels
- [whatsapp] Auto-approve owner, set device name, wrap buttons
- [whatsapp] Handle PairSuccess event for instant QR dismissal
- [whatsapp] Make Account ID optional, default to "main"
- [web] Add Slack channel to onboarding wizard


### Changed
- Clean up provider tests and onboarding UI
- Move implementation code out of mod.rs and lib.rs (#731)


### Removed
- [web] Remove duplicate assets warn import


### Fixed
- [graphql] Make sessionKey required for all chat operations
- [graphql] Address PR review — harden subscription filter and tests
- [onboarding] Show version in server footer
- [web] Prefer embedded assets over stale share dir
- [chat] Preserve sender names in user messages
- [providers] Address sender-name review feedback
- Move return inside empty-guard in append_guidelines_section (PR 714 review)
- [providers] Skip strict tool schemas for Google/Gemini via OpenRouter (#716)
- [providers] Also detect googleapis.com URLs for non-strict tools
- [providers] Collapse type arrays in strict mode patching (#716)
- [whatsapp] Upgrade whatsapp-rust ecosystem 0.2 → 0.5
- [whatsapp] Use time crate for timestamps, fix DashMap sync key ordering
- [whatsapp] Add QR code polling fallback for pairing
- [whatsapp] Skip duplicate account start, add QR polling e2e test
- [whatsapp] Use res.payload not res.result for QR polling
- [whatsapp] Render QR code as SVG in polling response
- [whatsapp] Skip history sync on pairing
- [whatsapp] Detect pairing completion via polling
- [channels] Add missing useRef import in page-channels.js
- [whatsapp] Set OS name to Moltis, add no-sync hints
- [whatsapp] Fix mangled no-sync hints in onboarding view
- [whatsapp] Auto-approve both PN and LID JIDs, add debug logging
- [whatsapp] Fix QR code not displaying when Account ID is empty
- [whatsapp] Instant pairing feedback and owner auto-approve
- [whatsapp] Subscribe to channel events in onboarding WS
- [whatsapp] Only auto-approve owner PN JID, not LID
- [whatsapp] Use PN JID for self-chat replies instead of LID
- [whatsapp] Show phone number in self-chat footer instead of LID
- [channels] Don't drop channel_user WS broadcast
- [whatsapp] Use bare phone number as self-chat session ID
- [agents] Strip trailing stop tokens leaked by LLMs as content
- [agents] Address PR review comments
- [providers] Repair openai compat CI regressions (#722)
- [agents] Collapse nested if per clippy collapsible_if lint
- [web] Restore setup onboarding alias
- [providers] Add null to enum arrays for nullable optional props (#724)
- [mcp] Treat any HTTP response as alive in health check (#732) (#733)
- [chat] Broadcast user_message event so API-sent messages appear in web UI (#734)

## [20260414.02] - 2026-04-14
### Changed
- [node-exec] Merge node exec types into owners
- [web] Remove prompt memory toolbar from chat header


### Fixed
- [web] Harden flaky e2e tests for cron delete and identity autosave
- [sandbox] Mask /proc and /sys host metadata in Docker/Podman containers
- [sandbox] Address PR review — strengthen test assertions
- [channels] Use rand::RngExt for random_range after rand 0.10 upgrade
- Update rand 0.10 imports across workspace
- [onboarding] Surface local-llm and LM Studio in recommended providers
- [auth] Update password minimum length strings from 8 to 12 characters
- [slack] Bump slack-morphism to 2.20 to enable TLS for socket mode
- Regenerate Cargo.lock for reqwest 0.13 and add query feature
- [web] Restore clear button for main session modal (#671)

## [20260414.01] - 2026-04-14
### Changed
- [node-exec] Merge node exec types into owners
- [web] Remove prompt memory toolbar from chat header


### Fixed
- [web] Harden flaky e2e tests for cron delete and identity autosave
- [sandbox] Mask /proc and /sys host metadata in Docker/Podman containers
- [sandbox] Address PR review — strengthen test assertions
- [channels] Use rand::RngExt for random_range after rand 0.10 upgrade
- Update rand 0.10 imports across workspace
- [onboarding] Surface local-llm and LM Studio in recommended providers
- [auth] Update password minimum length strings from 8 to 12 characters

## [20260413.06] - 2026-04-13
### Fixed
- [web] Gate openai live e2e server on api key

## [20260413.05] - 2026-04-13
### Added
- [voice] Add local whisper web setup
- [chat] Show cached input tokens


### Changed
- [providers] Normalize openai schemas with schema crates
- [providers] Move schema normalization out of mod.rs
- [chat] Unify usage propagation paths


### Removed
- [httpd] Remove unused route import
- [httpd] Drop unused get import


### Fixed
- [providers] Sanitize openai tool schemas
- [providers] Address PR review feedback
- [providers] Keep openai compat module under file limit
- [providers] Avoid expect in schema tests
- [web] Restore nostr channel icons
- [voice] Address PR review feedback
- [chat] Satisfy clippy in cached token tests
- [agents] Preserve streamed cache token usage
- [openai] Add live e2e coverage and schema guard
- [sessions] Wire archived sessions through gateway and web ui
- [sessions] Address archived review feedback
- [sessions] Allow archiving non-current channel chats
- [sessions] Preserve orphaned search hits
- [sessions] Allow unarchiving active channel chats

## [20260413.04] - 2026-04-13
### Changed
- Split oversized rust modules
- Split remaining oversized rust modules
- Remove replaced module entrypoints


### Removed
- [agents] Remove duplicate tool arg match arm


### Fixed
- [providers] Preserve native tool arg types
- [gateway] Repair split module blockers
- Restore post-merge validation
- Repair local lint regressions
- [chat] Clean up split validation fallout
- [gateway] Remove split warning debt
- [httpd] Restore split module boundaries
- [httpd] Drop stale server re-export
- [httpd] Repair test and bridge lint fallout
- [httpd] Restore vault test imports
- [telegram] Mark byte-truncated documents
- [httpd] Repair ngrok server split
- [provider-setup] Restore ollama validation payload
- [gateway] Drop duplicate broadcaster default impl
- [gateway] Remove stale mcp service doc comment
- [httpd] Move runtime-only route import
- [tools] Scope apple sandbox helpers to macos
- [tools] Restore sandbox test imports
- [ci] Restore main build stability

## [20260413.03] - 2026-04-13
### Fixed
- [gateway] Resolve release CI regressions

## [20260413.02] - 2026-04-13
### Added
- [voice] Add base_url config for OpenAI TTS and Whisper STT providers


### Changed
- [browser] Type browserless API version as enum
- [nodes] Extract core types and constants to dedicated crate
- [nodes] Move pure ssh and env helper functions to node-exec-types
- [mcp] Extract config parsing functions to moltis-mcp
- [mcp] Extract MCP-agent tool bridge to dedicated crate
- Group lock-free broadcast state into Broadcaster struct
- [tools] Unify cron schema field helper


### Fixed
- [telegram] Persist inbound documents
- [review] Make inbound document replay robust
- [browser] Add Browserless v2 websocket fallback support
- [browser] Change log level from warn to debug for sandboxed browser websocket connection failures
- Complete MCP bridge extraction and address Greptile review
- [broadcast] Make seq private, add #[must_use] to next_seq, add tests
- [chat] Serialize data dir override tests
- [voice] Address PR review feedback
- [tools] Hide wasm internals and relax cron schema
- [tools] Simplify cron schema unions
- [agents] Preserve legacy tool-call compatibility


### Security
- [nodes] Define NodeInfoProvider trait for decoupling

## [20260413.01] - 2026-04-13

## [20260412.01] - 2026-04-12
### Added
- [discord] Handle inbound voice and image attachments
- [hooks] Include channel provenance in payloads
- [web] Add Projects section to Settings sidebar navigation
- [tools] Native filesystem tools (Read, Write, Edit, MultiEdit, Glob, Grep)
- [tools] Typed error taxonomy for Read (not_found / permission_denied / too_large / not_regular_file)
- [tools] Phase 1 polish (byte cap, session key, fs-tools feature, contract tests)
- [tools] CRLF-tolerant Edit recovery for fs tools
- [tools] Per-session FsState with must-read-before-write + re-read loop detection
- [tools] [tools.fs] config + path allow/deny policy (phase 4)
- [tools] Checkpoint_before_mutation + binary base64 + respect_gitignore + docs page
- [tools] Phase 2 sandbox bridge for Read/Write/Edit/MultiEdit/Glob
- [tools] Phase 2b — Grep sandbox routing
- [tools] Phase 3c — adaptive Read paging coupled to context window
- [chat] Deterministic compaction with budget discipline
- [chat] Pluggable compaction modes with config + docs
- [chat] Implement recency_preserving compaction mode
- [chat] Implement structured compaction mode
- [chat] Surface mode + token usage in compaction broadcasts
- [chat,web] Surface compaction mode + tokens in UI and channels
- [compaction] Add chat.compaction.show_settings_hint opt-out
- [tools] Claude Code compat — BOM strip, binary extensions, smart quotes, mtime tracking
- [tools] PDF text extraction in Read tool
- [tools] Image dispatch in Read + Grep context alias + Edit param aliases
- [memory] Add prompt memory styles
- [memory] Extend config surfaces
- [provider-setup] Remove automatic model probe, add manual Test button
- [tools] Auto-page reads and serialize fs mutations
- [tools] Expose read continuation offsets
- [tools] Surface sandbox scan truncation
- [tools] Wire layered tool policy into runtime with per-provider, per-agent, per-channel, and per-sender support
- [tools] Add sandbox tools_policy as layer 6 in policy resolution
- [channels] Add Nostr DM channel support
- [channels] Add Nostr web UI, E2E tests, and documentation
- [channels] Add Nostr to onboarding flow
- [nostr] Add metrics, NIP-44 decryption, and integration tests
- [website] Add slack, matrix, and nostr channels
- [website] Add Nostr channel SVG icon
- [chat] Add summary budget discipline for compaction
- [website] Add community quote from discussion #680 and make quotes horizontally scrollable
- [web] Add option to disable terminal in Web UI
- [auth] Add brute-force protection with IP ban and account lockout


### Changed
- [chat] Align multimodal rewrite updates
- [telegram] Extract STT setup hint constant
- [discord] Reuse inbound downloader per handler
- [chat] Simplify deterministic compaction module
- [chat] Split compaction_run into per-strategy submodules
- [tools] Use ripgrep crates for fs grep
- [tools] Split read.rs into read/ module with pdf.rs and image.rs
- [tools] Centralize sandbox filesystem access
- [media] Sniff mime and expose sandbox file ops
- [tools] Add native host sandbox file ops
- [tools] Add container-aware fs transports
- [tools] Stream OCI file transfers
- [tools] Stream OCI reads from cp


### Fixed
- [agents] Dispatch ToolResultPersist hooks
- [agents] Sanitize ToolResultPersist tool names
- [agents] Handle Z.AI text tool calls and dedupe providers
- [gateway] Address PR comment follow-ups
- [hooks] Honor MessageReceived actions
- [chat] Harden MessageReceived hook handling
- [telegram] Avoid placeholder voice fallbacks
- [telegram] Preserve caption when STT unavailable
- [discord] Address review feedback
- [discord] Log stt-unavailable voice notes
- [common] Block mapped ipv6 ssrf bypass
- [hooks] Address review feedback
- [chat] Warn on invalid channel bindings
- [hooks] Address remaining provenance feedback
- [agents] Warn on invalid hook channel context
- [plugins] Refresh hook docs and logger timestamps
- [common] Update message received fixtures
- [web] Store container ref in teardownProjects, remove unused settings_projects route
- [tools] Fs tools require absolute paths, add workspace_root for Glob/Grep
- [agents] Warn on tool name collision in ToolRegistry::register*
- [compaction] Address code review issues #1-7
- [compaction] Address Greptile PR #653 review comments
- [chat] Correct recent_messages_preserved flag in compaction
- [compaction] Strip preamble and directive from re-compaction extraction
- [compaction] Invert candidate order so bullets outlive plain lines
- [compaction] Protect summary tags from budget-pressure dropping
- [compaction] Address PR #653 review findings
- [chat] Memory-file summary lookup across all compaction modes
- [compaction] Extract_summary_body picks newest summary on iterative re-compaction
- [chat] Wire chat.compaction.threshold_percent into auto-compact trigger
- [compaction] Address Greptile round-2 review on #653 (P1 + P2)
- [web] Track compacting status message per-session to avoid removing compact card
- [compaction] Restore 0.95 default threshold + remove dead code
- [chat] Close compact/store race and tag auto_compact broadcast paths
- [compaction] Address Greptile round-5 P2 findings on #653
- [tools] Enforce exec allowlist when approval_mode is off
- [tools] Deny dangerous commands in off mode instead of hanging
- [tools] Warn when safe-bin bypasses explicit allowlist in off mode
- [config] List tools.policy.profile in preset silent-policy warning
- [tools] Address filesystem review comments
- [tools] Address Greptile round-2 findings (max_read_bytes wiring, sandbox note_fs_mutation, truncated semantics)
- [tools] Address Greptile round-3 findings (Grep policy, must-read sandbox, Glob root deny)
- [tools] Address Greptile round-4 findings (binary read tracking, sandbox Write new-file)
- [tools] Sandbox Grep post-filters results through path policy
- [tools] PDF/image dispatch now enforces path policy, sandbox guard, and FsState recording
- [memory] Tighten qmd and rpc validation
- [chat] Honor [skills] enabled=false at runtime
- [chat] Replace test Mutex<()> with Semaphore
- [chat] Reuse existing config in context skill discovery
- [httpd] Redirect remote setup traffic to onboarding wizard
- [agents] Detect and break tool-call reflex loops (#658)
- [agents] Address Greptile review feedback on #658
- [agents] Loop detector handles mixed-outcome batches correctly (#658)
- [agents] Treat success=false without error field as failure (#658)
- [e2e] Wait for Preact render flush in matrix senders test
- [web] Show Clear button for main session in modal
- [memory] Add missing runtime module
- [web] Remove unused VALIDATION_HINT_RUNNING_TEXT, clear stale test results
- [gateway] Unify config-override test lock to prevent flaky test
- [tools] Smart-quote recovery preserves file content + sandbox grep uses PCRE
- [tools] Scope tar helper to tests
- [tools] Preserve loop warnings for auto-paged reads
- [chat] Populate sender_id in channel binding and runtime context
- [chat] Read sandbox state from runtime context in PolicyContext
- [agents] Add missing channel_sender_id field in test
- [tools] Use struct init instead of field reassign in test
- [tools] Expand profile field in provider, sender, and sandbox policy layers
- [httpd] Start stored channels on vault unseal
- [httpd] Add missing continue in unsupported channel type guard
- [config] Revert unintended matrix named field promotion
- [nostr] Fix integration test DM round-trip reliability
- [nostr] Prevent panic on UTF-8 boundary when truncating large DMs
- [website] Use official Nostr protocol logo (CC0, mbarulli/nostr-logo)
- [web] Preserve Nostr OTP settings on edit modal save
- [nostr] Resolve clippy collapsible_match and len_without_is_empty
- [nostr] Implement OTP challenge initiation and DM delivery
- [nostr] Implement OTP verification path for code replies
- [nostr] Use std::sync::RwLock for accounts map to avoid blocking panic
- [web] Remove channel-error class from conditionally-rendered error divs
- [chat] Correct budget accounting bugs in compress_summary
- [chat] Address greptile review feedback (greploop iteration 1)
- [channels] Finish discussion 425 follow-ups
- [channels] Address review feedback
- [channels] Harden channel command authorization and session scoping
- [gateway] Truncate approval previews safely
- [channels] Chunk unicode safely
- [chat] Isolate sqlite memory tests
- [web] Stabilize mocked channel refresh
- [channels] Address greptile review feedback
- [channels] Truncate command in approve/deny confirmation messages
- [auth] Second-pass security hardening
- [web] Address PR review comments
- [config] Preserve TOML section order on web UI save
- Apply local fixes


### Security
- [config] Warn when preset tool policies are set but tools.policy is empty
- [nostr] Address PR review comments
- [auth] Harden remote access with 9 security improvements

## [20260410.01] - 2026-04-10
### Added
- [oauth] Log loopback redirect URI rewrites at debug level
- [skills] Ship native read_skill tool
- [skills] Harden read_skill with assets/, binary files, and metadata surfacing


### Changed
- [oauth] Share loopback redirect normalizer and apply to provider setup
- [oauth] Eliminate dead branches in normalize_loopback_redirect


### Removed
- [web] Stabilize node selector and fork delete e2e


### Fixed
- [gateway] Dcg-guard PATH augmentation and loud missing-dcg warning
- [gateway] Refresh stale dcg-guard files and use async subprocess
- [gateway] Dcg-guard HOME fallback and unconditional startup log
- [agents] Suppress auto-continue after substantive final answer
- [agents] Address review feedback on auto-continue fix
- [mcp] Normalize loopback redirect URIs to http for OAuth registration
- [provider-setup] Normalize pre-loaded loopback redirect URIs
- [skills] Address Greptile review feedback on read_skill
- [skills] Address greptile review feedback (greploop iteration 2)
- [skills] Cap skill body size in read_primary (greploop iteration 3)
- [skills] Per-subdir sidecar cap + data_dir-scoped discoverer
- [voice] Honor whisper.model and whisper.language in STT factory


### Security
- [hooks] Pin dcg install to tag and verify checksum

## [20260409.04] - 2026-04-09
### Added
- [providers] Add Alibaba Cloud Coding Plan provider


### Fixed
- [ci] Avoid dynamic provider secrets

## [20260409.03] - 2026-04-09
### Removed
- Remove redundant http client fallback check


### Fixed
- [common] Ensure User-Agent survives all HTTP client fallback paths
- [test] Address PR review — clarify comment and assert no Error events
- [crons] Persist schedule field values across modal re-renders
- [crons] Read schedule fields as snake_case from server responses
- [providers] Deliver MiniMax system prompt via first user message
- [tests] Use streaming probe for OpenAI integration tests
- [providers] Bump GPT-5 probe output cap to 16 tokens
- [tests] Use Secret<T> for API keys in model discovery tests
- [providers] Handle multimodal content in MiniMax system prompt rewrite

## [20260409.02] - 2026-04-09
### Added
- [msteams] Comprehensive Teams channel implementation
- [providers] Add Gemini 3.x models to catalog and update capability detection
- [providers] Add ModelCapabilities struct to ModelInfo and DiscoveredModel
- [chat] Use ModelCapabilities in API responses instead of provider lookups


### Fixed
- [web] Integrate Tailscale Funnel into Teams channel setup
- [web] Remove 'Requires public URL' badge from Teams card
- [web] Simplify Teams onboarding now that Remote Access step exists
- [msteams] Address PR review feedback
- [msteams] Use Graph token for reactions and thread context
- [msteams] Prevent streaming retry storm and URL injection in search
- [web] Disambiguate OAuth E2E selector for model picker
- [web] Keep new chats at top of sidebar
- Auto-allow direnv in superset worktree setup
- Load BOOT.md per-session via system prompt instead of broken hook (#594)
- Remove stale boot-md assertion from discover_hooks test
- Update model count assertion for gemini-3 reasoning variants
- [installer] Address PR review feedback
- [browser] Include Podman in container availability check
- [browser] Update stale comment to include Podman
- [gateway] Narrow skill and memory watch roots
- [gateway] Avoid blocking skill watcher refresh
- [providers] Extract MiniMax system messages to top-level field
- [providers] Warn on non-string MiniMax system message content
- [providers] Add PartialEq/Eq to ModelCapabilities, use infer() in tests
- [providers] Narrow gemma exclusion to gemma-3n- only
- [agents] Surface workspace prompt truncation
- [gateway] Address review feedback
- [gateway] Match workspace prompt normalization
- [agents] Make workspace file truncation limit configurable (#593)
- Replace magic constant with ChatConfig::default() fallback
- Eliminate double chars().count() and add zero-value validation
- Report truncation when max_chars is zero
- Remove duplicate workspace_file_max_chars key in schema map
- [tools] Wire ExecConfig timeout and max_output_bytes to ExecTool
- [tools] Make timeout schema description reflect configured default
- [providers] Resolve 404 when selecting Ollama model in web UI
- [web] Preserve ollama pull hint in humanizeProbeError
- [providers] Forward auth header in Ollama native probe fallback
- Align auth middleware tests with gateway state

## [20260409.01] - 2026-04-09
### Added
- [matrix] Add slash command support
- [models] Make model detection opt-in and add stop button


### Fixed
- [matrix] Match help command by exact name, not prefix
- [models] Abort probe tasks on cancel, show feedback, await RPC
- [tls] Include lan bind SANs in auto-generated certs
- [tls] Address PR review feedback
- [agents] Use system message for auto-continue nudge instead of user message
- [common] Add default User-Agent header to shared HTTP client
- [common] Use MOLTIS_VERSION for default user-agent and apply headers in apply_proxy
- [agents] Keep auto-continue nudge as user message
- [provider-setup] Include lmstudio in known_providers and replace ollama name checks
- [provider-setup] Add dedicated local_only field to KnownProvider
- Harden superset setup envrc handling

## [20260408.01] - 2026-04-08
### Added
- [agents] Auto-continue when model stops mid-task + max iterations UX
- [config] Make auto-continue tool-call threshold configurable


### Fixed
- Address PR review — translatable continue message, document tool-call threshold
- Guard auto-continue against min_tool_calls=0 usize tautology
- [minimax] Restore system prompts and null tool args
- [providers] Discover live anthropic models
- [providers] Mark anthropic recommendations globally

## [20260407.01] - 2026-04-07
### Added
- [webhooks] Add generic webhook ingress for triggering AI agents
- [web] Link to Hoppscotch for webhook testing
- [web] Add CORS to webhook ingress and copy-curl button
- [website] Add Webhooks to landing page features


### Changed
- [web] Extract webhooks nav icon to external SVG file


### Fixed
- [cli] Report release version in --version output
- [providers] Propagate cache tokens in Responses API and custom providers
- [providers] Read cached_tokens from input_tokens_details in non-streaming Responses SSE
- [agents] Match provider-specific context window error strings
- [chat] Honor public sessionKey in GraphQL flows
- [chat] Address PR review — safer mock assertions and precedence test
- [web] Use globe icon for webhooks settings nav
- [web] Use ModelSelect and ComboSelect for webhook agent/model fields
- [web] Use public URL (ngrok/tailscale) for webhook endpoint display
- Improve webhook test script with verbose output and TLS support
- Use OnceLock for webhook state fields instead of Arc::get_mut
- [web] Show Hoppscotch link below webhook list, not only in empty state
- [web] Match webhooks layout to cron/sandbox and fix nav icon
- [web] Add missing space before Hoppscotch link
- [web] Constrain webhooks list width with max-w-form
- [web] Revert max-w-form, keep webhooks list full width
- [web] Add webhooks icon to settings nav via components.css
- [web] Recommend curl and Hoppscotch desktop for webhook testing
- Use HeaderValue::from_static for CORS headers, remove warning
- [web] Improve webhook test command button and footer text
- Don't dedup generic webhooks by body hash, rebuild i18n
- Address PR review comments
- [security] Redact auth secrets from webhook API responses
- Drain unprocessed webhook deliveries on worker startup
- Include 'processing' deliveries in crash-recovery drain
- Gitlab_token auth config key mismatch, add regression tests
- [security] Fail closed on non-parseable Stripe signature timestamp
- Enable foreign_keys pragma and explicit cascade delete
- [ci] Restore CHANGELOG.md to main state (changelog guard)
- Forward agent_id to chat.send_sync in webhook worker
- Enforce CIDR allowlist, set foreign_keys on pool options
- [security] Gate forwarded headers on behind_proxy, fix PagerDuty multi-sig
- Use ConnectInfo for direct IP, disable source_profile on edit
- [webhooks] Harden webhook execution and secrets
- Proactive audit — 6 issues found and fixed
- Wrap cascade deletes in transactions to prevent partial data loss
- Resolve settings nav CI regression


### Security
- Add webhooks feature documentation

## [20260406.05] - 2026-04-06
### Added
- [openclaw-import] Convert non-default agents to spawn_agent presets


### Fixed
- [web] Allow session sidebar links to open in new tabs
- [web] Tighten session sidebar link accessibility
- [docker] Add missing default features to Dockerfile build
- [docker] Use default features instead of explicit list

## [20260406.04] - 2026-04-06
### Added
- [website] Add provider/channel pills section and update branding
- [website] Add positioning, how-it-works, use cases, and community quote


### Changed
- [providers] Avoid quadratic SSE buffer copies
- [providers] Align copilot stream error handling


### Fixed
- [providers] Route Copilot enterprise tokens via proxy endpoint (#352)
- [providers] Address PR review comments on Copilot enterprise
- [providers] Harden Copilot enterprise proxy security
- [providers] Reject bare IP addresses in Copilot proxy-ep
- [providers] Address Copilot enterprise review feedback
- [providers] Stream enterprise copilot responses
- [website] Crop MiniMax icon, grayscale raster icons, official GraphQL logo
- [website] Add Discord source links to community quote
- [website] Update LoC stats on security page
- [website] Update LoC stats in all locale files
- [website] Regenerate all locale files with new homepage sections
- [website] Update i18n titles and regenerate all locale files
- [website] Translate new homepage sections in all 9 locale files
- [website] Translate remaining English strings in all locale files
- [website] Localize injected nav tabs
- [website] Address greptile review feedback
- [website] Sync i18n builder with locale pages
- [website] Correct i18n generator keys


### Security
- [providers] Redact CopilotTokenResponse token in Debug output

## [20260406.03] - 2026-04-06
### Fixed
- [web] Restore all-features build

## [20260406.02] - 2026-04-06
### Fixed
- [web] Map config reload errors explicitly

## [20260406.01] - 2026-04-06
### Added
- [cron] Auto-clean orphaned sessions and prune sandbox containers


### Fixed
- [swift-bridge] Await embedded httpd shutdown
- [cron] Use time crate for retention math and fix named-session guard
- [sandbox] Include remove_image_override in cleanup_session
- [cron] Skip pruning cycle when session key lookup fails
- [web] Reload offered channels from config
- [ci] Await channel preload in settings e2e

## [20260405.06] - 2026-04-05
### Security
- Add GitHub artifact attestations to release workflow

## [20260405.05] - 2026-04-05

## [20260405.04] - 2026-04-05

## [20260405.03] - 2026-04-05
### Fixed
- [web] Restore matrix onboarding icon

## [20260405.02] - 2026-04-05
### Added
- [providers] Add zai-code provider for Z.AI Coding plan
- [tools] Add cross-session search recall
- [tools] Add automatic edit checkpoints
- [projects] Harden context loading
- [skills] Add portable bundle quarantine flow
- [exec] Add ssh remote routing
- [web] Add skills bundle ui and ssh target visibility
- [web] Clarify ssh execution targets
- [ssh] Add managed deploy keys and targets
- [nodes] Add remote exec doctor panel
- [ssh] Harden managed targets and host pinning
- [nodes] Repair active ssh host pins from doctor
- [ssh] Add actionable runtime failure hints
- [web] Add tools overview to settings
- [web] Allow renaming channel-bound sessions
- [security] Add GPG signing for release artifacts
- [security] Add release verification script
- [gateway] Add channel settings agent tool
- [providers] Collapse model lists, hide legacy models, add recommended flag
- [web] Add live remote access settings
- [remote] Improve onboarding public access flow
- [tools] Add Firecrawl integration for web scraping and search
- [web] Add recommended provider tier in onboarding and docs guide
- [config] Add upstream_proxy for application-level HTTP proxy support
- [channels] Add Matrix channel integration
- [matrix] Complete channel parity and web ui coverage
- [matrix] Add encrypted chat support and vault-backed channel secrets
- [matrix] Add account ownership mode
- [matrix] Harden ownership recovery flow
- [matrix] Add generic channel location fallback


### Changed
- [ssh] Use secrecy for imported key material
- Replace vendored sqlx-sqlite with git dependency


### Removed
- [web] Remove unused gon import


### Fixed
- [providers] Address PR review comments for zai-code
- [vault] Allow unencrypted session history while sealed
- [vault] Address PR review comments
- [security] Address PR review feedback
- [ssh] Tighten timeout and warning handling
- [ssh] Address latest review follow-ups
- [gateway] Collapse legacy ssh node lookup
- [auth] Guard ssh key deletion race
- [httpd] Satisfy ssh route lint
- [ssh] Reject option-like targets
- [ssh] Hide import passphrases from argv
- [ssh] Quote known hosts path
- [web] Use browser location port for node join URL
- [web] Guard e2e assertion for default-port case
- [e2e] Use sidebar selector for sealed-vault session visibility test
- [web] Fall back to getRandomValues for session UUID on plain HTTP
- [web] Address PR review feedback
- [providers] Speed up model probes
- [security] Address PR review feedback for GPG signing
- [security] Prevent gpg --import grep from aborting verify script
- [security] Show GPG signer identity and failure diagnostics
- [security] Pin GPG key fingerprint to prevent TOFU attacks
- [voice] Surface elevenlabs stt failures
- [gateway] Address channel settings PR feedback
- [voice] Handle empty stt transcripts
- [web] Show unsupported model reason inline instead of tooltip-only
- [web] Show probe error inline in model selector cards
- [web] Show probe error inline in preferred models selector
- [web] Preserve server error message for model probes
- [web] Sort and collapse onboarding model selector like settings
- [web] Sort models by version number when no date available
- [httpd] Harden ngrok controller lifecycle
- [httpd] Retain ngrok controller after startup
- [ngrok] Harden loopback tunnel handling
- [ngrok] Avoid fatal startup on tunnel errors
- [ngrok] Clarify defaults and warnings
- Update local setup and ElevenLabs error logging
- [web] Localize toggle button in onboarding and fix JSDoc comment
- [chat] Use relative timestamps in created_at test
- [tools] Address firecrawl PR review feedback
- [tools] Resolve firecrawl web_search registration and timeout race
- [agents] Stabilize prompt cache and compact tool results
- [web] Stabilize send-document e2e test
- [mcp] Address PR review comments for streamable HTTP transport
- [mcp] Update template docs and log messages for streamable HTTP
- [providers] Default to vision support for unknown models (#556)
- [providers] Also exempt gpt-4-vision from denylist
- [providers] Surface real error on provider probe failure
- [web] Apply serverMessage pattern to validateProviderConnection
- [web] Allow multi-model selection during provider setup
- [web] Scope Select All to visible models, check save_models response
- [config] Wrap upstream_proxy in Secret<String> and redact credentials in logs
- [proxy] Use rfind for @ redaction, warn on parse failure, document Slack gap
- [providers] Rediscover models from /v1/models before probing
- [providers] Address PR review — move Ollama probes outside lock, use runtime env
- [providers] Check total model count in RediscoveryResult::is_empty
- [matrix] Address review feedback
- [matrix] Address latest review feedback
- [matrix] Set reply thread ids after main merge
- [matrix] Gate DM invites through dm_policy instead of room_policy
- [matrix] Gate poll responses through access control
- [voice] Use inspect_err in elevenlabs stt
- [web] Preserve @ in matrix allowlists
- [matrix] Unify otp approval flow and sender visibility
- [web] Default matrix setup to password auth
- [matrix] Improve ownership recovery UX
- [graphql] Implement retry ownership in test mock
- [e2e] Correct username assertion in matrix senders test
- [web] Satisfy biome hook and lint checks

## [20260328.03] - 2026-03-28
### Fixed
- [telegram] Route forum-topic replies to correct thread
- [telegram] Restore raw chat_id in logs, add thread_id to tracing
- [providers] Increase model probe timeout for local LLM servers
- [providers] Address PR review feedback

## [20260328.02] - 2026-03-28
### Added
- [telegram] Isolate forum-topic sessions by thread_id


### Changed
- [telegram] Consolidate parse_chat_target, fix typing indicator


### Fixed
- [telegram] Propagate thread_id parse errors in parse_chat_target
- [providers] Skip model discovery for custom providers with explicit models
- [providers] Replace redundant test with one that pins the new guard
- [provider-setup] Skip probe for custom providers without model
- [provider-setup] Remove redundant is_chat_capable_model filter
- [chat] Use system role for compaction summary
- [chat] Use user role for compaction summary
- [providers] Restore MiniMax top-level system prompt extraction
- [telegram] Allow unwrap in topic tests to satisfy workspace clippy lints
- [provider-setup] Use Arc<AtomicBool> instead of static in test

## [20260328.01] - 2026-03-28
### Added
- [website] Add local dev server with SSR partial injection
- [web] Add changelog link to header nav
- [providers] Add prompt caching for Anthropic and OpenRouter
- [telegram] Extract plaintext and markdown documents from messages
- [providers] Add Fireworks.ai as primary provider


### Changed
- [website] Shared nav via SSR partial, add Changelog link
- [telegram] Use std::str::from_utf8 for UTF-8 truncation
- [telegram] Normalize MIME type once, avoid redundant UTF-8 scans


### Fixed
- [website] Allow nav links without data-page to navigate normally
- [website] Highlight Changelog tab and show GitHub stars on /changelog
- [website] Share GitHub stars script via nav partial, fix Changelog click
- [web] Point report issue link to template chooser
- [providers] Align indentation in stream_with_tools debug macro
- [telegram] Address PR review comments on document handling
- [telegram] Enforce char limit on all document content paths
- [telegram] Prevent U+FFFD from byte-boundary truncation of CJK text
- [install] Remove spurious -1 revision from .deb filename in installer

## [20260327.05] - 2026-03-27
### Changed
- [release] Build changelog HTML in prepare-release instead of CI

## [20260327.04] - 2026-03-27
### Fixed
- [ci] Use file input for changelog blob to avoid argument list too long

## [20260327.03] - 2026-03-27
### Added
- [website] Add changelog HTML page and fix RPM version override

## [20260327.02] - 2026-03-27
### Fixed
- [install] Support date-based version tags in installer and package builds

## [20260327.01] - 2026-03-27
### Added
- [gateway] Embedded web chat UI at root endpoint
- [gateway] Add services, pairing, expanded methods and auth
- [agents] Add LLM chat with streaming, multi-provider support and feature flags
- [gateway] Add Tailwind-based chat UI with dark/light theme
- [config] Add multi-format config file with provider enable/disable support
- [gateway] Add model selector and WebSocket auto-reconnect
- [oauth] Add OpenAI Codex OAuth provider and reusable OAuth infrastructure
- [tools] Add LLM code execution with agent loop, tool calling, and security layers
- [agents] Add debug logging and end-to-end exec tool test
- [agents] Text-based tool calling fallback for non-native providers
- [gateway] Log user message on chat.send
- [memory] Implement memory management system with hybrid search
- [tools] Wire approval gating into exec tool with UI
- [brew] Add Homebrew formula for tap-based installation
- [website] Add static site and roadmap for moltis features
- [website] Rewrite with Tailwind CSS, Inter/JetBrains fonts, and polish
- [packaging] Add Debian package builds for amd64 and arm64
- [packaging] Add Arch Linux package builds for x86_64 and aarch64
- [packaging] Add RPM, Flatpak, Snap, AppImage, Nix, and Homebrew packaging
- [agents] Register all Codex models in provider registry
- [gateway] Structured error handling, exec cards, and approval UI
- [gateway] Add provider management UI with multi-model support
- [gateway] Persist API keys and add session management
- [gateway] Add session sidebar UI and fix session management
- [gateway] Route chat events by session key, add unread dots and thinking restore
- [agents] Add missing LLM providers and GitHub Copilot OAuth
- [gateway] Add session search with autocomplete, scroll-to and highlight
- [gateway] Include model and provider name in chat final events
- [claude] Save plans and sessions to prompts/ via hooks
- [gateway] Multi-page SPA with nav panel, crons page and methods page
- [cron] Wire cron callbacks and register CronTool for LLM use
- [cron] Implement production-grade cron scheduling system
- [projects] Add project management with context loading and session binding
- [gateway] Searchable model selector, per-session model, chat history
- [gateway] Persist model/provider in chat history and style model footer
- [gateway] Add token usage display per-message and per-session
- [gateway] Move model selector to chat page, add providers to nav panel
- [projects,sessions] Migrate project and session metadata storage to SQLite
- [gateway] Reorganize navigation, move providers to dedicated page
- [sandbox] Per-session sandbox toggle with sandbox-on-by-default
- [gateway] Show LLM thinking text, persist token usage, fix chat scroll
- [gateway] Add slash commands (/clear, /compact, /context) with autocomplete
- [context] Display sandbox details in /context command
- [providers] Add Kimi Code OAuth (device flow) provider
- [gateway] Add Channels navigation page
- [gateway] Move enabled toggle to last column in cron job table
- [telegram] Add username allowlist matching and message log store
- [compact] Auto-compact on context limit, use session provider, show summary card
- [worktree] Implement workspace worktree lifecycle features
- [ui] Project selector combo in chat header with session filtering
- [telegram] Per-channel sessions, slash commands, and default model config
- [gateway] Add logs/forensic page with real-time streaming and persistence
- [telegram] Command autocomplete, /new session, /sessions with inline keyboard
- [gateway] Live session updates, channel icons, and active session indicator
- [gateway] Add amber ping dot indicator for active sessions
- [gateway] UI improvements and provider/context enhancements
- [skills] Add agent skills system crate with discovery, registry, and CLI
- [gateway] Add Skills navigation page to web UI
- [skills] Repository-based skills with per-skill enable/disable
- [skills] Accept GitHub URLs in skill source input
- [gateway] Add native HTTPS/TLS support behind `tls` cargo feature
- [gateway] Hybrid asset serving with filesystem dev and embedded release
- [sandbox] Configurable images, on-demand caching, Apple Container auto-detection
- [onboarding] Replace wizard with inline identity editing in Settings
- [plugins] Add plugins crate with format adapters, install, and chat integration
- [gateway] Add Images page for managing sandbox container images
- [telegram] Add /model, /sandbox commands and improve /context display
- [gateway] Add setup code at startup and configurable config/data dirs
- [gateway] Clean auth state separation, gon pattern, and identity improvements
- [env] Add write-only environment variables with sandbox injection
- [security] Wrap secrets in secrecy::Secret<String> to prevent leaks
- [hooks] Add hook dispatch system with native and shell handlers
- [hooks] Add hook discovery, eligibility, metadata, CLI commands, and bundled hooks
- [memory] Wire memory system into gateway with tools, compaction, and session hooks
- [memory] Add embedding cache, local GGUF, fallback chain, batch API, file watcher, and pre-compaction flush
- [tools] Add web_search and web_fetch agent tools
- [sandbox] Add tracing to Apple Container and exec tool lifecycle
- [ui] Show container name in /context sandbox section
- [web] Forward client Accept-Language to web_fetch and web_search
- [sandbox] Auto-provision curl, python3, nodejs, npm in containers
- [sandbox] Expand default packages inspired by GitHub runner images
- [sandbox] Apple Container pre-built images, CLI commands, default config
- [sandbox] Expand packages from GitHub runner images, use Secret for web search keys
- [hooks] Wire all 15 hook events and add examples
- [chat] Add per-session run serialization to prevent history corruption
- [chat] Add configurable agent-level timeout enforcement
- [agents] Retry agent loop after compaction on context window overflow
- [tools] Add SpawnAgentTool for sub-agent / nested agent support
- [chat] Add message queue modes for concurrent send handling
- [agents] Sanitize tool results before appending to LLM message history
- [agents] Execute tool calls concurrently with join_all
- [mcp] Add MCP client support with discovery UI
- [gateway] Add nav sidebar count badges and MCP UI improvements
- [mcp] MCP context in chat, duplicate name handling, and misc improvements
- [mcp] Add McpTransport and McpClientTrait trait abstractions
- [mcp] Wire MCP tool bridges into agent ToolRegistry
- [mcp] SSE transport, health polling, auto-restart, and edit config
- [gateway] Add tailscale serve/funnel management, UI consistency overhaul, and HTTP/2 support
- [memory] Log status with DB size after initial sync
- [tailscale] Add Start Tailscale button when daemon is not running
- [channels] Assign default model to new telegram sessions
- [agents] Add model failover with per-provider circuit breakers
- [gateway] Add report an issue link to nav sidebar
- [config] Support MOLTIS_* env var overrides for all config fields
- [cron] Add heartbeat feature with persistent run history
- [cli] Add cargo-binstall support for binary installation
- Add Homebrew tap and auto-update workflow
- Generate random port on first run and make gateway the default command
- [ci] Add multi-arch Docker build workflow
- [agents] Enable streaming responses with tool support
- [metrics] Add Prometheus metrics with feature-gated support
- [metrics] Expand metrics to all crates with tracing feature
- [metrics] Add provider alias support for metrics differentiation
- [metrics] Add SQLite persistence and per-provider charts
- [db] Add sqlx migrations with per-crate ownership
- [local-llm] Add local LLM provider with GGUF/MLX backend selection
- [local-llm] Add MLX models and filter by backend
- [local-llm] Add HuggingFace search and custom model support
- [agents] Add unified local-llm provider with pluggable backends
- [ui] Add auto-search with debounce for HuggingFace search
- [ui] Show chat-only mode notice when selecting model without tools
- [ui] Show all configured models in providers page
- [providers] Add per-model removal and disable support
- [local-llm] Add tracing and metrics to GGUF provider
- [cli] Add database management commands (db reset, clear, migrate)
- [telegram] Auto-disable channel when another bot instance is running
- Add install script and update URLs to moltis.org
- Add Pi-inspired features — skill state, self-extension, branching, hot-reload
- [auth] Add scope support to API keys
- Typed ToolSource, per-session MCP toggle, debug panel convergence, docs & changelog
- [gateway] Add Fork button to chat header
- [gateway] Show warning banner when running on non-main git branch
- [gateway] Add mobile PWA support with push notifications (#40)
- [tls] Default HTTP redirect port to gateway port + 1 (#49)
- [memory] Add QMD backend support, citations, session export, and LLM reranking (#27)
- [deploy] Add --no-tls flag and one-click cloud deploy configs
- [gateway] Display process and system memory usage in header
- [ui] Red favicon and branch-prefixed title for non-main branches
- [ui] Add security warnings to MCP and Plugins pages
- [ui] Add click-to-view-source detail panel to enabled plugins table
- [deploy] Add MOLTIS_DEPLOY_PLATFORM env var to hide local-only providers on cloud
- [ui] Add allowlist field to onboarding channel step
- [ui] Prefill agent name and emoji in onboarding identity step
- [hooks] Add hooks web UI page
- [hooks] Seed example hook on first run
- [hooks] Add Preview/Source tabs for HOOK.md content
- [hooks] Click-to-copy hook source path
- [hooks] Show built-in hooks in the web UI
- [hooks] Wire up built-in hooks (boot-md, command-logger, session-memory)
- [channels] Replace allowlist textarea with tag-style input
- [workspace] Move persona and startup context to markdown files
- [browser] Add full browser automation support via CDP
- [browser] Enable browser tool by default
- [settings] Add Load Template button and docs link
- [browser] Display screenshot thumbnails in chat UI
- Add restart API and improve browser tool integration
- [browser] Add fullscreen lightbox for screenshot thumbnails
- [local-llm] Auto-detect backend based on model type
- [local-llm] Support Homebrew-installed mlx-lm
- [local-llm] Download and cache MLX models locally
- [local-llm] Notify user when downloading missing models
- [ui] Show download progress in chat when model is missing
- [browser] Add sandboxed browser support using Docker containers
- [browser] Add sandbox mode visibility in logs and response
- [browser] Pre-pull container image at startup with UI feedback
- [browser] Include page content in snapshot response
- [cli] Add browser command for managing browser configuration
- [browser] Add detailed logging for browser execution mode
- [ui] Show browser action and execution mode in chat
- [browser] Add screenshot download functionality
- [browser] Increase default viewport to 1920x1080
- [browser] Increase viewport to 2560x1440 with 2x Retina scaling
- [telegram] Add screenshot support for browser tool
- [browser,telegram] Improve screenshot handling and display
- [browser] Memory-based pool limits instead of fixed count
- [telegram] Send tool execution status during chat
- [logs] Show crate/module target in log output
- [ci] Add local validation gate with CI fallback
- [security] Add emergency disable for third-party skills
- [security] Surface commit provenance in skills UI
- [skills] Add websocket install progress and loading states
- [skills] Use websocket install status with honest UI messaging
- [scripts] Support local-only validation without a PR
- [workspace] Align context files with OpenClaw semantics
- [heartbeat] Skip LLM turns when HEARTBEAT.md is empty
- [heartbeat] Surface empty-file skip status in UI
- [gateway] Surface GitHub release update banner
- [voice] Add voice crate with TTS and STT providers
- [gateway] Integrate voice services with TTS and STT support
- [voice] Add voice UI with feature flag support
- [voice] Add multiple STT providers
- [voice] Add voice provider management UI with auto-detection
- [voice] Add ElevenLabs Scribe STT provider
- [voice] Integrate ElevenLabs Scribe STT and upgrade to v2
- [voice] Improve STT/TTS testing UX and fix ElevenLabs API
- [voice] Send voice messages directly to chat
- [telegram] Transcribe voice messages before dispatching to chat
- [telegram] Add info logging for unhandled media types
- [telegram] Add image support for multimodal LLM messages
- [channels] Improve voice handling and typed STT config
- [voice] Add typed provider metadata and voice preference flows
- [chat] Prefer same reply medium with text fallback
- [voice] Add provider list allowlists and narrow template defaults
- [chat] Add runtime host+sandbox prompt context
- [chat] Add Prompt button to inspect full system prompt
- [sandbox] Add tmux to default packages
- [tools] Add process tool and tmux skill for interactive terminal sessions
- Ship provider onboarding and model discovery improvements
- [ui] Redesign Voice onboarding step with settings-like experience
- [ui] Personalize TTS test phrases with user and bot names
- [gateway] Voice pending UI, TTS phrase generation, and empty message filtering
- [gateway] Persist TTS audio, per-session media, and silent replies
- [gateway] Add Context button showing full LLM messages array
- [gateway] Add queued message UI with cancel support
- [gateway] Add Copy button to full context panel
- [gateway] Add client-side message sequence number for ordering diagnostics
- [gateway] Add run_id and seq to persisted messages for parent/child linking
- [channels] Add reply threading support for Telegram messages
- [gateway] Auto-detect browser timezone via Intl.DateTimeFormat
- [metrics] Populate cache token counters from provider responses
- [tools] Add get_user_location tool with browser geolocation
- [sandbox] Add image, audio, media, and data processing packages
- [sandbox] Add document, office, and search packages
- [sandbox] Add GIS and OpenStreetMap packages
- [tools] Add sandbox_packages tool for on-demand package discovery
- [sandbox] Add communication packages, hybrid sandbox query, and mise
- [telegram] Add location sharing with live location tracking
- [onboarding] Redesign provider step as multi-provider list
- [gateway] Show no-providers card when no LLM models available
- [gateway] Validate-first provider setup in onboarding and settings
- [oauth] Import auto-detected tokens into central store at startup
- [providers] Add model selection for auto-detected providers
- [onboarding] Add summary step and improve channel UX
- [gateway] Add Telegram-style waveform audio player
- [gateway] Add generic session upload endpoint replacing base64-over-WS
- [gateway] Add drag-and-drop image upload to chat and auth tests
- [voice] Add TTS text sanitization and voice-friendly LLM prompting
- [tools] Reverse geocode user location to human-readable place names
- [agents] Retry once on transient LLM provider errors
- [tools] Add show_map tool and location precision modes
- [telegram] Consolidate bot responses into single message with logbook
- [auth] Passkey UX improvements and mDNS origin support
- [benchmarks] Add boot-path performance benchmarks for CodSpeed
- [cli] Add `moltis memory` subcommand with search and status
- [telegram] Send native location pin for show_map tool
- [telegram] Send voice replies with text transcript
- [gateway] Add search to project filter and fix button alignment
- [gateway] Attach project to new sessions from filtered view
- [sessions] Add server-side unread tracking via last_seen_message_count
- [gateway] Auto-detect models after saving provider API key (#83)
- [sessions] Add entity versioning to prevent stale updates
- [gateway] Improve logs filter UX and branch favicon contrast
- [gateway] Add logout button to header bar (#86)
- [gateway] Add comprehensive e2e test suite for web UI (#87)
- [gateway] Add auth-aware endpoint throttling and login retry UX
- [gateway] Auto-detect WebAuthn RP ID from PaaS environment variables
- [cli] Add --version flag
- [gateway] Centralize SPA routes and preserve config TOML comments
- [models] Tighten allowlist matching and support probe output (#91)
- [config] Add moonshot to default offered providers
- [onboarding] Preselect passkey auth method when available
- [config] Auto-create SOUL.md with default content on first run
- [gateway] Disconnect all WS clients on credential changes
- [providers] Prefer configured models and merge live model discovery
- [providers] Multi-select preferred models, keyOptional, createdAt sorting
- [providers] Filter non-chat models and fix per-model tool support
- [providers] Show model dates, probe on select, spacebar pause
- [onboarding] Multi-select model picker with probe badges
- [memory] Inject MEMORY.md into system prompt and fix file watcher
- Auto-select and install browser tool backends (#130)
- [agents] Strip <think> tags from OpenAI-compatible providers and add MiniMax
- [chat] Preserve reasoning text in tool cards across page reloads
- [agents] Add Z.AI (Zhipu) as OpenAI-compatible provider
- Env injection, sandbox recovery, UI fixes, provider improvements (#108)
- [gateway] Show server start time at bottom of onboarding
- [browser] Auto-inject low-memory Chromium flags on constrained systems
- [gateway] Add generic OpenAI-compatible provider support
- [tools] SSRF allowlist for Docker inter-container networking (#146)
- [config] Enable openrouter in default offered providers
- [mcp] Add OAuth 2.1 support for remote MCP servers (#148)
- [telegram] Add channel streaming with stream_mode fallback (#165)
- [tools] Add calc tool for safe arithmetic evaluation
- [cron] Add per-job model and execution target controls (#170)
- [gateway] Cache session histories and show switch loader
- [map] Add provider-aware show_map links
- [gateway] Render markdown and ansi tables in chat
- [browser] Persist Chrome profile across sessions (#162)
- [memory] Reduce baseline memory footprint for lightweight devices
- [telegram] Save voice audio to session media for web UI playback
- [cron] Add event-driven heartbeat wake with system events queue
- [telegram] Render markdown tables as formatted pre blocks
- [gateway] Static file caching for public share pages
- [cron] Deliver agent turn output to Telegram channels
- [gateway] Seed dcg-guard hook and polish onboarding badges
- [graphql] Add GraphQL API exposing all RPC methods (#200)
- [tools] Add send_image tool for channel image delivery (#224)
- [gateway] Expand identity emoji picker options (#206)
- [web] Browsable skills list in repo cards
- [providers] Add configurable OpenAI websocket stream transport (#227)
- [sandbox] Install latest gogcli in default sandbox images (#232)
- [agents] Add multi-agent personas with CRUD UI (#97)
- [caldav] Add CalDAV integration for calendar CRUD (#84)
- [channels] Add Microsoft Teams channel integration (#231)
- [import] Add OpenClaw import crate, CLI, gateway RPC, and UI (#217)
- [web] Add vault algorithm description to encryption settings page
- [web] Internationalization (i18n) with English and French locales (#237)
- [whatsapp] Add WhatsApp channel support (#73)
- [web] Add sandbox shared-home settings UI and config
- [protocol] Upgrade WebSocket protocol from v3 to v4 (#247)
- [agents,providers,chat] Universal tool support for all models
- [openclaw-import] Import workspace files and add safety messaging
- [providers] Promote Gemini to first-class OpenAI-compatible provider
- [i18n] Add full zh-CN (Simplified Chinese) localization (#260)
- Add channel-aware heartbeat delivery and send_message agent tool (#270)
- [memory] Add tree-sitter code splitter and RRF search merge
- [web] Add Shiki syntax highlighting to code blocks
- [sandbox] Add GitHub runner parity packages and enable corepack (#284)
- [providers] Add first-class LM Studio provider (#286)
- [agents] Enrich spawn_agent presets with identity, policies, memory (#271)
- [web] Show running version at bottom of identity settings
- [channels] Channel architecture phase 5, contract suites, and observability baseline (#289)
- [ci] Add release dry-run mode
- [browser] Add container_host for Docker-in-Docker connectivity (#300)
- [ios] Auto-discover server identity and show emojis (#297)
- [website] Migrate cloudflare website into monorepo (#302)
- [local-llm] Allow arbitrary HuggingFace model IDs for MLX models
- [web,tools] AOT WASM pre-compilation and Shiki CDN loading
- [cli] Remove wasm from default features to reduce memory
- [gateway] Make provider discovery startup non-blocking
- [monitoring] Track memory history and improve local-llm memory reporting (#325)
- [ios] Add local llama cpp memory field to GraphQL schema
- [providers] Include reasoning fields for kimi models (#323)
- [chat] Tabs to filter chats between sessions and cron (#338)
- [oauth] Support pasted callback URL fallback (#365)
- [providers] Add reasoning effort support for models with extended thinking (#363)
- [providers] Add Responses API support to GitHub Copilot provider (#393)
- [release] Migrate to date-based versioning (YYYYMMDD.NN) (#394)
- Support secret remote MCP URLs and headers (#416)
- [docker] Support generic provider env bootstrap (#401)
- [skills] Support safe agent-written sidecar files (#413)
- [mcp] Add custom display names for MCP servers
- [mcp] Add displayName to iOS GraphQL schema
- [skills] Gate installer behind install feature
- [gateway] Embedded web chat UI at root endpoint
- [gateway] Add services, pairing, expanded methods and auth
- [agents] Add LLM chat with streaming, multi-provider support and feature flags
- [gateway] Add Tailwind-based chat UI with dark/light theme
- [config] Add multi-format config file with provider enable/disable support
- [gateway] Add model selector and WebSocket auto-reconnect
- [oauth] Add OpenAI Codex OAuth provider and reusable OAuth infrastructure
- [tools] Add LLM code execution with agent loop, tool calling, and security layers
- [agents] Add debug logging and end-to-end exec tool test
- [agents] Text-based tool calling fallback for non-native providers
- [gateway] Log user message on chat.send
- [memory] Implement memory management system with hybrid search
- [tools] Wire approval gating into exec tool with UI
- [brew] Add Homebrew formula for tap-based installation
- [website] Add static site and roadmap for moltis features
- [website] Rewrite with Tailwind CSS, Inter/JetBrains fonts, and polish
- [packaging] Add Debian package builds for amd64 and arm64
- [packaging] Add Arch Linux package builds for x86_64 and aarch64
- [packaging] Add RPM, Flatpak, Snap, AppImage, Nix, and Homebrew packaging
- [agents] Register all Codex models in provider registry
- [gateway] Structured error handling, exec cards, and approval UI
- [gateway] Add provider management UI with multi-model support
- [gateway] Persist API keys and add session management
- [gateway] Add session sidebar UI and fix session management
- [gateway] Route chat events by session key, add unread dots and thinking restore
- [agents] Add missing LLM providers and GitHub Copilot OAuth
- [gateway] Add session search with autocomplete, scroll-to and highlight
- [gateway] Include model and provider name in chat final events
- [claude] Save plans and sessions to prompts/ via hooks
- [gateway] Multi-page SPA with nav panel, crons page and methods page
- [cron] Wire cron callbacks and register CronTool for LLM use
- [cron] Implement production-grade cron scheduling system
- [projects] Add project management with context loading and session binding
- [gateway] Searchable model selector, per-session model, chat history
- [gateway] Persist model/provider in chat history and style model footer
- [gateway] Add token usage display per-message and per-session
- [gateway] Move model selector to chat page, add providers to nav panel
- [projects,sessions] Migrate project and session metadata storage to SQLite
- [gateway] Reorganize navigation, move providers to dedicated page
- [sandbox] Per-session sandbox toggle with sandbox-on-by-default
- [gateway] Show LLM thinking text, persist token usage, fix chat scroll
- [gateway] Add slash commands (/clear, /compact, /context) with autocomplete
- [context] Display sandbox details in /context command
- [providers] Add Kimi Code OAuth (device flow) provider
- [gateway] Add Channels navigation page
- [gateway] Move enabled toggle to last column in cron job table
- [telegram] Add username allowlist matching and message log store
- [compact] Auto-compact on context limit, use session provider, show summary card
- [worktree] Implement workspace worktree lifecycle features
- [ui] Project selector combo in chat header with session filtering
- [telegram] Per-channel sessions, slash commands, and default model config
- [gateway] Add logs/forensic page with real-time streaming and persistence
- [telegram] Command autocomplete, /new session, /sessions with inline keyboard
- [gateway] Live session updates, channel icons, and active session indicator
- [gateway] Add amber ping dot indicator for active sessions
- [gateway] UI improvements and provider/context enhancements
- [skills] Add agent skills system crate with discovery, registry, and CLI
- [gateway] Add Skills navigation page to web UI
- [skills] Repository-based skills with per-skill enable/disable
- [skills] Accept GitHub URLs in skill source input
- [gateway] Add native HTTPS/TLS support behind `tls` cargo feature
- [gateway] Hybrid asset serving with filesystem dev and embedded release
- [sandbox] Configurable images, on-demand caching, Apple Container auto-detection
- [onboarding] Replace wizard with inline identity editing in Settings
- [plugins] Add plugins crate with format adapters, install, and chat integration
- [gateway] Add Images page for managing sandbox container images
- [telegram] Add /model, /sandbox commands and improve /context display
- [gateway] Add setup code at startup and configurable config/data dirs
- [gateway] Clean auth state separation, gon pattern, and identity improvements
- [env] Add write-only environment variables with sandbox injection
- [security] Wrap secrets in secrecy::Secret<String> to prevent leaks
- [hooks] Add hook dispatch system with native and shell handlers
- [hooks] Add hook discovery, eligibility, metadata, CLI commands, and bundled hooks
- [memory] Wire memory system into gateway with tools, compaction, and session hooks
- [memory] Add embedding cache, local GGUF, fallback chain, batch API, file watcher, and pre-compaction flush
- [tools] Add web_search and web_fetch agent tools
- [sandbox] Add tracing to Apple Container and exec tool lifecycle
- [ui] Show container name in /context sandbox section
- [web] Forward client Accept-Language to web_fetch and web_search
- [sandbox] Auto-provision curl, python3, nodejs, npm in containers
- [sandbox] Expand default packages inspired by GitHub runner images
- [sandbox] Apple Container pre-built images, CLI commands, default config
- [sandbox] Expand packages from GitHub runner images, use Secret for web search keys
- [hooks] Wire all 15 hook events and add examples
- [chat] Add per-session run serialization to prevent history corruption
- [chat] Add configurable agent-level timeout enforcement
- [agents] Retry agent loop after compaction on context window overflow
- [tools] Add SpawnAgentTool for sub-agent / nested agent support
- [chat] Add message queue modes for concurrent send handling
- [agents] Sanitize tool results before appending to LLM message history
- [agents] Execute tool calls concurrently with join_all
- [mcp] Add MCP client support with discovery UI
- [gateway] Add nav sidebar count badges and MCP UI improvements
- [mcp] MCP context in chat, duplicate name handling, and misc improvements
- [mcp] Add McpTransport and McpClientTrait trait abstractions
- [mcp] Wire MCP tool bridges into agent ToolRegistry
- [mcp] SSE transport, health polling, auto-restart, and edit config
- [gateway] Add tailscale serve/funnel management, UI consistency overhaul, and HTTP/2 support
- [memory] Log status with DB size after initial sync
- [tailscale] Add Start Tailscale button when daemon is not running
- [channels] Assign default model to new telegram sessions
- [agents] Add model failover with per-provider circuit breakers
- [gateway] Add report an issue link to nav sidebar
- [config] Support MOLTIS_* env var overrides for all config fields
- [cron] Add heartbeat feature with persistent run history
- [cli] Add cargo-binstall support for binary installation
- Add Homebrew tap and auto-update workflow
- Generate random port on first run and make gateway the default command
- [ci] Add multi-arch Docker build workflow
- [agents] Enable streaming responses with tool support
- [metrics] Add Prometheus metrics with feature-gated support
- [metrics] Expand metrics to all crates with tracing feature
- [metrics] Add provider alias support for metrics differentiation
- [metrics] Add SQLite persistence and per-provider charts
- [db] Add sqlx migrations with per-crate ownership
- [local-llm] Add local LLM provider with GGUF/MLX backend selection
- [local-llm] Add MLX models and filter by backend
- [local-llm] Add HuggingFace search and custom model support
- [agents] Add unified local-llm provider with pluggable backends
- [ui] Add auto-search with debounce for HuggingFace search
- [ui] Show chat-only mode notice when selecting model without tools
- [ui] Show all configured models in providers page
- [providers] Add per-model removal and disable support
- [local-llm] Add tracing and metrics to GGUF provider
- [cli] Add database management commands (db reset, clear, migrate)
- [telegram] Auto-disable channel when another bot instance is running
- Add install script and update URLs to moltis.org
- Add Pi-inspired features — skill state, self-extension, branching, hot-reload
- [auth] Add scope support to API keys
- Typed ToolSource, per-session MCP toggle, debug panel convergence, docs & changelog
- [gateway] Add Fork button to chat header
- [gateway] Show warning banner when running on non-main git branch
- [gateway] Add mobile PWA support with push notifications (#40)
- [tls] Default HTTP redirect port to gateway port + 1 (#49)
- [memory] Add QMD backend support, citations, session export, and LLM reranking (#27)
- [skills] Show confirmation hint after skill creation/update
- [skills] Add skill editing and forking from the web UI
- [skills] Show confirmation hint after skill creation/update
- [skills] Add skill editing and forking from the web UI
- [mcp] Make request timeout configurable
- [agents] Lazy tool registry with tool_search meta-tool
- [local-llm] Add opt-in vulkan gguf support
- [providers] Add MiniMax M2.7 and missing M2.1 highspeed models
- [prompt] Stabilize system prompt for local LLM KV cache
- [scripts] Skip local-validate.sh when commit already passed
- [docker] Add Node.js/npm to Docker image for MCP servers
- [ci] Add pre_release option to release workflow


### Changed
- [website] Extract inline CSS to separate styles.css file
- [gateway] Split monolithic app.js into 24 ES modules
- [gateway] Extract inline JS styles to CSS and add message dedup
- [gateway] Preact skills page, REST APIs, biome linting, perf fixes
- [gateway] Reduce cognitive complexity in JS modules and add biome CI
- [gateway] Faster log loading with memory-first reads and batch rendering
- Simplify auth, onboarding, and settings code
- Simplify hooks and memory code
- [env] Wrap env var values in secrecy::Secret
- Remove all unsafe code and add workspace-wide deny(unsafe_code)
- Keep API keys wrapped in Secret<String> through provider construction
- [agents] Simplify tool result sanitization
- [mcp] Remove dead code and deduplicate config parsing
- [memory] Skip redundant work on sync restart
- [gateway] Use inline script for identity instead of server-side HTML replace
- [api] Split /api/skills and /api/plugins into separate endpoints
- [ui] Use CSS classes for tailscale status bar, single line layout
- [ui] Rename ts-status-bar to generic info-bar classes and fix image page font size
- [tailscale] Remove Start Tailscale button and up endpoint
- [ui] Move session rename/delete to chat header and clean up
- Remove unsafe set_var in detect.rs tests, resolve merge conflicts
- [gateway] Reorder nav items alphabetically with Chat first
- [ci] Extract signing logic into composite action
- [ui] Move install hint markup to HTML template
- [ui] Move model notice card HTML to template
- Rename crate moltis-cli to moltis
- [deps] Centralize all dependency versions in workspace root
- [ci] Run lightweight lint jobs on GitHub-hosted runners
- [ci] Restore Docker jobs to release workflow, remove docker.yml
- [plugins] Replace hand-rolled date arithmetic with time crate
- [gateway] Use time::Duration::days in tls expiry check
- Use moltis_config::data_dir() for all path resolution
- [browser] Add browser detection and simplify session handling
- [settings] Rename Tools to Configuration and edit full config
- [browser] Use typed structs for OpenAI tool schemas
- [local-llm] Add modular ResponseParser trait for output parsing
- [browser] Use unified sandbox infrastructure for browser containers
- [gateway] Add defense-in-depth auth checks and DRY server.rs
- [browser] Send execution mode from server in tool_call_start
- [channels] Use ChannelType enum instead of string matching
- [browser] Sandbox mode follows session, fix tall screenshot lightbox
- [ci] Move local status check logic into script
- [ci] Avoid blocking lint and test on local zizmor
- [security] Use gitoxide metadata and improve trust UX
- [scripts] Rename local-validate-pr.sh to local-validate.sh
- [chat] Simplify runtime host+sandbox prompt context
- [sandbox] Remove redundant mkdir exec from ensure_ready
- Simplify branch review fixes across UI, gateway, and config
- [gateway] Use typed structs for chat broadcast payloads
- [ui] Rename /images and /api/images routes to /sandboxes
- [gateway] Consolidate GatewayState per-field RwLocks into single RwLock<GatewayInner>
- [mcp] Consolidate McpManager per-field RwLocks into single RwLock<McpManagerInner>
- [gateway] Replace project select with custom combo dropdown
- [gateway] Unify auth into single check_auth() gate
- [browser] Make stale cleanup path expression-based
- [ci] Move E2E job into CI workflow
- [build] Enable thin LTO and binary stripping in release profile
- [gateway] Extract shared voice, identity, and channel utils
- [tools] Resolve browser sandbox mode from SandboxRouter directly
- [gateway] Reorder onboarding screens
- [session] Typed params for patch and voice_generate (#131)
- [gateway] Remove standalone /crons route, use /settings/crons
- [agents] Simplify prompt builder and runtime context
- [prompt] Compact prompt sections and add server runtime time
- [web] Extract web UI into dedicated moltis-web crate
- [identity] Consolidate creature+vibe into single theme field
- [gateway] Extract moltis-service-traits crate (Phase 0)
- [gateway] Extract moltis-tls and moltis-tailscale crates (Phase 1)
- [gateway] Extract moltis-auth crate (Phase 2)
- [gateway] Extract moltis-provider-setup crate (Phase 3)
- [gateway] Extract moltis-chat crate (Phase 4)
- [web] Move share_render.rs from gateway to moltis-web (Phase 1c)
- [providers] Extract provider implementations into new crate
- [errors] Move crates to typed thiserror enums (#226)
- [tools] Replace anyhow bridge with crate::Result for internal APIs (#257)
- [ffi] Tighten unsafe_code allowances
- [channels] Registry-driven dispatch for cheap new channels (#277)
- [gateway] Fetch updates from releases manifest instead of GitHub API
- [web] Move settings nav icons from JS to CSS
- Externalize web/wasm assets and reduce memory footprint (#321)
- [web] Move chat history hydration to paged HTTP
- [web] Paginate sessions and auto-load older history
- [tools] Split sandbox.rs into sandbox/ module directory
- [website] Extract inline CSS to separate styles.css file
- [gateway] Split monolithic app.js into 24 ES modules
- [gateway] Extract inline JS styles to CSS and add message dedup
- [gateway] Preact skills page, REST APIs, biome linting, perf fixes
- [gateway] Reduce cognitive complexity in JS modules and add biome CI
- [gateway] Faster log loading with memory-first reads and batch rendering
- Simplify auth, onboarding, and settings code
- Simplify hooks and memory code
- [env] Wrap env var values in secrecy::Secret
- Remove all unsafe code and add workspace-wide deny(unsafe_code)
- Keep API keys wrapped in Secret<String> through provider construction
- [agents] Simplify tool result sanitization
- [mcp] Remove dead code and deduplicate config parsing
- [memory] Skip redundant work on sync restart
- [gateway] Use inline script for identity instead of server-side HTML replace
- [api] Split /api/skills and /api/plugins into separate endpoints
- [ui] Use CSS classes for tailscale status bar, single line layout
- [ui] Rename ts-status-bar to generic info-bar classes and fix image page font size
- [tailscale] Remove Start Tailscale button and up endpoint
- [ui] Move session rename/delete to chat header and clean up
- Remove unsafe set_var in detect.rs tests, resolve merge conflicts
- [gateway] Reorder nav items alphabetically with Chat first
- [ci] Extract signing logic into composite action
- [ui] Move install hint markup to HTML template
- [ui] Move model notice card HTML to template
- Rename crate moltis-cli to moltis
- [media] Replace manual MIME lookup with mime_guess crate
- [auth] Extract GatewayState::is_secure() to centralise cookie Secure logic
- [browser] Use match instead of nested if/else for dir creation
- [httpd] Extract moltis-httpd crate as HTTP transport facade
- [ci] Restore release.yml clippy runner to self-hosted
- [tools] Unify rescue helper, add tracing, exclude action-level keys
- [docker] Split runtime installs into separate layers
- [dev] Deduplicate release-preflight by delegating to lint


### Removed
- Merge branch 'main' into claude/remove-unsafe-code-ehZlQ
- [ui] Remove decimal digits from memory display
- [ci] Add latest Docker tag on tag pushes, remove unused branch tag
- [ci] Remove template-expanded gate debug output
- [gateway] Use danger button for session delete
- [gateway] Size-match delete button with fork and add danger style
- [gateway] Match delete button size to fork button
- [browser] Remove linux-only unused mut in stale cleanup
- [codspeed] Remove noisy n=1 session_store_list case
- Remove cargo-binstall install path
- [browser] Remove needless return in match arm
- [gateway] Remove unnecessary path qualification flagged by clippy
- [cron] Remove oneOf from tool schema for OpenAI responses
- [readme] Drop logo icon to save vertical space
- [readme] Restore 64px favicon, remove link from title
- Remove trailing whitespace in validate.rs
- [cli] Remove binstall metadata and fix feature indentation
- [sandbox] Remove redundant error conversions
- [web] Remove nested onboarding scroll and restore settings nav icons
- [web] Declutter chat controls and fix dropdown positioning
- Merge branch 'main' into claude/remove-unsafe-code-ehZlQ
- [local-llm] Remove unused gguf runtime helper


### Fixed
- [oauth] Use correct OpenAI client_id, add config layer and tests
- [agents] Prefer tool-capable provider when tools are registered
- [agents] Register builtin providers before genai to enable tool calling
- [gateway] Auto-focus chat input on page load
- [sessions] Add missing sandbox_enabled column migration, stop swallowing errors
- [gateway] Move sandbox button next to model select, fix default state
- Resolve all clippy warnings (collapsible_if, map_flatten, or_default, unused imports)
- [gateway] Open nav panel by default
- [gateway] Add session icons and replace spinner in-place
- [gateway] Add missing renderCompactCard export to page-chat.js
- [gateway] Fix routing, styling, and layout issues in JS modules
- [telegram] Remove channel prefix from user messages and log LLM responses
- [ci] Fix biome version and worktree test default branch
- [ci] Restore biome version pin and fix cargo fmt
- Update CLAUDE.md pre-commit checks and fix clippy warnings
- [ci] Update setup-biome to v2.7.0 and fix sort_by_key clippy lint
- [tests] Configure git user in worktree test setup for CI
- Replace sort_by with sort_by_key in project store
- [assets] Fix biome formatting and empty catch block in sandbox.js
- [ui] Match settings nav item height and style to main navigation
- [security] Wrap API keys in Secret<String> for memory subsystem
- Use RecommendedCache for cross-platform watcher, sort_by_key for clippy
- [hooks] Ignore broken pipe on stdin write when child doesn't read it
- [ui] Show URL and query in web tool call cards
- [sandbox] Restart stopped Apple containers in ensure_ready
- [sandbox] Pass sleep infinity to Apple Container run
- [sandbox] Promote exec routing and container logs to info level
- [sandbox] Handle Apple Container inspect returning empty for nonexistent containers
- [sandbox] Default working_dir to "/" when running inside container
- [security] Reject cross-origin WebSocket upgrades (CSWSH protection)
- [sandbox] Inject env vars in Apple Container backend
- [exec] Redact env var values from command output
- [exec] Redact base64 and hex encoded env var values from output
- [config] Use correct defaults for ToolsConfig when not deserialized
- [gateway] Nav badge counts and cron page instant load
- [memory] Write memory files to data dir instead of cwd
- [gateway] Eliminate identity flash on page load
- [ui] Use consistent button class for skills Remove button
- [ui] Fix skills Disable button style and add error feedback
- [skills] Route disable to correct RPC for plugin-sourced skills
- [ui] Use inline style for tailscale status green dot
- [ui] Use status-dot connected class for tailscale green dot
- [ui] Replace Tailwind arbitrary classes with inline styles in tailscale templates
- [ui] Make tailscale status bar fit-content width
- [ui] Make info-bar full width to match warning/error divs
- [ui] Constrain info-bar to max-width 600px matching alert divs
- [ui] Add alert-info-text to shared alert base styles
- [ui] Add btn-row and btn-row-mt CSS classes for button spacing
- [ui] Space cancel/create buttons apart and normalize height
- [ui] Improve tailscale loading message to set expectations
- [tailscale] Open Tailscale.app on macOS instead of running tailscale up
- [ui] Preserve funnel security warning when rebuilding tailscale DOM
- [ui] Use alert-warning-text style for funnel auth warning
- [ui] Replace auth text with green button linking to security settings
- [ui] Move auth button below the funnel security warning panel
- [ui] Remove @layer components wrapper to fix nav specificity
- [ui] Update session list after /clear and select next on delete
- [ui] Format environment variable timestamps with luxon
- [gateway] Stop echoing web UI messages to Telegram channel
- Collapse nested if statements in config loader
- [ci] Fix package build failures
- [ci] Correct cargo-deb assets order
- [local-llm] Detect available package managers for MLX install
- [local-llm] Detect mlx-lm installed via brew
- [ui] Show install commands on separate lines
- [local-llm] Rename provider from local-gguf to local-llm
- [local-llm] Fix HuggingFace API response parsing
- [ui] Show searching state on HuggingFace search button
- [ui] Close modal and prevent multiple clicks on HF model selection
- [chat] Remove per-message tools warning broadcast
- [ui] Remove scroll from local model list
- [ci] Install cmake for llama.cpp build and fix Biome errors
- [ci] Fix local-llm build in CI
- [migrations] Use set_ignore_missing to allow multi-crate migrations
- [cli] Run gateway migrations from db migrate command
- [telegram] Don't delete channel from database on conflict
- [gateway] Load images nav count async to avoid blocking page serve
- [gateway] Replace branch icon, fix fork UX issues
- Streaming tool calls, skill directory, skills page UX
- [skills] Handle disable for personal/project skills
- [skills] Use app modal for delete confirmation instead of system confirm()
- [skills] Danger button on delete modal, fix disable routing for SkillsService
- [ui] Use decimal units (MB, GB) for memory display
- [ui] Strengthen skills security warning
- [ui] Prevent branch banner error from blanking the page
- [ci] Deploy docs to gh-pages branch instead of using deploy-pages action
- [ci] Use GitHub-hosted runners for PRs to protect self-hosted runners
- [ci] Add persist-credentials: false to docs workflow checkout
- [ci] Use macOS runners for apple-darwin builds
- [ci] Use project-local cargo config for cross-compilation
- [ci] Fix all build failures in release workflow
- [ci] Migrate release builds to GitHub-hosted runners
- [ci] Build Docker images natively per arch instead of QEMU emulation
- [ci] Only build Docker images on release tags, not main pushes
- [deploy] Persist config directory on cloud providers
- [ci] Use ubuntu-24.04-arm for Linux ARM jobs
- [ci] Correct cargo-sbom CycloneDX format flag
- [ci] Derive release package versions from git tags
- [ui] Compact memory info display in header bar
- [docker] Add runtime libgomp and image smoke test
- [gateway] Route setup through onboarding and stabilize hosted WS auth
- [ui] Disable sandbox controls when no backend is available
- [ci] Remove gh dependency from homebrew workflow
- [ci] Run homebrew update after release assets
- [ci] Avoid invalid secrets context in release workflow
- [hooks] Show GitHub source link for built-in hooks instead of editor
- [hooks] Use checked_div for avg latency calculation
- [gateway] Keep telegram typing active until reply send
- [sandbox] Skip prebuild when disabled and require daemon access
- [gateway] Use instance callback URL for OAuth flows
- [ci] Apply nightly rustfmt and clarify formatting checklist
- [ci] Add CUDA static libs and linker paths
- [ci] Map Debian CUDA libs into CUDA root for static link
- [browser] Register BrowserTool in tool registry
- [agents] Correctly pass tools through ProviderChain and detect tool result failures
- [browser] Include install instructions when browser launch fails
- [browser] Check macOS app bundles before PATH for browser detection
- [browser] Improve tool description with explicit examples
- [agents] Add strict mode to OpenAI-compatible tool schemas
- [browser] Default to navigate action when only url is provided
- [chat] Preserve message order when tool calls interleave with text
- [chat] Hide schema validation error cards from UI
- [chat] Show validation errors as muted informational cards
- [browser] Recursively add additionalProperties to nested schemas
- [browser] Ensure all properties in required for OpenAI strict mode
- [browser] Use shared to_openai_tools in openai_codex provider
- [browser] Add to_responses_api_tools for Codex API format
- [browser] Handle objects without properties in strict mode
- [browser] Use data URI format for screenshots
- [browser] Handle both raw base64 and data URI in screenshot display
- [local-llm] Reject MLX models in GGUF loader with helpful error
- [local-llm] Route MLX models to MLX backend via local_llm provider
- [local-llm] Allow HuggingFace repo IDs for MLX models
- [local-llm] Parse mlx_lm CLI output correctly
- [gateway] Use LocalLlmProvider for UI-added models
- [local-llm] Check legacy registry for MLX models
- [local-llm] Use local cache for MLX models instead of HF repo path
- [local-llm] Multiple fixes for model download and registration
- [browser] Improve container detection and settings error handling
- [lint] Resolve biome errors for CI
- [browser] Update deprecated rand API calls
- [browser] Improve tool description to trigger on 'browse' keyword
- [browser] Don't write screenshot files to disk
- [sandbox] Resolve race condition where sandbox shows disabled at startup
- [gateway] Add missing config and metrics routes to push-notifications build
- [ui] Remove duplicate browser info from result area
- [browser] Retry on dead WebSocket connections
- [browser] Validate URLs to reject LLM garbage
- [browser] Improve screenshot handling and suppress chromiumoxide logs
- [browser] Ensure viewport is applied to pages and add debug logging
- [browser] Wait for Chrome to be ready before connecting
- [browser] Auto-track session_id to prevent pool exhaustion
- [browser] Use navigation timeout for sandboxed browser connections
- [telegram] Re-send typing indicator after tool status message
- [ci] Pin sccache action to commit SHA
- [ci] Publish local validation statuses to PR head repo
- [ci] Fallback to gh auth token for local status publishing
- [ci] Clean stale llama cmake dirs in local validator
- [ci] Use non-CUDA local validation defaults on macOS
- [ci] Wait for local statuses before failing PR checks
- [ci] Correct parallel local check PID handling
- [ci] Stabilize local validator output and show run URL
- [security] Harden skill execution and web asset safety
- [security] Require trust before enabling installed skills
- [security] Harden tarball extraction and pin install provenance
- [security] Require re-trust when skill source drifts
- [security] Block suspicious dependency install commands
- [security] Recover orphaned installs and protect seed skills
- [skills] Seed template and quiet invalid skill warnings
- [memory] Align indexing scope with openclaw defaults
- [memory] Coalesce stale-index cleanup logs
- [validation] Require clean tree for local PR status
- [agents] Replace serde_json::Value with typed ChatMessage in LlmProvider
- [scripts] Add lockfile check and enforce clean worktree in all modes
- [clippy] Address nested-if and duplicate branch warnings
- [ui] Remove redundant heartbeat token-saver panel
- [chat] Keep skill management tools available at runtime
- [gateway] Support configured GitHub repo for update checks
- [update] Disable checks when repo URL is unset
- [ci] Pin workflow actions to commit SHAs
- [ci] Expose CUDA compat libs for test runtime
- [concurrency] Remove Mutex<()> sentinels and harden exec working dir
- [startup] Ensure data dir exists and isolate exec tests
- [startup] Fail fast when workspace dirs cannot be created
- [exec] Satisfy clippy lints in sandbox detection tests
- Add voice schema to build_schema_map for validation
- [voice] Auto-enable provider and share ElevenLabs key between TTS/STT
- [voice] Align transcribing indicator right and fix recording delay
- [voice-ui] Align mic state timing and preserve settings scroll
- [chat-ui] Show reply medium badge in assistant footer
- [sandbox] Create /home/sandbox directory in generated images
- [sandbox] Create /home/sandbox at container startup via exec
- [sandbox] Default working directory to /home/sandbox
- [sandbox] Keep apt index in pre-built images
- [gateway] Remove no-op or_else closure flagged by clippy
- [ui] Bust Safari cache in dev mode and fix detected providers border
- [tools] Handle empty session_name in process tool start action
- [gateway] Validate config before restart to prevent crash loops
- [gateway] Use YAML list for allowed-tools in seeded tmux skill
- [gateway] Auto-enable ElevenLabs counterpart in onboarding voice test
- [gateway] Persist empty assistant messages for LLM history coherence
- [gateway] Remove separate voice-generating indicator during TTS
- [gateway] Enable TTS counterpart when saving ElevenLabs/Google STT key
- [gateway] Reconstruct tool messages in full context view
- [gateway] Fix queued message detection and ordering
- [gateway] Move queued messages to dedicated bottom tray
- [gateway] Clear queued tray on session switch and add debug logs
- [gateway] Never attach model footer or timestamp to user messages
- [gateway] Prevent multi-move race in queued message tray
- [gateway] Defer user message persist until after semaphore
- [gateway] Move queued messages only after response is rendered
- [tools] Fall back to DuckDuckGo when web_search API key is missing
- [onboarding] Skip finish screen and redirect to chat directly
- [ui] Rename Images nav item to Sandboxes
- [ui] Revert API routes to /api/images, remove "live" status text
- [cron] Skip heartbeat when no prompt configured, fix duplicate runs
- [tools] Collapse nested if in location tool (clippy)
- [gateway] Collapse nested if in ws timezone handling (clippy)
- [agents] Make openai-codex token refresh async to prevent runtime panic
- [gateway] Ensure session row exists before setting channel binding
- Sync Cargo.lock with workspace version 0.3.1
- [gateway] Use is_none_or instead of map_or for clippy
- [sessions] Use blocking file lock to prevent concurrent write failures
- Auto-sync Cargo.lock in local-validate.sh
- [docker] Resolve deployment errors on DigitalOcean Docker containers
- [install] Add missing -1 revision to .deb filename
- [gateway] Show real IP in banner when bound to 0.0.0.0
- [oauth] Downgrade missing token file log from warn to debug
- [oauth] Downgrade routine token store logs to debug
- [providers] Show model selector after OAuth in onboarding
- [onboarding] Improve scroll padding in channel and summary steps
- [gateway] Use route-specific body limit for upload endpoint
- [gateway] Reduce global request body limit to 2 MB
- [agents] Pass multimodal content to LLM providers instead of stripping images
- [gateway] Unlock audio autoplay via AudioContext on user gesture
- [tools] Instruct LLM to use lat/lon for searches, not place names
- [tools] Use place names in map links and skip Brave when unconfigured
- [tools] Block DuckDuckGo fallback for 1h after CAPTCHA
- [voice] Add autoplay debug logging, stop audio on mic, Esc cancels
- [ci] Move coverage job into CI workflow
- [ci] Slim down coverage job to avoid disk space exhaustion
- [ci] Exclude moltis-tools from coverage and drop sccache
- Collapse nested if blocks per clippy
- Collapse nested if blocks in provider_setup per clippy
- [tools] Use if-let instead of is_some/unwrap in map test
- Deny unwrap/expect in production code via clippy lints
- [gateway] Mark boot_time test as ignored for CI coverage
- [gateway] Fix session project filter and add clear all sessions
- [gateway] Prevent layout shift from active session border
- [gateway] Collapse nested if per clippy collapsible_if lint
- [ci] Consolidate release uploads into single job
- [gateway] Reactive session badge with optimistic updates
- [gateway] Bump session badge for channel messages
- [gateway] Stop auto-switching session on channel messages
- [gateway] Broadcast session_cleared so /clear from channels syncs web UI
- [voice] Add media-src to CSP for TTS playback and improve voice diagnostics
- [auth] Clear setup_complete when last passkey is removed
- [gateway] Bump session badge on every WS event
- [gateway] Shrink passkey action buttons and hide danger zone when no auth configured
- [ci] Align release clippy with nightly flags and fix test lints
- [browser] Add lifecycle management for browser containers (#88)
- [auth] Break onboarding redirect loop for non-local connections
- [settings] Disable browser password autofill for env fields
- [gateway] Reconnect WebSocket after remote onboarding auth
- [docker] Bind 0.0.0.0 and expose CA cert port for TLS setup
- [ci] Gate release builds on E2E tests and fix onboarding-auth flake
- [ci] Gate release builds on E2E tests and fix onboarding-auth regex
- [docker] Add chromium and demote codex token warning to debug
- [scripts] Skip --all-features for clippy on macOS without CUDA
- [scripts] Allow dirty working tree in local-only validation
- [exec] Use host data dir when no container runtime is available
- [onboarding] Update low memory warning message
- [ui] Rename "provider" to "LLM" in navigation and headings
- [gateway] Preserve chat stream event order
- [auth] Enforce auth after passkey/password setup (#93)
- [e2e] Make NoopChatService::clear() succeed and handle no-provider errors in tests
- [config] Resolve lock reentrancy deadlock in update_config
- [onboarding] Defer WS connection until auth completes
- [config] Prevent SOUL.md re-seeding after explicit clear
- [onboarding] Update test for save_soul empty-file behavior
- [ci] Restore node cache hardening and gate on zizmor
- [onboarding] Keep voice step visible in auth-gated flow
- [gateway] Stabilize anthropic onboarding model selection
- Pin deploy templates to explicit versions and auto-update on release (#98)
- [ci] Sync workspace version in Docker build
- [gateway] Update session preview immediately after clear
- [onboarding] Persist selected llm models on continue
- [agents] Cap tool-call IDs to OpenAI 40-char limit
- [config] Isolate stale-key test from env overrides
- [gateway] Include saveProviderKey extraction in provider-validation
- [e2e] Handle pre-configured providers in onboarding tests
- [gateway,tools] Fix sessions.patch camelCase, streaming artifacts, and host working dir
- [tools] Gate macOS-only sandbox helpers with cfg to fix dead-code errors on Linux CI
- [tools] Browser tool falls back to host when no container runtime
- [agents] Add memory_save and memory-anchor hints to system prompt
- [agents] Guide memory_save toward topic files to keep context lean
- [gateway] Fix disabled button rendering and add validation progress hints
- [gateway] Restore voicePending state on session reload
- [tools] Is_sandboxed() returns false when no container runtime exists
- [gateway] Constrain sandbox warnings to max-w-form width
- [memory] Normalize embeddings base_url and add disable_rag mode (#147)
- [gateway] Hide crons/heartbeat submenu when embedded in settings
- [cron] Use data_dir for job storage instead of hardcoded ~/.clawdbot/cron
- [plugins] Set working_dir on shell hooks so relative paths resolve correctly
- [gateway] Improve onboarding and provider validation flows
- [gateway] Allow openrouter model discovery without model id
- [chat] Retry provider 429s across web and channel flows (#149)
- [gateway] Queue channel replies per message with followup default (#150)
- [onboarding] Restore saved model selection and stabilize Anthropic e2e
- [tools] Harden apple container sandbox workdir bootstrap
- [cron] Normalize shorthand cron tool inputs
- [e2e] Avoid response listener race in sandboxes test
- [chat] Compact based on next-request context pressure (#166)
- [agents] Require explicit /sh for forced exec fallback (#161)
- [map] Support show_map multipoints and grouped map links (#168)
- [config] Make agent loop limit configurable and sync docs
- [skills] Support marketplace skills[] repositories
- [onboarding] Persist browser timezone and expose prompt today
- [web-search] Make DuckDuckGo fallback opt-in
- [gateway] Align show_map listing ratings to the right
- [prompt] Clarify sandbox vs data_dir path semantics
- [agents] Append runtime datetime at prompt tail
- [web-search] Load runtime env keys and robustify brave parsing
- [prompt] Preserve datetime tail ordering and add profile plan
- [mcp] Strip internal metadata from MCP tool call arguments
- [crons] Fix modal default validation and form reset on schedule change (#181)
- [gateway] Deduplicate voice replies on Telegram channels (#173)
- [browser] Centralize stale CDP connection detection (#172)
- [browser] Enable profile persistence on Apple Container
- [terminal] Force tmux window resize on client viewport change
- [telegram] Default DM policy to allowlist for new channels
- Update expired Discord invite link
- [telegram] Dispatch voice messages with empty transcription to chat
- [gateway] Broadcast voice audio filename over WebSocket for real-time playback
- [voice] Improve voice reply prompt and audio player placement
- [voice] Deliver TTS audio even when text was streamed
- [tools] Don't register web_search tool when API key is missing
- [agents] Don't emit regular text as ThinkingText when alongside tool calls
- [gateway,telegram,voice] Correct web UI stuck on thinking dots, table rendering, and TTS
- [telegram] Use vertical card layout for wide tables, deliver logbook for streamed voice
- [telegram] Send activity logbook as raw HTML, not markdown
- [e2e] Update share page nonce assertion after script externalization
- [cli] Skip jemalloc on Windows
- [gateway] Redirect plain HTTP to HTTPS on TLS port and optimize metrics SQLite
- [agents] Register DeepSeek via OpenAI-compatible provider for tool support
- [gateway] Sync cron modal signals when editing a job
- [providers] Prefer subscription OAuth and expose Codex onboarding
- [gateway] Surface insufficient_quota without retries
- [voice] Reuse OpenAI LLM API key for TTS and STT providers (#198)
- [ci] Only tag Docker image as latest for highest semver release (#211)
- [e2e] Fix provider selector and onboarding voice step tests
- [e2e] Use exact text match for onboarding provider row selection
- [graphql] Fix session id type, missing ok fields, chat routing, and add session.active query (#218)
- [cron] Document delivery fields in tool description and schema (#213)
- Pass speaking_rate, pitch and speed from config to voice providers (#212)
- [cli] Forward feature flags to moltis-web optional dependency
- [gateway] Harden reverse-proxy tls and websocket handling (#230)
- [cli] Skip jemalloc on linux/aarch64 (#229)
- [gateway,web] Normalize service error mapping and nav expectations
- [e2e] Rebuild gateway when startup binary is stale
- [e2e] Harden session creation wait and anthro cargo env
- [e2e] Handle no-provider chat state in agent specs
- [e2e] Skip openclaw step in onboarding openai flow
- [import] Resolve OpenClaw workspace from config and cross-machine paths
- [openclaw-import] Collapse nested if chains for clippy
- [docker] Use modern Docker CLI in published image (#238)
- [web] Update qmd package references to @tobilu/qmd
- [ci] Exclude swift-bridge from coverage and increase E2E poll timeout
- [e2e] Match error code instead of localized message in full context tests
- [gateway] Remove duplicate /certs/ca.pem route in start_gateway
- [ci] Unblock clippy and macOS bridge wasm build
- [oauth] Make OpenAI Codex callback port 1455 work in Docker (#258)
- [tests] Accept restricted-host auto sandbox backend
- [ios] Add missing .none case in NWTXTRecord switch
- [web] Read DOM value in identity blur handlers to prevent stale closure skip
- [ios] Add cancel button to connection banner for unreachable servers
- [agents] Resolve clippy collapsible-if and expect-used lints
- [ci] Add WASM component build step to all release jobs
- [ci] Parallelize macOS app with release package builds
- [release] Ship v0.10.2 packaging fixes
- [sandbox] Make apple container keepalive portable (#269)
- [local-llm] Combine compile-time and runtime Metal detection
- [auth] Auto-detect new WebAuthn hosts and prompt passkey refresh (#268)
- [web] Replace rg with grep in changelog guard and deduplicate passkey status refresh
- [web] Lazy-load Shiki to prevent blocking page mount
- [web] Fix Shiki highlighter init failures in E2E tests
- [web] Make thinking stop button smaller with left spacing
- [chat] Surface error when LLM returns empty response with zero tokens
- [providers] Emit StreamEvent::Error on non-success finish_reason
- [e2e] Make sandboxes container tests deterministic
- [e2e] Replace remaining racy waitForResponse with route interceptors
- [mcp] Make optional MCP tool params nullable to prevent empty string errors (#283)
- [provider-setup] Reorder validation probes to prefer fast models (#280)
- [sandbox] Resolve host gateway IP for Podman < 5.0 (#287)
- [e2e] Fix flaky "deleting unmodified fork" test
- [ci] Stale lockfile, missing Tailwind in macOS job, OAuth e2e setup
- [ci] Use standalone Tailwind binary for macOS app job
- [e2e] Fix OAuth token-exchange failure test and add error-context capture
- [web] Auto-install node_modules in Tailwind build script
- [web] Retry openclaw onboarding scan until ws is ready
- [ci] Add Tailwind CSS build step to release workflow, Dockerfile, and snapcraft
- [e2e] Wait for session history render before DOM injection in chat-abort
- [ci] Harden tailwindcss cli downloads
- [swift-bridge] Stabilize gateway migration and stream tests
- [config] Support provider url alias for remote Ollama config (#299)
- [ci] Make release dry-run job conditions valid
- [providers] Use Ollama capabilities for tool support detection (#301)
- [scripts] Roll back heavy local validation parallelism
- [web] Skip npm install when TAILWINDCSS binary is provided
- [ci] Update website/releases.json on release
- [web] Add missing i18n button key for preferredModels
- [local-llm] Use sampler API for mlx-lm >= 0.20
- [gateway] Break redirect loop when onboarded but auth not configured (#310)
- [gateway] Reduce idle CPU from metrics loop and log broadcast feedback
- [gateway] Speed up startup by deferring tailscale and log scan
- [gateway] Improve browser warmup integration
- [scripts] Run local nextest with ci timeout profile
- [ci] Build macOS app arm64 in fast path
- [web] Move session history off websocket and cap payload size
- [web] Use combo select for session header selectors
- [web] Externalize SVG icons and restore empty-chat centering
- [web] Align e2e with controls modal and daily model refresh
- [ci] Stage wasm assets for cargo-deb packaging
- [packaging] Use cli-relative web assets in cargo-deb
- Install rustls CryptoProvider before channel startup (#336)
- [ci,tools] Unblock dependabot and support wasmtime 36
- [auth] Honor forwarded host for proxy session cookies
- [config] Include tmux in default sandbox packages
- [mdns] Use stable host label to avoid mDNSResponder conflict and double-.local suffix (#349)
- [web] Prevent Enter key from triggering actions during IME composition (#341)
- [biome] Update schema to 2.4.6 and move noUnusedExpressions to suspicious
- [ci] Update biome version to 2.4.6 in CI workflows
- [macos] Extract makeTextView to fix function body length lint violation
- [providers] Report compatible client_version for Codex model discovery (#359)
- [prompt] Omit sandbox/node info from runtime prompt when disabled (#362)
- [web] Allow deleting cron sessions from chat sidebar (#357)
- [chat] Skip duplicate text fallback when TTS disabled and voice streamed (#373)
- [web] Break redirect loop when accessing via Tailscale Serve (#356)
- Node WebSocket connection and UI connection string (#382)
- [config] Write IDENTITY.md and SOUL.md to agents/main/ instead of root (#384)
- [auth] Bypass auth for local API requests during onboarding (#386)
- [whatsapp] Sled persistence, graceful shutdown, and review fixes (#387)
- [cron] Add delay_ms to avoid LLM computing absolute timestamps (#377)
- [gateway] Retain proxy shutdown sender to prevent immediate proxy exit (#368)
- [agents] Include tool_result messages in LLM conversation history (#389)
- [sandbox] Auto-detect host data dir for docker-in-docker (#396)
- [chat] Compact the active channel session (#399)
- [providers] Strip stop tokens from MLX streaming output (#397)
- [config] Support legacy memory embedding keys (#400)
- [web] Address installation feedback from user testing (#398)
- [tools] Harden apple container bootstrap execs (#405)
- [telegram] Strip HTML tags from plain fallback (#404)
- [web] Clarify cron setup modal copy (#409)
- [web] Keep onboarding accessible after auth reset (#415)
- [web] Improve onboarding password autofill hints (#406)
- [chat] Persist aborted partial history (#418)
- [agents] Retry empty structured tool names (#410)
- [tools] Make sandbox cfg gates consistent for cross-platform CI
- [local-llm] Restore custom GGUF setup without restart (#417)
- [browser] Scope cached browser sessions per chat (#412)
- [browser] Align sandbox browserless timeout with pool lifecycle (#403)
- [providers] Keep minimax system messages in history
- [tools] Only expose exec node parameter when nodes are connected
- [gateway] Address greptile review feedback
- [gateway] Create heartbeat cron job on update when missing
- [gateway] Address review feedback on heartbeat cron job creation
- [agents] Sanitize model-mangled tool names from parallel calls
- [agents] Pass sanitized tool name to hook dispatch
- [agents] Add suffix-stripping invariant comment and functions_ edge case test
- [mcp] Fix JS bugs in display name edit flow
- [mcp] Add missing display_name field to doctor test structs
- [mcp] Destructure props in renderServerName and send null for blank display_name
- [tools] Replace hand-rolled html_to_text with html2text crate
- [tools] Address PR review for html_to_text
- [tools] Address second round of PR review
- [tools] Collapse consecutive blank lines, fix doc comment
- [sessions] Use write(true)+seek instead of append(true) for fd_lock on Windows
- [release] Update conditions for jobs to handle dry-run scenarios correctly
- [oauth] Use correct OpenAI client_id, add config layer and tests
- [agents] Prefer tool-capable provider when tools are registered
- [agents] Register builtin providers before genai to enable tool calling
- [gateway] Auto-focus chat input on page load
- [sessions] Add missing sandbox_enabled column migration, stop swallowing errors
- [gateway] Move sandbox button next to model select, fix default state
- Resolve all clippy warnings (collapsible_if, map_flatten, or_default, unused imports)
- [gateway] Open nav panel by default
- [gateway] Add session icons and replace spinner in-place
- [gateway] Add missing renderCompactCard export to page-chat.js
- [gateway] Fix routing, styling, and layout issues in JS modules
- [telegram] Remove channel prefix from user messages and log LLM responses
- [ci] Fix biome version and worktree test default branch
- [ci] Restore biome version pin and fix cargo fmt
- Update CLAUDE.md pre-commit checks and fix clippy warnings
- [ci] Update setup-biome to v2.7.0 and fix sort_by_key clippy lint
- [tests] Configure git user in worktree test setup for CI
- Replace sort_by with sort_by_key in project store
- [assets] Fix biome formatting and empty catch block in sandbox.js
- [ui] Match settings nav item height and style to main navigation
- [security] Wrap API keys in Secret<String> for memory subsystem
- Use RecommendedCache for cross-platform watcher, sort_by_key for clippy
- [hooks] Ignore broken pipe on stdin write when child doesn't read it
- [ui] Show URL and query in web tool call cards
- [sandbox] Restart stopped Apple containers in ensure_ready
- [sandbox] Pass sleep infinity to Apple Container run
- [sandbox] Promote exec routing and container logs to info level
- [sandbox] Handle Apple Container inspect returning empty for nonexistent containers
- [sandbox] Default working_dir to "/" when running inside container
- [security] Reject cross-origin WebSocket upgrades (CSWSH protection)
- [sandbox] Inject env vars in Apple Container backend
- [exec] Redact env var values from command output
- [exec] Redact base64 and hex encoded env var values from output
- [config] Use correct defaults for ToolsConfig when not deserialized
- [gateway] Nav badge counts and cron page instant load
- [memory] Write memory files to data dir instead of cwd
- [gateway] Eliminate identity flash on page load
- [ui] Use consistent button class for skills Remove button
- [ui] Fix skills Disable button style and add error feedback
- [skills] Route disable to correct RPC for plugin-sourced skills
- [ui] Use inline style for tailscale status green dot
- [ui] Use status-dot connected class for tailscale green dot
- [ui] Replace Tailwind arbitrary classes with inline styles in tailscale templates
- [ui] Make tailscale status bar fit-content width
- [ui] Make info-bar full width to match warning/error divs
- [ui] Constrain info-bar to max-width 600px matching alert divs
- [ui] Add alert-info-text to shared alert base styles
- [ui] Add btn-row and btn-row-mt CSS classes for button spacing
- [ui] Space cancel/create buttons apart and normalize height
- [ui] Improve tailscale loading message to set expectations
- [tailscale] Open Tailscale.app on macOS instead of running tailscale up
- [ui] Preserve funnel security warning when rebuilding tailscale DOM
- [ui] Use alert-warning-text style for funnel auth warning
- [ui] Replace auth text with green button linking to security settings
- [ui] Move auth button below the funnel security warning panel
- [ui] Remove @layer components wrapper to fix nav specificity
- [ui] Update session list after /clear and select next on delete
- [ui] Format environment variable timestamps with luxon
- [gateway] Stop echoing web UI messages to Telegram channel
- Collapse nested if statements in config loader
- [ci] Fix package build failures
- [ci] Correct cargo-deb assets order
- [local-llm] Detect available package managers for MLX install
- [local-llm] Detect mlx-lm installed via brew
- [ui] Show install commands on separate lines
- [local-llm] Rename provider from local-gguf to local-llm
- [local-llm] Fix HuggingFace API response parsing
- [ui] Show searching state on HuggingFace search button
- [ui] Close modal and prevent multiple clicks on HF model selection
- [chat] Remove per-message tools warning broadcast
- [ui] Remove scroll from local model list
- [ci] Install cmake for llama.cpp build and fix Biome errors
- [ci] Fix local-llm build in CI
- [migrations] Use set_ignore_missing to allow multi-crate migrations
- [cli] Run gateway migrations from db migrate command
- [telegram] Don't delete channel from database on conflict
- [gateway] Load images nav count async to avoid blocking page serve
- [gateway] Replace branch icon, fix fork UX issues
- Streaming tool calls, skill directory, skills page UX
- [skills] Handle disable for personal/project skills
- [skills] Use app modal for delete confirmation instead of system confirm()
- [skills] Danger button on delete modal, fix disable routing for SkillsService
- [graphql] Add skill_save to MockSkills trait impl
- [gateway] Warm container_cli OnceLock at startup via spawn_blocking
- [security] Prevent stored XSS via HTML inline and block script extensions
- [e2e] Use correct event bus dispatch for send_document tests
- [e2e] Use system-event RPC to dispatch tool events in send_document tests
- [sandbox] Fix clippy CI failures on Linux
- [sandbox] Gate apple-only tests with cfg(target_os = "macos")
- [e2e] Mock slow sandbox APIs in container tests
- [e2e] Make send_document icon test more robust on CI
- [mcp] Address review feedback on is_alive and timeout field naming
- [mcp] Reorder config update to write memory before disk
- Add missing request_timeout_secs field and allow expect in test
- [agents] Address PR review feedback for lazy tool registry
- [e2e] Mock sandbox backend in container e2e tests
- [e2e] Add afterEach unrouteAll to sandbox tests
- [auth] Add Secure attribute to session cookie when TLS is active
- [tools] Make sandbox off test explicit
- [ci] Replace glslc with glslang-tools for Ubuntu 22.04
- [providers] Address PR #408 review feedback
- [ci] Install glslc from LunarG Vulkan SDK on Ubuntu 22.04
- [whatsapp] Improve discoverability and debug logging (#460)
- [whatsapp] Address PR review feedback
- [channels] Redact secrets in channel config API responses
- [channels] Address PR review feedback
- [channels] Compute serialize_struct field counts dynamically
- [providers] Update stale MiniMax context-window comment to include M2.7
- [tools] Ignore exec node param when no nodes are connected (#427)
- [tools] Error on configured default_node when disconnected, not silent fallthrough
- [anthropic] Document system message merging for alternating-role constraint
- [chat] Re-inject datetime in context-overflow retry path
- [browser] Set world-writable permissions on container profile directory
- [browser] Replace expect() with ? in test to satisfy clippy
- [server] Support IPv6 bind addresses (#447)
- [server] Address PR review feedback on IPv6 bind fix
- [server] Include bind address in oauth error, assert address family in test
- [import] Preserve config template comments during OpenClaw import
- [import] Assert on import report in comment-preservation test
- [httpd] Address greptile review findings on PR #465
- Address second round of greptile review on PR #465
- [node-host] Update systemd unit test for quoted ExecStart args
- [web] Allow delete button on cron sessions
- Address greptile review round 3 on PR #465
- [httpd] Align metrics history window and cleanup interval with gateway
- [providers] Address PR review — double-Done, missing ProviderRaw, URL normalization
- [tools] Rescue stringified JSON and flat params in cron tool
- [tools] Exclude "patch" and "job" from flat-param rescue keys
- [ci] Replace non-existent glslc package with glslang-tools
- [docker] Use Node 22 LTS via NodeSource, persist npm cache
- [docker] Avoid silent curl failure, cache npx in container example
- [docker] Use GPG key + apt source instead of NodeSource setup script
- [docker] Ensure /etc/apt/keyrings exists before NodeSource GPG import
- [web] Await event subscriptions before accepting broadcasts
- [ci] Install vulkan-sdk from LunarG instead of Ubuntu's old libvulkan-dev
- [dev] Make local checks OS-aware for LLM backends
- [dev] Merge main and update OS-aware local-LLM checks
- [dev] Address PR review feedback
- [skills] Use slug as fallback when skill name fails validation
- [skills] Address PR review feedback on slug fallback
- [skills] Address second round of PR review feedback
- [skills] Use tempdir in slug fallback error tests for isolation
- [skills] Improve slug error messages and test isolation
- [gateway] Suppress update banner for dev builds


### Security
- [ci] Add zizmor workflow security scan to deb-packages workflow (#8)
- [skills] Add requirements system, spec compliance, and markdown rendering
- [gateway] Add passkey/password auth, API key support, and protected API routes
- Update README/CLAUDE.md, add From impl for SandboxConfig
- [readme] Rewrite with introduction, quickstart, how-it-works, and security sections
- [ui] Replace inline alert/width styles with CSS classes
- [ui] Replace inline style with ml-2 class and add funnel security warning
- [ui] Merge funnel security warning into auth warning banner
- [ui] Split funnel warning into always-visible security text and conditional auth text
- [ci] Add supply chain security and Docker documentation
- [ci] Switch to Sigstore keyless signing for all artifacts
- [ci] Require signed commits
- [ci] Remove redundant signed commits workflow
- Update repository URLs from penso/moltis to moltis-org/moltis
- [security] Add cron rate limiting, job notifications, and fix method auth
- [cli] Add `moltis config check` command
- [ui] Add multi-step onboarding wizard at /onboarding
- [ui] Hide auth banner during onboarding, show auth step for remote users
- [telegram] OTP self-approval for non-allowlisted DM users
- [tools] Add BrowserTool for LLM agents with documentation
- [browser] Add security features and tools settings UI
- Update CHANGELOG and browser-automation docs
- Merge remote-tracking branch 'origin/main' into security-skills
- [security] Add append-only skill security audit log
- [security] Add third-party skills hardening guide
- Merge remote-tracking branch 'origin/main' into security-skills
- Unify plugins and skills into single system
- Add voice services documentation
- [gateway] Add logs download, compression, CORS security, and tower middleware stack
- [onboarding] Add passkey as default auth option in security step
- [cli] Add `moltis doctor` health check command
- [hooks] Add BeforeLLMCall/AfterLLMCall hooks and nonce-based CSP
- [gateway] Consolidate navigation into settings page
- Gate workflows on zizmor security checks
- [docs] Update Docker image references from penso/moltis to moltis-org/moltis
- Keep security controls after auth reset on localhost
- Unify localhost auth-disabled security warning
- [tools] Add dangerous command blocklist as approval safety floor
- [readme] Restructure with comparison matrix and crate architecture
- [vault] Add encryption-at-rest vault for environment variables (#219)
- [discord] Add Discord channel integration (#239)
- [sandbox] Add Wasmtime WASM sandbox, Docker hardening, generic failover (#243)
- [sandbox] Trusted network mode with domain-filtering proxy (#15)
- [ios,courier] IOS companion app and APNS push relay (#248)
- [macos] Wire settings UI to rust config backend (#267)
- [channels] Shared channel webhook middleware pipeline (#290)
- [nodes] Add multi-node support with device pairing, remote exec, and UI (#291)
- [security] Add direct nginx websocket proxy example (#364)
- [ci] Add zizmor workflow security scan to deb-packages workflow (#8)
- [skills] Add requirements system, spec compliance, and markdown rendering
- [gateway] Add passkey/password auth, API key support, and protected API routes
- Update README/CLAUDE.md, add From impl for SandboxConfig
- [readme] Rewrite with introduction, quickstart, how-it-works, and security sections
- [ui] Replace inline alert/width styles with CSS classes
- [ui] Replace inline style with ml-2 class and add funnel security warning
- [ui] Merge funnel security warning into auth warning banner
- [ui] Split funnel warning into always-visible security text and conditional auth text
- [ci] Add supply chain security and Docker documentation
- [ci] Switch to Sigstore keyless signing for all artifacts
- [ci] Require signed commits
- [ci] Remove redundant signed commits workflow
- Update repository URLs from penso/moltis to moltis-org/moltis
- [security] Add cron rate limiting, job notifications, and fix method auth
- [tools] Add send_document tool for file sharing to channels

## [0.10.18] - 2026-03-09
### Added
- [gateway] Make provider discovery startup non-blocking
- [monitoring] Track memory history and improve local-llm memory reporting (#325)
- [ios] Add local llama cpp memory field to GraphQL schema
- [providers] Include reasoning fields for kimi models (#323)
- [chat] Tabs to filter chats between sessions and cron (#338)
- [oauth] Support pasted callback URL fallback (#365)
- [providers] Add reasoning effort support for models with extended thinking (#363)


### Changed
- Externalize web/wasm assets and reduce memory footprint (#321)
- [web] Move chat history hydration to paged HTTP
- [web] Paginate sessions and auto-load older history


### Removed
- [web] Remove nested onboarding scroll and restore settings nav icons
- [web] Declutter chat controls and fix dropdown positioning


### Fixed
- [gateway] Speed up startup by deferring tailscale and log scan
- [gateway] Improve browser warmup integration
- [scripts] Run local nextest with ci timeout profile
- [ci] Build macOS app arm64 in fast path
- [web] Move session history off websocket and cap payload size
- [web] Use combo select for session header selectors
- [web] Externalize SVG icons and restore empty-chat centering
- [web] Align e2e with controls modal and daily model refresh
- [ci] Stage wasm assets for cargo-deb packaging
- [packaging] Use cli-relative web assets in cargo-deb
- Install rustls CryptoProvider before channel startup (#336)
- [ci,tools] Unblock dependabot and support wasmtime 36
- [auth] Honor forwarded host for proxy session cookies
- [config] Include tmux in default sandbox packages
- [mdns] Use stable host label to avoid mDNSResponder conflict and double-.local suffix (#349)
- [web] Prevent Enter key from triggering actions during IME composition (#341)
- [biome] Update schema to 2.4.6 and move noUnusedExpressions to suspicious
- [ci] Update biome version to 2.4.6 in CI workflows
- [macos] Extract makeTextView to fix function body length lint violation
- [providers] Report compatible client_version for Codex model discovery (#359)
- [prompt] Omit sandbox/node info from runtime prompt when disabled (#362)
- [web] Allow deleting cron sessions from chat sidebar (#357)
- [chat] Skip duplicate text fallback when TTS disabled and voice streamed (#373)
- [web] Break redirect loop when accessing via Tailscale Serve (#356)


### Security
- [nodes] Add multi-node support with device pairing, remote exec, and UI (#291)
- [security] Add direct nginx websocket proxy example (#364)

## [0.10.17] - 2026-03-05
### Fixed
- [config] Include tmux in default sandbox packages

## [0.10.16] - 2026-03-05
### Fixed
- [ci,tools] Unblock dependabot and support wasmtime 36
- [auth] Honor forwarded host for proxy session cookies

## [0.10.15] - 2026-03-05
### Fixed
- Install rustls CryptoProvider before channel startup (#336)

## [0.10.14] - 2026-03-05
### Fixed
- [packaging] Use cli-relative web assets in cargo-deb

## [0.10.13] - 2026-03-04
### Fixed
- [ci] Stage wasm assets for cargo-deb packaging

## [0.10.12] - 2026-03-04
### Added
- [ci] Add release dry-run mode
- [browser] Add container_host for Docker-in-Docker connectivity (#300)
- [ios] Auto-discover server identity and show emojis (#297)
- [website] Migrate cloudflare website into monorepo (#302)
- [local-llm] Allow arbitrary HuggingFace model IDs for MLX models
- [web,tools] AOT WASM pre-compilation and Shiki CDN loading
- [cli] Remove wasm from default features to reduce memory
- [gateway] Make provider discovery startup non-blocking
- [monitoring] Track memory history and improve local-llm memory reporting (#325)
- [ios] Add local llama cpp memory field to GraphQL schema


### Changed
- [web] Move settings nav icons from JS to CSS
- Externalize web/wasm assets and reduce memory footprint (#321)
- [web] Move chat history hydration to paged HTTP
- [web] Paginate sessions and auto-load older history


### Removed
- [web] Remove nested onboarding scroll and restore settings nav icons
- [web] Declutter chat controls and fix dropdown positioning


### Fixed
- [config] Support provider url alias for remote Ollama config (#299)
- [ci] Make release dry-run job conditions valid
- [providers] Use Ollama capabilities for tool support detection (#301)
- [scripts] Roll back heavy local validation parallelism
- [web] Skip npm install when TAILWINDCSS binary is provided
- [ci] Update website/releases.json on release
- [web] Add missing i18n button key for preferredModels
- [local-llm] Use sampler API for mlx-lm >= 0.20
- [gateway] Break redirect loop when onboarded but auth not configured (#310)
- [gateway] Reduce idle CPU from metrics loop and log broadcast feedback
- [gateway] Speed up startup by deferring tailscale and log scan
- [gateway] Improve browser warmup integration
- [scripts] Run local nextest with ci timeout profile
- [ci] Build macOS app arm64 in fast path
- [web] Move session history off websocket and cap payload size
- [web] Use combo select for session header selectors
- [web] Externalize SVG icons and restore empty-chat centering
- [web] Align e2e with controls modal and daily model refresh


### Security
- [nodes] Add multi-node support with device pairing, remote exec, and UI (#291)

## [0.10.11] - 2026-03-02

## [0.10.10] - 2026-03-02
### Fixed
- [swift-bridge] Stabilize gateway migration and stream tests

## [0.10.9] - 2026-03-02
### Fixed
- [ci] Harden tailwindcss cli downloads

## [0.10.8] - 2026-03-02
### Changed
- [gateway] Fetch updates from releases manifest instead of GitHub API


### Fixed
- [ci] Add Tailwind CSS build step to release workflow, Dockerfile, and snapcraft
- [e2e] Wait for session history render before DOM injection in chat-abort

## [0.10.7] - 2026-03-02
### Added
- [sandbox] Add GitHub runner parity packages and enable corepack (#284)
- [providers] Add first-class LM Studio provider (#286)
- [agents] Enrich spawn_agent presets with identity, policies, memory (#271)
- [web] Show running version at bottom of identity settings
- [channels] Channel architecture phase 5, contract suites, and observability baseline (#289)


### Changed
- [channels] Registry-driven dispatch for cheap new channels (#277)


### Fixed
- [e2e] Make sandboxes container tests deterministic
- [e2e] Replace remaining racy waitForResponse with route interceptors
- [mcp] Make optional MCP tool params nullable to prevent empty string errors (#283)
- [provider-setup] Reorder validation probes to prefer fast models (#280)
- [sandbox] Resolve host gateway IP for Podman < 5.0 (#287)
- [e2e] Fix flaky "deleting unmodified fork" test
- [ci] Stale lockfile, missing Tailwind in macOS job, OAuth e2e setup
- [ci] Use standalone Tailwind binary for macOS app job
- [e2e] Fix OAuth token-exchange failure test and add error-context capture
- [web] Auto-install node_modules in Tailwind build script
- [web] Retry openclaw onboarding scan until ws is ready


### Security
- [macos] Wire settings UI to rust config backend (#267)
- [channels] Shared channel webhook middleware pipeline (#290)

## [0.10.6] - 2026-03-01
### Fixed
- [web] Fix Shiki highlighter init failures in E2E tests
- [web] Make thinking stop button smaller with left spacing
- [chat] Surface error when LLM returns empty response with zero tokens
- [providers] Emit StreamEvent::Error on non-success finish_reason

## [0.10.5] - 2026-03-01
### Fixed
- [web] Lazy-load Shiki to prevent blocking page mount

## [0.10.4] - 2026-03-01
### Added
- [web] Add Shiki syntax highlighting to code blocks

## [0.10.3] - 2026-03-01
### Added
- Add channel-aware heartbeat delivery and send_message agent tool (#270)
- [memory] Add tree-sitter code splitter and RRF search merge


### Changed
- [ffi] Tighten unsafe_code allowances


### Fixed
- [sandbox] Make apple container keepalive portable (#269)
- [local-llm] Combine compile-time and runtime Metal detection
- [auth] Auto-detect new WebAuthn hosts and prompt passkey refresh (#268)
- [web] Replace rg with grep in changelog guard and deduplicate passkey status refresh

## [0.10.2] - 2026-02-28


### Added

### Changed

### Deprecated

### Removed

### Fixed

- Release packaging now installs cross-compilation targets on the active nightly toolchain in the Homebrew binary job, fixing `error[E0463]: can't find crate for core` during macOS binary builds.
- Docker release builds now copy `apps/courier` into the image build context so Cargo workspace metadata resolves correctly during WASM component builds.
### Security

## [0.10.1] - 2026-02-28


### Added

### Changed

### Deprecated

### Removed

### Fixed

### Security

## [0.10.0] - 2026-02-28


### Added

- **Gemini first-class provider**: Google Gemini is now registered via the OpenAI-compatible endpoint with native tool calling, vision/multimodal support, streaming, and model discovery. Replaces the previous genai-backed fallback that lacked tool support. Supports both `GEMINI_API_KEY` and `GOOGLE_API_KEY` environment variables
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
