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
