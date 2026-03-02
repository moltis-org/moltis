# Comparison

How Moltis compares to other open-source AI agent frameworks.

> **Disclaimer:** This comparison reflects publicly available information at the
> time of writing. Projects evolve quickly — check each project's repository for
> the latest details. Contributions to keep this page accurate are welcome.

## At a Glance

| | [OpenClaw](https://github.com/openclaw/openclaw) | [PicoClaw](https://github.com/sipeed/picoclaw) | [NanoClaw](https://github.com/qwibitai/nanoclaw) | [ZeroClaw](https://github.com/zeroclaw-labs/zeroclaw) | **Moltis** |
|---|---|---|---|---|---|
| Language | TypeScript | Go | TypeScript | Rust | **Rust** |
| Agent loop | ~430K LoC | Small | ~500 LoC | ~3.4K LoC | **~5K LoC** |
| Full codebase | — | — | — | 1,000+ tests | **~200K LoC** (3,100+ tests) |
| Runtime | Node.js + npm | Single binary | Node.js | Single binary (3.4 MB) | **Single binary (44 MB)** |
| Sandbox | App-level | — | Docker | Docker | **Docker + Apple Container** |
| Memory safety | GC | GC | GC | Ownership | **Ownership, zero `unsafe`\*** |
| Auth | Basic | API keys | None | Token + OAuth | **Password + Passkey + API keys** |
| Voice I/O | Plugin | — | — | — | **Built-in (15+ providers)** |
| MCP | Yes | — | — | — | **Yes (stdio + HTTP/SSE)** |
| Hooks | Yes (limited) | — | — | — | **15 event types** |
| Skills | Yes (store) | Yes | Yes | Yes | **Yes (+ OpenClaw Store)** |
| Memory/RAG | Plugin | — | Per-group | SQLite + FTS | **SQLite + FTS + vector** |

\* `unsafe` is denied workspace-wide in Moltis. The only exceptions are opt-in
FFI wrappers behind the `local-embeddings` feature flag, not part of the core.

## Architecture Approach

### OpenClaw — Full-featured monolith

OpenClaw is the original and most popular project (~211K stars). It ships as a
Node.js application with 52+ modules, 45+ npm dependencies, and a large surface
area. It has the richest ecosystem of third-party skills and integrations, but
the codebase is difficult to audit end-to-end.

### PicoClaw — Minimal Go binary

PicoClaw targets extreme resource constraints — $10 SBCs, RISC-V boards, and
devices with as little as 10 MB of RAM. It boots in under 1 second on 0.6 GHz
hardware. The trade-off is a narrower feature set: no sandbox isolation, no
built-in memory/RAG, and limited extensibility.

### NanoClaw — Container-first TypeScript

NanoClaw strips away OpenClaw's complexity to deliver a small, readable
TypeScript codebase with first-class container isolation. Agents run in
Linux containers with filesystem isolation. It uses Claude Agent SDK for
sub-agent delegation and per-group CLAUDE.md memory files. The trade-off is
Node.js as a runtime dependency and a smaller feature surface.

### ZeroClaw — Lightweight Rust

ZeroClaw compiles to a tiny 3.4 MB binary with <5 MB RAM usage and sub-10ms
startup. It uses trait-driven architecture with 22+ provider implementations
and 9+ channel integrations. Memory is backed by SQLite with hybrid vector +
FTS search. The focus is on minimal footprint and broad platform support.

### Moltis — Auditable Rust gateway

Moltis prioritizes auditability and defense in depth. The core agent engine
(runner + provider model) is ~5K lines; the core (excluding the optional web UI)
is ~196K lines across 46 modular crates, each independently auditable. Key
differences from ZeroClaw:

- **Larger binary (44 MB)** in exchange for built-in voice I/O, browser
  automation, web UI, and MCP support
- **Apple Container support** in addition to Docker
- **WebAuthn passkey authentication** — not just tokens
- **15 lifecycle hook events** with circuit breaker and dry-run mode
- **Built-in web UI** with real-time streaming, settings management, and
  session branching

## Security Model

| Aspect | OpenClaw | PicoClaw | NanoClaw | ZeroClaw | **Moltis** |
|--------|----------|----------|----------|----------|------------|
| Code sandbox | App-level permissions | None | Docker containers | Docker containers | Docker + Apple Container |
| Secret handling | Environment variables | Environment variables | Environment variables | Encrypted profiles | `secrecy::Secret`, zeroed on drop |
| Auth method | Basic password | API keys only | None (WhatsApp auth) | Token + OAuth | Password + Passkey + API keys |
| SSRF protection | Plugin | — | — | DNS validation | DNS-resolved, blocks loopback/private/link-local/CGNAT |
| WebSocket origin | — | N/A | — | — | Cross-origin rejection |
| `unsafe` code | N/A (JS) | N/A (Go) | N/A (JS) | Minimal | Denied workspace-wide\* |
| Hook gating | — | — | — | Skills-based | `BeforeToolCall` inspect/modify/block |
| Rate limiting | — | — | — | — | Per-IP throttle, strict login limits |

## Performance

| Metric | OpenClaw | PicoClaw | ZeroClaw | **Moltis** |
|--------|----------|----------|----------|------------|
| Binary / dist size | ~28 MB (node_modules) | <10 MB | 3.4 MB | 44 MB |
| Cold start | >30s | <1s | <10ms | ~1s |
| RAM (idle) | >100 MB | <10 MB | <5 MB | ~30 MB |
| Min hardware | Modern desktop | $10 SBC (RISC-V) | $10 SBC | Raspberry Pi 4+ |

Moltis is larger because it bundles a web UI, voice engine, browser automation,
and MCP runtime. Use `--no-default-features --features lightweight` for
constrained devices.

## When to Choose What

**Choose OpenClaw if** you want the largest ecosystem, maximum third-party
skills, and don't mind Node.js as a dependency.

**Choose PicoClaw if** you need to run on extremely constrained hardware
($10 boards, RISC-V) and can accept a minimal feature set.

**Choose NanoClaw if** you want a small, readable TypeScript codebase with
container isolation and don't need voice, MCP, or a web UI.

**Choose ZeroClaw if** you want the smallest possible Rust binary, sub-10ms
startup, and broad channel support without a web UI.

**Choose Moltis if** you want:
- A single auditable Rust binary with built-in web UI
- Voice I/O with 15+ providers (8 TTS + 7 STT)
- MCP server support (stdio + HTTP/SSE)
- WebAuthn passkey authentication
- Apple Container sandbox support (macOS native)
- 15 lifecycle hook events with circuit breaker
- Embeddings-powered long-term memory with hybrid search
- Cron scheduling, browser automation, and Tailscale integration

## Links

- [OpenClaw](https://github.com/openclaw/openclaw) — [Docs](https://docs.openclaw.ai)
- [PicoClaw](https://github.com/sipeed/picoclaw) — [Site](https://picoclaw.net)
- [NanoClaw](https://github.com/qwibitai/nanoclaw)
- [ZeroClaw](https://github.com/zeroclaw-labs/zeroclaw) — [Site](https://zeroclaw.net)
- [Moltis](https://github.com/moltis-org/moltis) — [Docs](https://docs.moltis.org)
