# WASM Component Model Tool System — Implementation Plan

## Context

The `wasm` branch just landed a `WasmSandbox` using WASI Preview 1 with built-in
coreutils (~20 commands) and `.wasm` module execution. This plan evolves that into a
Component Model + Preview 2 tool system where agent tools (calc, web_fetch, etc.) can
run as isolated WASM components with typed WIT interfaces, host-provided capabilities,
module caching, resource limits, and structured output.

The work is split into 12 sequential steps, each a single PR-sized chunk.

---

## Step 1: WIT Interface Definitions + Host Bindgen

**Goal:** Define the WIT contracts, add workspace deps, set up host `bindgen!`.

**Files to create:**
- `wit/moltis-tool.wit` — `types` interface (`tool-value` variant, `tool-result`,
  `tool-error`) + `pure-tool` world (exports: `execute`, `parameters-schema`, `name`,
  `description`)
- `wit/moltis-http.wit` — `outgoing-handler` interface (`http-request`,
  `http-response`, `http-error`, `handle` func) + `http-tool` world (imports handler,
  exports tool functions)
- `crates/tools/src/wasm_component.rs` — `wasmtime::component::bindgen!` for both
  worlds; `marshal_tool_result()` converting WIT `tool-value` to `serde_json::Value`

**Files to modify:**
- `Cargo.toml` (root) — add `wit-bindgen = "0.41"`, `wit-bindgen-rt = "0.41"` to
  workspace deps; add `component-model` feature to wasmtime:
  `wasmtime = { features = ["component-model"], version = "30" }`
- `crates/tools/Cargo.toml` — `wasm` feature gains `"wasmtime/component-model"`
- `crates/tools/src/lib.rs` — `#[cfg(feature = "wasm")] pub mod wasm_component;`

**Tests:** Verify bindgen compiles, `marshal_tool_result` handles all `tool-value`
variants (text, number, integer, boolean, json).

---

## Step 2: Component Engine + Module Cache

**Goal:** `WasmComponentEngine` with fuel/epoch config + in-memory compiled component
cache (avoids `unsafe` deserialization that `unsafe_code = "deny"` blocks).

**Files to create:**
- `crates/tools/src/wasm_engine.rs` — `WasmComponentEngine` struct holding
  `wasmtime::Engine` (with `wasm_component_model(true)`, `consume_fuel(true)`,
  `epoch_interruption(true)`) + `RwLock<HashMap<[u8; 32], Component>>` in-memory cache
  keyed by sha256 of wasm bytes. `compile_component(&self, wasm_bytes) -> Component`
  checks cache first. Also `compile_module()` for backward-compat P1 .wasm files.

**Files to modify:**
- `crates/tools/src/sandbox.rs` — `WasmSandbox::new()` creates
  `WasmComponentEngine` (shared `Arc`) instead of bare `wasmtime::Engine`
- `crates/tools/src/lib.rs` — add module

**Note:** We skip disk-based serialization cache because `Module::deserialize` /
`Component::deserialize` are `unsafe` and the workspace denies `unsafe_code`. Built-in
tools compile from `include_bytes!` once at startup and stay in the `Arc` cache.
User-provided `.wasm` files compile each time (still fast for small modules).

**Tests:** Compile+cache round-trip; cache hit on second call; different bytes =
different entry; concurrent access.

---

## Step 3: Resource Limiter + Per-Tool Fuel Budgets

**Goal:** `WasmResourceLimiter` for memory growth control; per-tool fuel/memory config.

**Files to create:**
- `crates/tools/src/wasm_limits.rs` — `WasmResourceLimiter` implementing
  `wasmtime::ResourceLimiter` (caps `memory_growing`, `table_growing`);
  `WasmToolLimits` struct with `default_memory: usize` (16 MB),
  `default_fuel: u64` (1M), `overrides: HashMap<String, ToolLimitOverride>`;
  `resolve(tool_name) -> (fuel, memory)` method.

**Files to modify:**
- `crates/config/src/schema.rs` — add `wasm_tool_limits: Option<WasmToolLimitsConfig>`
  to `SandboxConfig` with `default_memory`, `default_fuel`, `tool_overrides` map
- `crates/config/src/validate.rs` — register new keys in `build_schema_map()`
- `crates/tools/src/lib.rs` — add module

**Default limits:**

| Tool | Fuel | Memory |
|------|------|--------|
| default | 1M | 16 MB |
| calc | 100K | 1 MB |
| web_fetch | 10M | 32 MB |
| web_search | 10M | 32 MB |
| show_map | 10M | 64 MB |
| location | 5M | 16 MB |

**Tests:** Memory growth beyond limit rejected; fuel exhaustion returns error; override
resolution (tool-specific > default); config deser.

---

## Step 4: WasmToolRunner — Host-Side AgentTool Adapter

**Goal:** Adapter making any WASM component implementing the `pure-tool` or `http-tool`
WIT world look like an `AgentTool`.

**Files to create:**
- `crates/tools/src/wasm_tool_runner.rs` — `WasmToolRunner` struct holding
  `Arc<WasmComponentEngine>`, `Component` (pre-compiled), cached name/description/
  schema, fuel budget, memory limit. Implements `AgentTool`: `execute()` runs in
  `spawn_blocking`, creates fresh `Store` with `ResourceLimiter`, sets fuel, starts
  epoch thread, instantiates component, calls `execute(params_json)`, marshals result.

**Files to modify:**
- `crates/agents/src/tool_registry.rs` — add `ToolSource::Wasm { component_hash }`
  variant; add `register_wasm()` method
- `crates/tools/src/lib.rs` — add module

**Key pattern:** One `Component` (compiled once, cached), many `Store`s (one per
invocation, cheap). The `Store` owns the `WasmResourceLimiter` and fuel budget. The
epoch thread is per-invocation with configurable timeout.

**Tests:** Run a minimal pure-tool component through the runner; test fuel exhaustion;
test structured error return; verify `ToolSource::Wasm` in `list_schemas()`.

---

## Step 5: Convert calc to WASM Component (Proof of Concept)

**Goal:** First real WASM tool. End-to-end validation of the entire pipeline.

**Files to create:**
- `crates/wasm-tools/calc/Cargo.toml` — `crate-type = ["cdylib"]`, deps:
  `wit-bindgen`, `wit-bindgen-rt`
- `crates/wasm-tools/calc/src/lib.rs` — `wit_bindgen::generate!` for `pure-tool`
  world; `CalcComponent` implementing `Guest` with duplicated evaluation logic
  (~300 lines, pure functions, rarely changes)
- `crates/tools/src/embedded_wasm.rs` — `include_bytes!` for release, `fs::read`
  fallback for debug

**Files to modify:**
- `Cargo.toml` (root) — add `crates/wasm-tools/calc` to workspace members
- `rust-toolchain.toml` — add `targets = ["wasm32-wasip2"]`
- `justfile` — add `wasm-tools` recipe:
  `cargo build --target wasm32-wasip2 -p moltis-wasm-calc --release`
- `crates/gateway/src/server.rs` — register `WasmToolRunner` wrapping calc (initially
  as `"calc_wasm"` alongside native `"calc"` for side-by-side comparison)

**Build note:** The wasm-tools crate is compiled to a different target and embedded via
`include_bytes!`. CI builds it in a separate step before the host build. It is NOT a
Cargo dependency of any host crate.

**Tests:** Compare native CalcTool vs WASM output for 20+ expressions; test fuel
exhaustion on pathological input; verify identical JSON schema.

---

## Step 6: Host HTTP Capability + SSRF Extraction

**Goal:** Implement host side of `moltis:http/outgoing-handler`. Centralize SSRF
in a shared module.

**Files to create:**
- `crates/tools/src/ssrf.rs` — extract `ssrf_check()`, `is_private_ip()`,
  `is_ssrf_allowed()` from `web_fetch.rs`
- HTTP host impl in `crates/tools/src/wasm_component.rs` — `HttpHostImpl`
  using `reqwest::blocking::Client` (inside `spawn_blocking`); calls `ssrf_check()`
  before every request; enforces `max_response_bytes`; optional domain allowlist
  (future: trusted-network branch populates this)

**Files to modify:**
- `crates/tools/src/web_fetch.rs` — import from `ssrf` module instead of local fns
- `crates/tools/src/wasm_engine.rs` — add `create_http_linker()` linking HTTP handler
  + WASI P2 for http-tool world
- `crates/tools/src/lib.rs` — add `pub mod ssrf;`

**Tests:** HTTP host blocks loopback/private/link-local IPs; respects allowlist;
enforces max bytes; timeout; existing web_fetch SSRF tests pass after extraction.

---

## Step 7: Convert web_fetch + web_search to WASM Components

**Goal:** WASM tools using host HTTP capability for all network access.

**Files to create:**
- `crates/wasm-tools/web-fetch/Cargo.toml` + `src/lib.rs` — `http-tool` world;
  uses imported `handle()` for HTTP; URL parsing + redirect following in guest;
  HTML-to-text extraction in guest; no SSRF (host enforces)
- `crates/wasm-tools/web-search/Cargo.toml` + `src/lib.rs` — `http-tool` world;
  Brave Search + DuckDuckGo; API key passed in params
- `crates/tools/src/wasm_tool_runner.rs` — add `CachingWasmToolRunner` wrapping
  runner + `Mutex<HashMap<String, CacheEntry>>` TTL cache (host-side, since WASM
  components are stateless)

**Files to modify:**
- `crates/tools/src/embedded_wasm.rs` — add embedded bytes
- `Cargo.toml` (root) — add workspace members
- `justfile` — update `wasm-tools` recipe
- `crates/gateway/src/server.rs` — register WASM web tools

**Tests:** Mock HTTP via host handler; compare native vs WASM output; test caching;
test SSRF flows through host.

---

## Step 8: Capability-Based Permissions via WIT Imports

**Goal:** Tools declare imports in WIT world; host verifies at construction and grants
only declared capabilities.

**Files to create:**
- `wit/moltis-fs.wit` — `filesystem` interface (`read-file`, `write-file`, `list-dir`,
  `file-exists`) scoped to sandbox root; `fs-tool` world
- `crates/tools/src/wasm_capabilities.rs` — `WasmCapability` enum (`Pure`, `Http`,
  `Filesystem`, `KeyValueStore`); `WasmToolManifest` (name, world, capabilities, fuel,
  memory); manifest verification at `WasmToolRunner` construction

**Files to modify:**
- `crates/tools/src/wasm_tool_runner.rs` — capability verification in constructor
- `crates/tools/src/wasm_component.rs` — filesystem host impl (scoped to sandbox root,
  path escape prevention via canonicalization)

**Tests:** Capability mismatch rejects component; pure-tool can't call HTTP; http-tool
can't access filesystem; fs-tool gets scoped access only.

---

## Step 9: Convert location + show_map to WASM Components

**Goal:** Location uses host HTTP for geocoding. ShowMap uses host HTTP + image crate.

**Files to create:**
- `crates/wasm-tools/location/Cargo.toml` + `src/lib.rs` — `http-tool` world;
  Nominatim geocoding via host HTTP; coordinate handling in guest
- `crates/wasm-tools/show-map/Cargo.toml` + `src/lib.rs` — `http-tool` world;
  OSM tile fetch via host HTTP; `image` crate (`default-features = false,
  features = ["jpeg", "png"]`) for compositing; returns base64 PNG

**Files to modify:**
- `crates/tools/src/embedded_wasm.rs` — add embedded bytes
- `Cargo.toml` (root) — add workspace members
- `crates/gateway/src/server.rs` — register tools

**Note:** `image` crate compiles to `wasm32-wasip2` without SIMD. ShowMap gets 64 MB
memory limit (Step 3 per-tool override).

**Tests:** Location geocoding via mock; ShowMap compositing produces valid PNG;
marker rendering at various zoom levels; compare native vs WASM output.

---

## Step 10: SandboxRouter Integration — Per-Session WASM Tool Sets

**Goal:** WASM-backend sessions get WASM tools; container sessions keep native tools.

**Files to modify:**
- `crates/tools/src/sandbox.rs` — add `wasm_engine: Option<Arc<WasmComponentEngine>>`
  and `wasm_tools: Option<Vec<Arc<dyn AgentTool>>>` to `SandboxRouter`; add
  `tool_variant(session_key) -> ToolVariant`; enum `ToolVariant::Wasm` / `Native`
- `crates/agents/src/tool_registry.rs` — add `swap_tool(name, tool)` and
  `clone_with_wasm_tools(wasm_tools)` methods
- `crates/gateway/src/server.rs` — register both variants; chat runtime checks
  `SandboxRouter::tool_variant()` to decide which registry clone to use

**Migration:** Native tools remain default. WASM tools activate only when
`backend = "wasm"` or a per-session override selects WASM.

**Tests:** WASM session gets WASM tools; container session gets native tools; session
variant switching; per-session override works.

---

## Step 11: Structured Output — Typed WIT Returns

**Goal:** Per-tool typed returns. Host marshals to JSON for LLM.

**Files to modify:**
- `wit/moltis-tool.wit` — per-tool result types: `calc-result { result: f64,
  normalized-expr: string }`, `fetch-result { url, content-type, content, truncated,
  original-length }`, `search-result`, etc.
- Guest crates — return typed structs instead of JSON strings
- `crates/tools/src/wasm_component.rs` — typed marshaling fns per result type
- `crates/tools/src/wasm_tool_runner.rs` — dispatch marshaling by tool name

**Backward compat:** WASM tools produce same JSON shape as native. LLM sees no change.

**Tests:** Typed results marshal to expected JSON; error types marshal; round-trip
equivalence with native tools.

---

## Step 12: WasmSandbox P2 Upgrade + Shell Parsing Removal

**Goal:** Upgrade `exec_wasm_module()` from P1 to Component Model P2. Add structured
`exec_tool()` bypassing shell parsing.

**Files to modify:**
- `crates/tools/src/sandbox.rs`:
  - `exec_wasm_module()` — detect component vs core module (magic bytes). Components
    use `component::Linker` + `wasmtime_wasi::add_to_linker_sync`. Core modules fall
    back to P1.
  - New `exec_tool(&self, component_bytes, params: Value, id: &SandboxId) -> Result<Value>` —
    direct component instantiation. No `shell-words`, no `CommandSegment`, no redirects.
  - Existing shell-based `exec()` kept for container sessions.
- `crates/tools/src/wasm_engine.rs` — `is_component(wasm_bytes) -> bool` detection

**Tests:** P2 component execution; P1 fallback; structured `exec_tool()`; fuel/epoch
with P2; all existing WasmSandbox tests pass.

---

## Build Toolchain

| Change | Where |
|--------|-------|
| `targets = ["wasm32-wasip2"]` | `rust-toolchain.toml` |
| `wit-bindgen = "0.41"` | `Cargo.toml` workspace deps |
| `wit-bindgen-rt = { version = "0.41", features = ["bitflags"] }` | `Cargo.toml` workspace deps |
| `wasmtime = { version = "30", features = ["component-model"] }` | `Cargo.toml` workspace deps |
| `just wasm-tools` recipe | `justfile` |
| CI: build wasm-tools before host | CI config |

## Critical Files

| File | Role |
|------|------|
| `crates/tools/src/sandbox.rs` | WasmSandbox, SandboxRouter, WasmBuiltins |
| `crates/agents/src/tool_registry.rs` | AgentTool trait, ToolRegistry |
| `crates/tools/src/calc.rs` | First conversion candidate (line 390: `evaluate_expression`) |
| `crates/tools/src/web_fetch.rs` | SSRF extraction source |
| `crates/gateway/src/server.rs` | Tool registration (~line 2955) |
| `crates/config/src/schema.rs` | SandboxConfig (line 1511) |
| `crates/config/src/validate.rs` | Schema validation map |

## Verification (each step)

1. `just wasm-tools` (from step 5) — builds guest components
2. `just format && just release-preflight` — fmt + clippy
3. `cargo test` — all tests pass
4. Side-by-side: WASM tool output == native tool output for shared inputs
5. `./scripts/local-validate.sh` when PR exists
