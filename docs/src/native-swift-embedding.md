# Native Swift App with Embedded Moltis Rust Core (POC)

This guide shows a **proof-of-concept path** to build a native Swift app where Swift is the UI layer and Moltis Rust code is embedded as a local library.

Goal:

- Keep business/runtime logic in Rust.
- Build native iOS/macOS UI in Swift/SwiftUI.
- Ship as one app bundle from the Swift side (no separate Rust service process).

## Feasibility

Yes — this architecture is feasible with an FFI boundary.

The most practical POC shape is:

1. Add a small Rust crate that compiles as `staticlib`.
2. Expose a narrow C ABI (`extern "C"`) surface.
3. Call that ABI from Swift via a bridging header/module map.
4. Keep Swift responsible for presentation and user interaction.

## Recommended POC Architecture

```
SwiftUI / UIKit / AppKit
        |
        v
Swift wrapper types (safe Swift API)
        |
        v
C ABI bridge (headers + extern "C")
        |
        v
Rust core facade (thin FFI-safe layer)
        |
        v
Existing Moltis crates (chat/providers/config/etc.)
```

### Boundary Rules

For the POC, keep the ABI intentionally small:

- `moltis_version()`
- `moltis_chat_json(request_json)`
- `moltis_free_string(ptr)`
- `moltis_shutdown()`

Pass JSON strings across FFI to avoid unstable struct layouts early on.

## Rust-side Implementation Notes

Create a dedicated bridge crate (example name: `crates/swift-bridge`):

- `crate-type = ["staticlib"]` for Apple targets.
- Keep all `extern "C"` functions in one module.
- Never expose internal Rust structs directly.
- Return `*mut c_char` and provide explicit free functions.
- Convert internal errors into structured JSON error payloads.

Safety checklist:

- Validate all incoming pointers and UTF-8.
- Do not panic across FFI boundaries (`catch_unwind` at boundary).
- Keep ownership explicit (allocator symmetry for returned memory).
- Do not leak secrets into logs or debug output.

## Swift-side Integration Notes

Use YAML-generated Xcode projects for the POC (no hand-maintained `.xcodeproj`):

1. Define app targets in `apps/macos/project.yml`.
2. Generate project with XcodeGen.
3. Link `Generated/libmoltis_bridge.a` and include `Generated/moltis_bridge.h`.
4. Use a Swift facade (`MoltisClient`) to own pointer and lifetime rules.
5. Keep Swift linted via `apps/macos/.swiftlint.yml`.

From repo root:

```bash
just swift-build-rust
just swift-generate
just swift-lint
just swift-build
```

The UI remains purely SwiftUI while core requests/responses flow through the Rust bridge.


## Intel + Apple Silicon (Universal `libmoltis`)

Yes — you can build `libmoltis` for both Intel and Apple Silicon and merge them into one universal macOS static library.

### Build both architectures

```bash
rustup target add x86_64-apple-darwin aarch64-apple-darwin

# Intel
cargo build -p moltis-swift-bridge --release --target x86_64-apple-darwin

# Apple Silicon
cargo build -p moltis-swift-bridge --release --target aarch64-apple-darwin
```

### Merge into one universal archive

```bash
mkdir -p target/universal-macos/release
lipo -create \
  target/x86_64-apple-darwin/release/libmoltis_bridge.a \
  target/aarch64-apple-darwin/release/libmoltis_bridge.a \
  -output target/universal-macos/release/libmoltis_bridge.a

lipo -info target/universal-macos/release/libmoltis_bridge.a
```

This universal `libmoltis_bridge.a` can then be linked by your Swift macOS app, so one app build supports both Intel and M-series Macs.

### Recommended packaging for Xcode

For production, prefer an `XCFramework` (device/simulator/platform-safe packaging) rather than manually juggling multiple `.a` files.

## Async/Streaming Strategy

Moltis is async-first. For a POC:

- Start with request/response calls over FFI.
- Add streaming in phase 2 using callback registration or poll handles.

Simple incremental plan:

1. Blocking/synchronous POC call (prove bridge correctness).
2. Background `Task` wrapping on Swift side.
3. Token streaming callback API when stable.

## Single-Binary Expectation Clarification

On Apple platforms, you typically ship a **single app artifact** that includes Swift executable + statically linked Rust library in one app bundle.

So for your POC requirement (Swift UI app that embeds Rust core without a separate Rust daemon), this is achievable.

## POC Milestones

1. Add `swift-bridge` crate exposing one health function (`moltis_version`).
2. Add one end-to-end chat method (`moltis_chat_json`).
3. Build and link from minimal SwiftUI app.
4. Validate memory lifecycle with repeated calls.
5. Expand API surface only after the boundary is stable.

## Risks to Watch Early

- ABI drift (solve with one owned header and narrow API).
- Threading assumptions across Swift and Rust runtimes.
- Logging and secret handling at the boundary.
- Cross-target build complexity (simulator vs device architectures).

## Why This Fits Moltis

Moltis already has clear crate boundaries and async services. A thin FFI facade lets Swift own the native UX while reusing provider orchestration, config, and session logic from Rust.
