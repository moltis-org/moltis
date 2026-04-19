# Native Swift App with Embedded Moltis Rust Core

This guide covers the native Swift/macOS app that embeds Moltis Rust code as a static library via FFI.

Architecture:

- Business/runtime logic lives in Rust.
- Native macOS UI built in SwiftUI.
- Shipped as one app bundle (no separate Rust service process).

## Implementation

The FFI bridge lives in `crates/swift-bridge` (`crate-type = ["staticlib"]`) with the Swift app in `apps/macos/`.

1. Rust crate compiles as `staticlib` for Apple targets.
2. Public API exposed via `extern "C"` functions (JSON in/out over `*mut c_char`).
3. Swift calls the ABI through a bridging header (`Bridging-Header.h`).
4. Swift owns all presentation and user interaction.

## Architecture Diagram

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

### FFI API Surface

All functions pass JSON strings across the boundary. Returned `*mut c_char` pointers are allocated by Rust and must not be freed by the caller.

The API is organized across six modules:

| Module | Functions |
|--------|-----------|
| `ffi_core` | `moltis_version`, `moltis_get_identity`, `moltis_chat_json`, `moltis_known_providers`, `moltis_detect_providers`, `moltis_save_provider_config`, `moltis_list_models`, `moltis_refresh_registry`, `moltis_set_log_callback`, `moltis_set_session_event_callback`, `moltis_set_network_audit_callback`, `moltis_start_httpd`, `moltis_stop_httpd`, `moltis_httpd_status`, `moltis_abort_session`, `moltis_peek_session`, `moltis_shutdown` |
| `ffi_config` | `moltis_get_config`, `moltis_save_config`, `moltis_memory_status`, `moltis_memory_config_get`, `moltis_memory_config_update`, `moltis_memory_qmd_status`, `moltis_get_soul`, `moltis_save_soul`, `moltis_save_identity`, `moltis_save_user_profile`, `moltis_list_env_vars`, `moltis_set_env_var`, `moltis_delete_env_var` |
| `ffi_sessions` | `moltis_list_sessions`, `moltis_switch_session`, `moltis_create_session`, `moltis_session_chat_stream` |
| `ffi_auth` | `moltis_auth_status`, `moltis_auth_password_change`, `moltis_auth_reset`, `moltis_auth_list_passkeys`, `moltis_auth_remove_passkey`, `moltis_auth_rename_passkey` |
| `ffi_sandbox` | `moltis_sandbox_status`, `moltis_sandbox_list_images`, `moltis_sandbox_delete_image`, `moltis_sandbox_prune_images`, `moltis_sandbox_check_packages`, `moltis_sandbox_build_image`, `moltis_sandbox_get_default_image`, `moltis_sandbox_set_default_image`, `moltis_sandbox_get_shared_home`, `moltis_sandbox_set_shared_home`, `moltis_sandbox_list_containers`, `moltis_sandbox_stop_container`, `moltis_sandbox_remove_container`, `moltis_sandbox_clean_containers`, `moltis_sandbox_disk_usage`, `moltis_sandbox_restart_daemon` |
| `chat` | `moltis_chat_stream` (callback-based streaming) |

## Rust-side Implementation Notes

The bridge crate is `crates/swift-bridge`:

- `crate-type = ["staticlib"]` for Apple targets.
- `extern "C"` functions organized across modules (`ffi_core`, `ffi_config`, `ffi_sessions`, `ffi_auth`, `ffi_sandbox`, `chat`).
- Never expose internal Rust structs directly.
- Return `*mut c_char` (caller must not free; Rust manages allocation).
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

## Streaming

Streaming is fully implemented via callback functions. The bridge provides:

- `moltis_chat_stream(request_json, callback, user_data)` — global session streaming.
- `moltis_session_chat_stream(request_json, callback, user_data)` — per-session streaming.

Events are delivered as JSON with `type` field: `delta` (token), `done` (usage stats), `error`. The stream runs on the bridge's tokio runtime and returns immediately; the caller must keep `user_data` alive until a terminal event.

## Crate Features

The `moltis-swift-bridge` crate has optional features:

- `metrics` (default) — exposes metrics integration.
- `qmd` (default) — enables QMD memory surface.
- `tracing` (default) — tracing integration.
- `trusted-network` (default) — trusted network support.

## Risks to Watch Early

- ABI drift (solve with one owned header and narrow API).
- Threading assumptions across Swift and Rust runtimes.
- Logging and secret handling at the boundary.
- Cross-target build complexity (simulator vs device architectures).

## Why This Fits Moltis

Moltis already has clear crate boundaries and async services. A thin FFI facade lets Swift own the native UX while reusing provider orchestration, config, and session logic from Rust.
