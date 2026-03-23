# iOS App Guidelines

The Moltis iOS app connects to a running Moltis gateway over the network. It does **not** embed any Rust code — it's a pure Swift/SwiftUI app that communicates via WebSocket RPC and GraphQL.

## Architecture

- **WebSocket RPC** (`/ws/chat`): Real-time chat, streaming events, session management
- **GraphQL** (`/graphql`): Queries for sessions, models, usage stats, config
- **REST** (`/api/auth/*`): Authentication (login, API key creation)

## Build

```bash
just ios-generate   # XcodeGen → .xcodeproj
just ios-build      # xcodebuild (generic iOS)
just ios-lint       # SwiftLint
just ios-open       # Open in Xcode
```

## Key Conventions

- No external dependencies — use `URLSession` for all networking
- Store API keys in Keychain via `KeychainHelper`
- Use `@EnvironmentObject` for store injection (ConnectionStore, ChatStore, etc.)
- Live Activities via ActivityKit — shared `MoltisActivityAttributes` between app and widget
- App Group `group.org.moltis.ios` for shared data between app and widget extension
- Match visual theme from `MoltisTheme` (same colors as macOS app / web UI)

## Changelog

Update `CHANGELOG.md` in this directory when making user-facing changes. Follow [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) format.
