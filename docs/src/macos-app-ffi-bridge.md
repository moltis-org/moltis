# macOS App FFI Bridge (Work in Progress)

```admonish warning
This macOS app integration is not finished yet. It is currently being built.
```

This page documents how `apps/macos` currently bridges Swift to Rust through FFI.

## Runtime Architecture

```text
┌──────────────────────────────────────────────────────────────────────────┐
│ Moltis.app (single macOS process)                                        │
│                                                                          │
│  SwiftUI Views                                                           │
│  (ContentView, OnboardingView, SettingsView, ...)                        │
│                    │                                                     │
│                    ▼                                                     │
│  State stores                                                            │
│  (ChatStore, ProviderStore, LogStore)                                    │
│                    │                                                     │
│                    ▼                                                     │
│  Swift FFI facade: MoltisClient.swift                                    │
│  - encodes requests to JSON                                              │
│  - calls C symbols from `moltis_bridge.h`                                │
│  - decodes JSON responses / bridge errors                                │
└────────────────────┬─────────────────────────────────────────────────────┘
                     │
                     │ C ABI (`moltis_*`)
                     ▼
┌──────────────────────────────────────────────────────────────────────────┐
│ Rust bridge static library: `libmoltis_bridge.a`                         │
│ crate: `crates/swift-bridge`                                             │
│                                                                          │
│  `extern "C"` exports                                                    │
│  (chat, streaming, providers, sessions, httpd, version, shutdown, ...)   │
│                    │                                                     │
│                    ▼                                                     │
│  Rust bridge internals                                                   │
│  - pointer/UTF-8 + JSON validation                                       │
│  - panic boundary (`catch_unwind`)                                       │
│  - tokio runtime + provider registry + session storage                   │
│                                                                          │
│  FFI modules:                                                            │
│  - `ffi_core` — version, chat, providers, httpd                          │
│  - `ffi_sessions` — list, create, switch, streaming                      │
│  - `ffi_config` — config, identity, soul, memory, env vars               │
│  - `ffi_auth` — passkeys, password, auth status                          │
│  - `ffi_sandbox` — images, containers, packages, disk usage              │
└────────────────────┬─────────────────────────────────────────────────────┘
                     │
                     ▼
┌──────────────────────────────────────────────────────────────────────────┐
│ Reused Moltis crates                                                     │
│ (`moltis-providers`, `moltis-sessions`, `moltis-gateway`, etc.)          │
└──────────────────────────────────────────────────────────────────────────┘

Reverse direction callbacks:
- Rust logs: `moltis_set_log_callback(...)` -> Swift `LogStore`
- Rust streaming events: `moltis_*_chat_stream(...)` callback -> Swift closures
- Rust session events: `moltis_set_session_event_callback(...)` -> Swift `ChatStore`
```

## Build and Link Pipeline

```text
`just swift-build-rust`
        │
        ▼
scripts/build-swift-bridge.sh
  1) cargo build -p moltis-swift-bridge --target x86_64-apple-darwin
  2) cargo build -p moltis-swift-bridge --target aarch64-apple-darwin
  3) lipo -create -> universal `libmoltis_bridge.a`
  4) cbindgen -> `moltis_bridge.h`
  5) copy both artifacts into `apps/macos/Generated/`
        │
        ▼
`just swift-generate` (xcodegen from `apps/macos/project.yml`)
        │
        ▼
Xcode build
  - header search path: `apps/macos/Generated`
  - library search path: `apps/macos/Generated`
  - links `-lmoltis_bridge`
  - uses `Sources/Bridging-Header.h` -> includes `moltis_bridge.h`
```

## Main FFI Touchpoints

- Swift header import: `apps/macos/Sources/Bridging-Header.h`
- Swift facade: `apps/macos/Sources/MoltisClient.swift`
- Rust exports: `crates/swift-bridge/src/ffi_*.rs` (domain modules)
- Artifact builder: `scripts/build-swift-bridge.sh`
- Xcode linking config: `apps/macos/project.yml`

## Real-time Session Sync

Sessions created in the macOS app appear in the web UI (and vice versa) in
real time thanks to a shared `tokio::sync::broadcast` channel — the
`SessionEventBus`.

```text
┌──────────────┐  publish   ┌─────────────────┐  subscribe  ┌────────────────┐
│ Bridge FFI   │ ────────→ │ SessionEventBus  │ ────────→  │ FFI callback   │→ macOS app
│ (macOS app)  │           │ (broadcast chan)  │            │ (bridge lib.rs)│
└──────────────┘           └─────────────────┘            └────────────────┘
                                  ↑
┌──────────────┐  publish         │
│ Gateway RPCs │ ─────────────────┘
│ (sessions.*) │        (also broadcasts to WS clients directly)
└──────────────┘
```

When HTTPD is enabled, the bridge passes its bus instance to `prepare_gateway()`
so both share the same channel. Events:

| Kind      | Trigger                                    |
|-----------|--------------------------------------------|
| `created` | `sessions.resolve` (new), `sessions.fork`, bridge `moltis_create_session` |
| `patched` | `sessions.patch`                           |
| `deleted` | `sessions.delete`                          |

Swift receives events via `moltis_set_session_event_callback` — each event is a
JSON object `{"kind":"created","sessionKey":"..."}` dispatched to
`ChatStore.handleSessionEvent()` on the main thread.

## Current Status

The bridge is already functional for core flows (version, chat, streaming, providers, sessions, embedded httpd), but this is still a POC-stage macOS app and the integration surface is still evolving.

## FFI Functions

### Core (`ffi_core`)

| Function | Purpose |
|----------|---------|
| `moltis_version` | Get gateway version |
| `moltis_get_identity` | Get gateway identity info |
| `moltis_shutdown` | Graceful shutdown |
| `moltis_chat_json` | Send chat message |
| `moltis_abort_session` | Abort active chat |
| `moltis_peek_session` | Peek at active session state |
| `moltis_detect_providers` | Probe for local/cloud providers |
| `moltis_known_providers` | List configured providers |
| `moltis_list_models` | List available models |
| `moltis_refresh_registry` | Re-scan model registry |
| `moltis_save_provider_config` | Save provider settings |
| `moltis_start_httpd` | Start embedded HTTP server |
| `moltis_stop_httpd` | Stop embedded HTTP server |
| `moltis_httpd_status` | Get HTTP server status |
| `moltis_httpd_status` | Query HTTP server state |
| `moltis_peek_session` | Peek at session context |

### Sessions (`ffi_sessions`)

| Function | Purpose |
|----------|---------|
| `moltis_list_sessions` | List all sessions |
| `moltis_create_session` | Create a new session |
| `moltis_switch_session` | Switch active session |
| `moltis_session_chat_stream` | Stream chat responses (callback) |

### Config (`ffi_config`)

| Function | Purpose |
|----------|---------|
| `moltis_get_config` | Read full config |
| `moltis_save_config` | Write config values |
| `moltis_get_identity` | Read agent identity |
| `moltis_save_identity` | Update agent identity |
| `moltis_get_soul` | Read agent soul/prompt |
| `moltis_save_soul` | Update agent soul |
| `moltis_save_user_profile` | Update user profile |
| `moltis_memory_status` | Memory system status |
| `moltis_memory_config_get` | Read memory config |
| `moltis_memory_config_update` | Update memory config |
| `moltis_memory_qmd_status` | QMD feature status |
| `moltis_list_env_vars` | List environment variables |
| `moltis_set_env_var` | Set an environment variable |
| `moltis_delete_env_var` | Delete an environment variable |

### Auth (`ffi_auth`)

| Function | Purpose |
|----------|---------|
| `moltis_auth_status` | Auth state check |
| `moltis_auth_password_change` | Change password |
| `moltis_auth_reset` | Reset auth |
| `moltis_auth_list_passkeys` | List passkeys |
| `moltis_auth_remove_passkey` | Remove passkey |
| `moltis_auth_rename_passkey` | Rename passkey |

### Sandbox (`ffi_sandbox`)

| Function | Purpose |
|----------|---------|
| `moltis_sandbox_status` | Sandbox status |
| `moltis_sandbox_list_images` | List sandbox images |
| `moltis_sandbox_delete_image` | Delete sandbox image |
| `moltis_sandbox_prune_images` | Prune unused images |
| `moltis_sandbox_build_image` | Build custom image |
| `moltis_sandbox_check_packages` | Check installed packages |
| `moltis_sandbox_get_default_image` | Get default image |
| `moltis_sandbox_set_default_image` | Set default image |
| `moltis_sandbox_get_shared_home` | Get shared home path |
| `moltis_sandbox_set_shared_home` | Set shared home path |
| `moltis_sandbox_list_containers` | List running containers |
| `moltis_sandbox_stop_container` | Stop a container |
| `moltis_sandbox_remove_container` | Remove a container |
| `moltis_sandbox_clean_containers` | Clean all containers |
| `moltis_sandbox_disk_usage` | Sandbox disk usage |
| `moltis_sandbox_restart_daemon` | Restart sandbox daemon |
