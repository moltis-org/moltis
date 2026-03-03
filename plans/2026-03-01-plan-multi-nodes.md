# Multi-Node Implementation Plan

**Status:** In Progress
**Priority:** High
**Date:** 2026-03-01
**Branch:** `multi-nodes`
**Scope:** Implement OpenClaw-compatible multi-node support: persistent pairing, headless node host, command routing to remote nodes, CLI commands, and web UI.

## Background

OpenClaw nodes are companion devices (iOS, Android, macOS, headless) that connect to a Gateway via WebSocket with `role: "node"`. They expose capabilities (`canvas.*`, `camera.*`, `system.run`, `system.which`, `system.notify`, `location.get`, `sms.send`) via `node.invoke`. The key feature is **remote command execution**: binding `tools.exec.host = "node"` routes shell commands to a remote machine while the LLM runs locally.

Moltis already has: `NodeRegistry`, `NodeSession`, `PairingState`, `node.invoke` RPC, mDNS discovery, GraphQL types, iOS schema. Missing: persistent pairing, device token auth, headless node host binary, command routing, CLI commands, web UI.

## Architecture

```
+------------------+     WebSocket      +----------------+
| Headless Node    |------------------>|  Gateway        |
| (system.run)     |  role: "node"     |                |
+------------------+                   |  ExecTool      |
                                       |  -> if host=   |
+------------------+     WebSocket     |     "node"     |
| iOS/macOS Node   |------------------>|  -> node.invoke|
| (canvas,camera)  |  role: "node"     |                |
+------------------+                   +----------------+
```

---

## Phase 0: Persist Pairing & Device Tokens to DB

**Goal:** Survive gateway restarts. Currently `PairingState` is in-memory only.

### 0.1 DB Migration

**File:** `crates/gateway/migrations/20260301100000_device_pairing.sql`

Tables: `paired_devices`, `pair_requests`, `device_tokens`.
- `paired_devices`: device_id PK, display_name, platform, public_key, status (active/revoked), timestamps
- `pair_requests`: id PK, device_id, nonce, status (pending/approved/rejected/expired), expires_at
- `device_tokens`: token_hash PK, token_prefix, device_id FK, scopes (JSON), issued_at, revoked flag

### 0.2 Persistent PairingStore

**File:** `crates/gateway/src/pairing.rs` — refactor to use SQLite pool

Replace `HashMap<String, PairRequest>` / `HashMap<String, DeviceToken>` with `SqlitePool`-backed methods. Keep in-memory cache for fast reads.

Key methods remain same API: `request_pair()`, `approve()`, `reject()`, `list_pending()`, `list_devices()`, `rotate_token()`, `revoke_token()`. Add `verify_device_token(raw_token)` with SHA-256 hash lookup.

### 0.3 Device Token Auth in WS Handshake

**File:** `crates/gateway/src/ws.rs`

Check `connect_params.auth.device_token` during handshake. If present, verify against `device_tokens` table. If valid, populate scopes. If invalid/revoked, reject 401.

### 0.4 Wire Into Server Startup

**File:** `crates/gateway/src/server.rs`

Run new migration, create `PairingStore::new(pool)`, pass to `GatewayInner`.

---

## Phase 1: Config — `tools.exec.node` and `tools.exec.host`

### 1.1 Schema Changes

**File:** `crates/config/src/schema.rs`

Add to `ExecConfig`: `host: String` (default "local", valid: "local"|"node"), `node: Option<String>` (node id/name).

### 1.2 Validation Changes

**File:** `crates/config/src/validate.rs`

Add `("host", Leaf)` and `("node", Leaf)` to `exec()` schema map. Add semantic check for valid `host` values.

---

## Phase 2: Command Routing to Remote Nodes

**Goal:** When `tools.exec.host = "node"`, forward shell commands via `node.invoke` with `system.run`.

### 2.1 Node Command Forwarder

**File:** `crates/gateway/src/node_exec.rs` (new)

`NodeExecForwarder` builds `system.run` args, calls `node.invoke` internally, parses stdout/stderr/exitCode.

### 2.2 ExecTool Integration

**File:** `crates/tools/src/exec.rs`

New branch: check if session has node binding, if yes route through `NodeExecForwarder`.

### 2.3 Env Stripping for Remote Routing

Safe allowlist: `TERM`, `LANG`, `LC_*`, `COLORTERM`, `NO_COLOR`, `FORCE_COLOR`.
Block: `DYLD_*`, `LD_*`, `NODE_OPTIONS`, `PYTHON*`, `PERL*`, `RUBYOPT`, `SHELLOPTS`, `PS4`.

---

## Phase 3: Headless Node Host

### 3.1 New Crate: `crates/node-host/`

Depends on: `moltis-protocol`, `tokio`, `tokio-tungstenite`, `serde_json`, `sha2`, `tracing`.

### 3.2 WS Client Loop

Connect to gateway WS with `role: "node"`, capabilities `["system.run", "system.which"]`. Handle `node.invoke.request` events, return `node.invoke.result`.

### 3.3 Local Approvals

`~/.moltis/exec-approvals.json`: allowlist-based security. Security levels: deny, allowlist (default), full.

### 3.4 Config Persistence

`~/.moltis/node.json`: nodeId, displayName, gatewayHost, gatewayPort, deviceToken, tls, securityLevel.

### 3.5 Service Install

macOS: launchd plist. Linux: systemd --user unit. `moltis node install/uninstall`.

### 3.6 CLI Commands

**File:** `crates/cli/src/node_commands.rs` — `moltis node run|install|status|stop|restart|uninstall`
**File:** `crates/cli/src/nodes_commands.rs` — `moltis nodes list|pending|approve|describe|rename|invoke|run`

---

## Phase 4: Web UI — Nodes Page

### 4.1 Page at `/nodes`

Sections: Connected Nodes (cards), Pending Pairing (approve/reject), Paired Devices (revoke), Exec Routing config, Setup Guide.

### 4.2 Real-Time Updates

Via event bus: `presence` for connect/disconnect, `node.pair.requested/resolved` for pairing.

### 4.3 GonData

Add connected node count + pending pairing count + exec config to gon.

---

## File Change Summary

### New Files
| File | Purpose |
|------|---------|
| `crates/gateway/migrations/20260301100000_device_pairing.sql` | DB tables |
| `crates/gateway/src/node_exec.rs` | Node command forwarder |
| `crates/node-host/Cargo.toml` | Headless node host crate |
| `crates/node-host/src/lib.rs` | NodeHost + WS client loop |
| `crates/node-host/src/exec_runner.rs` | Local command runner |
| `crates/node-host/src/approvals.rs` | Local approval manager |
| `crates/node-host/src/service.rs` | Service install/uninstall |
| `crates/cli/src/node_commands.rs` | `moltis node` subcommands |
| `crates/cli/src/nodes_commands.rs` | `moltis nodes` subcommands |
| `crates/web/src/assets/js/page-nodes.js` | Web UI nodes page |

### Modified Files
| File | Changes |
|------|---------|
| `crates/gateway/src/pairing.rs` | SQLite-backed persistence |
| `crates/gateway/src/ws.rs` | Device token auth in handshake |
| `crates/gateway/src/state.rs` | Wire PairingStore |
| `crates/gateway/src/server.rs` | Migration, PairingStore init, gon |
| `crates/gateway/src/methods/pairing.rs` | Use persistent store |
| `crates/config/src/schema.rs` | Add host, node to ExecConfig |
| `crates/config/src/validate.rs` | Schema map + checks |
| `crates/tools/src/exec.rs` | Node routing branch |
| `crates/cli/src/main.rs` | Add Node + Nodes commands |
| `crates/web/src/assets/index.html` | Nav link |
| `crates/web/src/assets/js/routes.js` | Route |
| `Cargo.toml` | Workspace member |

## Implementation Order

1. Phase 0 — DB migration + persistent pairing (prerequisite)
2. Phase 1 — Config schema (small, unblocks Phase 2)
3. Phase 2 — Command routing (core value)
4. Phase 3 — Node host binary (enables end-to-end testing)
5. Phase 4 — Web UI
