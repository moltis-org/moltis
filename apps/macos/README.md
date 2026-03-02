# Moltis macOS App

This app embeds Moltis Rust code into a native macOS SwiftUI app.

On first launch, the app opens an AppKit onboarding wizard to configure identity
and model defaults before entering the chat UI.

## Prerequisites

- `xcodegen`
- `swiftlint`
- `cbindgen`
- Xcode command line tools

## Build flow

From the repository root:

```bash
just swift-build-rust
just swift-generate
just swift-lint
just swift-build
just swift-run
```

Run the app from Xcode:

```bash
just swift-open
```
