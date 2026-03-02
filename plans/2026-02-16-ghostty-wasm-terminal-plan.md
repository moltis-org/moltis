# Ghostty WASM Terminal Research and Integration Plan

Date: 2026-02-16  
Status: Draft for later implementation  
Scope: Web UI terminal renderer for `Settings > Terminal` in Moltis

## Goal

Evaluate whether Moltis should embed Ghostty-based WASM terminal rendering in the web UI, while keeping Moltis as a single binary that serves bundled assets.

## What Was Researched

1. Upstream Ghostty wasm support:
   - `libghostty-vt` has a wasm build path (`initWasm` in Ghostty build system).
   - Ghostty includes wasm helper APIs under `include/ghostty/vt/wasm.h`.
   - Current API is low-level C ABI style and intended to be wrapped by higher-level browser code.

2. `ghostty-web` package:
   - Provides xterm-like browser API over Ghostty VT parser.
   - Uses a patch against Ghostty today.
   - Explicitly notes intent to move to official upstream wasm distribution when available.

3. Bundle size impact (from `ghostty-web@0.4.0` package metadata):
   - `ghostty-vt.wasm`: `423,045` bytes
   - `ghostty-web.js`: `681,918` bytes
   - Minimal runtime payload (js + wasm): `1,104,963` bytes (~1.05 MiB, pre-compression)
   - Full dist payload listed: `1,799,426` bytes (~1.72 MiB)

## Feasibility Assessment

Short answer: feasible, but should be feature-gated first.

Rationale:
- Technical integration is straightforward if we keep Moltis backend transport unchanged and only swap renderer in the browser.
- The binary size increase is noticeable but acceptable for desktop/server distribution.
- Main risk is supply chain and maintenance because `ghostty-web` currently depends on a Ghostty patch.

## Architecture Fit for Moltis

Recommended boundary:

1. Keep backend terminal transport exactly as-is:
   - `process` tool (tmux-based session lifecycle)
   - host/sandbox routing logic
   - websocket/RPC plumbing

2. Add frontend renderer abstraction:
   - `plain` renderer (current baseline)
   - `ghostty` renderer (optional)

3. Renderer selection:
   - Build/config flag and runtime setting, default to `plain`.
   - Lazy-load Ghostty assets only when `Settings > Terminal` is opened.

This keeps Ghostty as a UI concern, not a backend concern.

## Proposed Implementation Phases

## Phase 0: Prep

1. Add `terminal_renderer` config enum (`plain | ghostty`), default `plain`.
2. Add feature gate in UI and server-injected gon/config payload.
3. Add no-op fallback path if renderer init fails.

## Phase 1: Asset Integration

1. Vendor pinned Ghostty web assets into gateway assets tree:
   - `ghostty-web.js`
   - `ghostty-vt.wasm`
2. Record source version and checksums in a small manifest file.
3. Ensure cache/versioned asset URLs align with existing asset pipeline.

## Phase 2: UI Wiring

1. Implement renderer adapter in `page-terminal.js`:
   - `init(container)`
   - `write(data)`
   - `onData(cb)`
   - `resize(cols, rows)`
   - `dispose()`
2. Keep existing terminal session controls and transport unchanged.
3. Add runtime fallback to `plain` renderer on wasm load/init errors.

## Phase 3: Validation

1. Add E2E coverage for:
   - Terminal open/connect
   - Input round-trip
   - Resize handling
   - Reconnect/session continuity
   - Fallback behavior when wasm load fails
2. Add basic performance checks:
   - First open latency
   - Render throughput sanity for bursty output

## Phase 4: Rollout

1. Keep disabled by default initially.
2. Enable experimentally via config for advanced users.
3. Promote to broader use only after stability signals are good.

## Risks and Mitigations

1. Upstream volatility (`ghostty-web` patch dependency):
   - Mitigation: pin version + checksums, track upstream libghostty wasm progress.

2. Binary size growth:
   - Mitigation: lazy-load assets, include only minimal runtime files, keep feature-gated.

3. Browser compatibility edge cases:
   - Mitigation: fallback renderer path always available.

4. Operational debugging complexity:
   - Mitigation: add explicit renderer mode telemetry/logging in UI debug info.

## Recommendation

Proceed, but behind a feature gate and with strict pinning.

If minimizing maintenance and binary size is priority, defer default enablement until Ghostty ships an official stable wasm distribution for this use case.

## Open Questions

1. Should Ghostty renderer be available in all builds or only optional release profiles?
2. Do we want user-facing runtime switch in Settings, or only config-level switch first?
3. Should we include font rendering defaults tuned for terminal fidelity, or keep current defaults?
4. How much additional startup latency is acceptable on first terminal open?

## Sources

- https://raw.githubusercontent.com/ghostty-org/ghostty/main/include/ghostty/vt/wasm.h
- https://raw.githubusercontent.com/ghostty-org/ghostty/main/src/build/GhosttyLibVt.zig
- https://raw.githubusercontent.com/coder/ghostty-web/main/README.md
- https://raw.githubusercontent.com/coder/ghostty-web/main/patches/ghostty-wasm-api.patch
- https://data.jsdelivr.com/v1/package/npm/ghostty-web@0.4.0
- https://mitchellh.com/writing/libghostty-is-coming
