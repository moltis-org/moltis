# Windows Moltis App (WinUI 3 + Rust FFI) Plan

## Summary

Build a native Windows app equivalent to `apps/macos` using WinUI 3 + C#, reusing the existing Rust C ABI bridge (`moltis_*`) and shipping MSIX as the primary artifact.

## Decisions

- UI stack: WinUI 3 + C#
- Interop boundary: reuse current Rust C ABI
- Packaging: MSIX
- V1 scope: onboarding + chat + settings

## Architecture

- Add `crates/native-bridge-core` for shared non-FFI bridge logic.
- Keep `crates/swift-bridge` for macOS `staticlib` output.
- Add `crates/windows-bridge` as `cdylib` exporting the same `moltis_*` ABI.
- Add `apps/windows` WinUI app with:
  - `MoltisNative` P/Invoke bindings
  - `MoltisClient` JSON facade
  - State stores mirroring mac app scope

## Work Breakdown

1. Extract shared bridge logic into `crates/native-bridge-core`.
2. Rewire `crates/swift-bridge` to call shared core.
3. Implement `crates/windows-bridge` FFI wrappers with identical symbol surface.
4. Add `scripts/build-windows-bridge.ps1` to build DLL and generate header via `cbindgen`.
5. Scaffold `apps/windows` solution/project and bundle `moltis_bridge.dll`.
6. Implement onboarding/chat/settings flows via C# client facade.
7. Add MSIX packaging config and release artifact upload.

## Public Interfaces

- Preserve existing C ABI names and JSON payload contracts.
- Add Windows bridge deliverables:
  - `moltis_bridge.dll`
  - Import library
  - Generated `moltis_bridge.h`

## Testing

- Rust:
  - `cargo +nightly-2025-11-30 fmt --all -- --check`
  - `cargo +nightly-2025-11-30 clippy -Z unstable-options --workspace --all-features --all-targets --timings -- -D warnings`
  - Targeted bridge integration tests (error envelopes, callbacks, streaming terminal events)
- Windows app:
  - Native integration tests (version/chat/session/provider roundtrips)
  - UI smoke E2E: first-launch onboarding, send/stream message, restart persistence

## CI and Release

- Add Windows app CI job on `windows-latest`:
  - Build bridge DLL
  - Build WinUI app
  - Run integration tests
- Add release job to package, sign, and upload MSIX alongside existing artifacts.

## Acceptance Criteria

- Windows app runs fully in-process with Rust bridge, with no localhost transport for UI communication.
- Onboarding/chat/settings work end to end with persisted sessions.
- ABI compatibility is validated by tests.
- CI and release pipeline produce signed MSIX artifacts.

## Assumptions

- Target is `x86_64-pc-windows-msvc` for v1.
- ARM64 Windows is out of scope for v1.
- Full macOS feature parity is deferred beyond onboarding/chat/settings.
