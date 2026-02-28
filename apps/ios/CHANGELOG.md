# Changelog

All notable changes to the Moltis iOS app will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Added

- Initial iOS app with full chat, session management, and Live Activities
- WebSocket RPC client for real-time chat streaming (protocol v3)
- GraphQL client for sessions, models, and server status queries
- API key authentication with Keychain storage
- Multiple saved server connections
- Live Activity on Lock Screen and Dynamic Island showing AI progress
- Tool call banners with SF Symbols for bash, read, write, search, browse
- Session list with search, create, and delete
- Model picker grouped by provider
- Theme colors matching macOS app and web UI
- Connect screen links each discovered server to CA PEM download (`/certs/ca.pem`) and includes in-app iOS trust-install steps
- Nearby servers now auto-check TLS trust and show a prominent CA download button only when trust is missing
- Connection checks now show explicit setup guidance when remote auth is incomplete, password auth is missing, or GraphQL is disabled
- Chat now includes explicit keyboard dismissal controls (Done key, scroll-to-dismiss, tap-to-dismiss) so tab navigation remains reachable
- Connection banner now reports server drop/retry state with automatic WebSocket reconnect attempts
- Chat now has a ChatGPT-style top bar with large model/provider pill, top-right settings button, and a left slide-out sessions drawer
- Removed the bottom tab bar so chat is the single root screen, with sessions/settings accessed from the top controls
- Refined chat controls: larger send/stop action button and redesigned connection banner card with richer status details
- Settings now includes optional location sharing, with iOS permission/status handling and real-time GraphQL mutation updates to `agents.updateIdentity`
- Thinking text display (italic orange caption) shown below tool call banner while streaming
- Peek session support via `chat.peek` RPC with `PeekResult` model (active state, thinking text, tool call names)
- Abort broadcast handler (`aborted` event) that cleans up streaming state and Live Activity

### Fixed

- New session creation now uses a generated `session:<uuid>` key with `sessions.switch` flow (matching web) instead of calling removed RPC method `sessions.create`
- Model loading query now matches current GraphQL schema (`models.list { id name provider }`) and logs detailed GraphQL/RPC diagnostics to Xcode console
- Connect actions (`Check Connection`, `Login & Connect`, `Connect with API Key`) now render as standard prominent buttons
- Removed the custom keyboard accessory dismiss button in chat to prevent overlap with the send control
