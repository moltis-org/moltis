# Changelog

All notable changes to the Moltis macOS app will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Added

- Real-time session sync with web UI — sessions created, deleted, or patched in
  the web UI now instantly appear in the macOS sidebar (and vice versa) via the
  new `SessionEventBus` and `moltis_set_session_event_callback` FFI
- Full Moltis gateway in the HTTP Server pane — starts the complete gateway
  (web UI, REST API, WebSocket endpoints, DB migrations, background services)
  instead of a minimal health-check-only server
- Bind address picker (Loopback / All interfaces) replacing the free-text host field
- Progress spinner during gateway startup (DB migrations can take 1-3s)
- Async server startup dispatched off the main thread to keep the UI responsive
- Model picker labels now show context size and tier badges
- Provider model list sorting by context window size
- Identity loaded from `moltis.toml` + `SOUL.md` at startup via new
  `moltis_get_identity` FFI — Settings→Identity now shows agent name, emoji,
  theme, user name, and soul
- Version loaded on app launch via `chatStore.loadVersion()`
- Persistent session storage via FFI — sessions backed by JSONL files shared
  with the web UI and TUI (no longer ephemeral in-memory arrays)
- Session CRUD FFI functions: `moltis_list_sessions`, `moltis_switch_session`,
  `moltis_create_session`, `moltis_session_chat_stream`
- Session sidebar now loads from disk and shows message counts
- Abort generation button (red stop circle) replaces the send button while streaming
- Peek popover on session toolbar (eye icon) showing thinking text, active tool calls, or idle state
- `abortSession(key:)` and `peekSession(key:)` FFI bindings via `moltis_abort_session` and `moltis_peek_session`
- Chat messages are persisted to `~/.moltis/data/sessions/<key>.jsonl`
- "Open in Browser" button in HTTP Server pane when the gateway is running

### Changed

- Session metadata now stored in SQLite (`moltis.db`) instead of JSON file,
  so the macOS app and web UI share the same session list

- HTTP Server description updated to reflect full gateway capability
- Server start/stop controls disabled while startup is in progress
- Chat is no longer tied to HTTP server — works with or without HTTPD enabled

### Fixed

- Bind address picker and port field now visible before enabling HTTP server
  (previously hidden inside the enable-toggle conditional block)

## 2025-02-25

### Added

- Full-screen monospace TOML configuration editor with file I/O
  - Loads and saves `~/.moltis/moltis.toml` directly
  - Unsaved-changes indicator, Reveal in Finder button, Cmd+S save
- Streaming LLM responses via C function pointer callback (`moltis_chat_stream` FFI)
- Themed message bubbles matching web UI colors (light/dark mode)
- Live token bar with K/M formatting for input/output tokens
- In-app logging system with Rust-to-Swift bridge (`moltis_set_log_callback`)
  - LogStore with level/target/search filtering, pause/resume
  - JSONL export and clipboard copy
- Thinking dots and streaming cursor animations
- HTTP Server settings pane with start/stop toggle and status display
- `HttpdPane` view wired into the settings sidebar

### Fixed

- Sidebar list selection appearance by switching from NSPanel to WindowGroup
- Added grouped Section headers and search field to settings
- Cmd+, keyboard shortcut for opening settings

## 2025-02-24

### Added

- Initial macOS app structure under `apps/macos/`
- Swift-Rust FFI bridge via `moltis-swift-bridge` static library
  - JSON serialization between Swift and Rust
  - `with_ffi_boundary` panic safety wrapper
  - `consumeCStringPointer` + `moltis_free_string` memory management
- Chat interface with message history and send functionality
- Provider management: list known providers, save API keys, detect configured providers
- Model listing and selection
- Settings UI with sidebar navigation (matching macOS System Settings style)
  - Identity, Environment, Memory, Notifications, Cron Jobs, Security,
    Channels, Hooks, LLM Provider, MCP Servers, Skill Packs, Voice,
    Terminal, Sandbox, Monitoring, Logs, GraphQL, Configuration sections
  - Colored icon badges, section grouping, search filtering
- Structured DisclosureGroup list views for Channels, Hooks, MCP, Skills, Crons
- Onboarding flow for first-launch provider setup
- Retina app icon asset catalog (16–1024px from Moltis crab SVG)
- `MoltisClient` helper with `callBridge` pattern for FFI encode-call-decode
- `BridgeHttpdStatus`, `startHttpd`, `stopHttpd`, `httpdStatus` client methods
- Resizable settings window via AppKit styleMask integration

### Changed

- Simplified model structs with `convertFromSnakeCase` decoding (removed 7 manual CodingKeys enums)
- Extracted `MessageBubbleView` into its own file

### Fixed

- All SwiftLint violations from initial codebase
