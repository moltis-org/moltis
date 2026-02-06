# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

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

### Documentation

- Added mobile-pwa.md with PWA installation and push notification documentation
- Updated CLAUDE.md with cargo feature policy (features enabled by default)
- Updated browser-automation.md with browser detection, screenshot display, and
  model error handling sections
