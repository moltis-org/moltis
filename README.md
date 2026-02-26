<div align="center">

<a href="https://moltis.org"><img src="https://raw.githubusercontent.com/moltis-org/moltis-website/main/favicon.svg" alt="Moltis" width="64"></a>

# Moltis — A Rust-native claw you can trust

One binary — sandboxed, secure, yours.

[![CI](https://github.com/moltis-org/moltis/actions/workflows/ci.yml/badge.svg)](https://github.com/moltis-org/moltis/actions/workflows/ci.yml)
[![codecov](https://codecov.io/gh/moltis-org/moltis/graph/badge.svg)](https://codecov.io/gh/moltis-org/moltis)
[![CodSpeed](https://img.shields.io/endpoint?url=https://codspeed.io/badge.json&style=flat&label=CodSpeed)](https://codspeed.io/moltis-org/moltis)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/Rust-1.91%2B-orange.svg)](https://www.rust-lang.org)
[![Discord](https://img.shields.io/discord/1469505370169933837?color=5865F2&label=Discord&logo=discord&logoColor=white)](https://discord.gg/XnmrepsXp5)

[Installation](#installation) • [Comparison](#comparison) • [Architecture](#architecture--crate-map) • [Security](#security) • [Features](#features) • [How It Works](#how-it-works) • [Contributing](CONTRIBUTING.md)

</div>

---

Moltis recently hit [the front page of Hacker News](https://news.ycombinator.com/item?id=46993587). Please [open an issue](https://github.com/moltis-org/moltis/issues) for any friction at all. I'm focused on making Moltis excellent.

**Secure by design** — Your keys never leave your machine. Every command runs in a sandboxed container, never on your host.

**Your hardware** — Runs on a Mac Mini, a Raspberry Pi, or any server you own. One Rust binary, no Node.js, no npm, no runtime.

**Full-featured** — Voice, memory, scheduling, Telegram, Discord, browser automation, MCP servers — all built-in. No plugin marketplace to get supply-chain attacked through.

**Auditable** — The agent loop + provider model fits in ~5K lines. The core (excluding the optional web UI) is ~121K lines across modular crates you can audit independently, with 2,300+ tests and zero `unsafe` code\*.

## Installation

```bash
# One-liner install script (macOS / Linux)
curl -fsSL https://www.moltis.org/install.sh | sh

# macOS / Linux via Homebrew
brew install moltis-org/tap/moltis

# Docker (multi-arch: amd64/arm64)
docker pull ghcr.io/moltis-org/moltis:latest

# Or build from source
cargo install moltis --git https://github.com/moltis-org/moltis
```

## Comparison

| | OpenClaw | PicoClaw | NanoClaw | ZeroClaw | **Moltis** |
|---|---|---|---|---|---|
| Language | TypeScript | Go | TypeScript | Rust | **Rust** |
| Agent loop | ~430K LoC | Small | ~500 LoC | ~3.4K LoC | **~5K LoC** (`runner.rs` + `model.rs`) |
| Full codebase | — | — | — | 1,000+ tests | **~124K LoC** (2,300+ tests) |
| Runtime | Node.js + npm | Single binary | Node.js | Single binary (3.4 MB) | **Single binary (44 MB)** |
| Sandbox | App-level | — | Docker | Docker | **Docker + Apple Container** |
| Memory safety | GC | GC | GC | Ownership | **Ownership, zero `unsafe`\*** |
| Auth | Basic | API keys | None | Token + OAuth | **Password + Passkey + API keys + Vault** |
| Voice I/O | Plugin | — | — | — | **Built-in (15+ providers)** |
| MCP | Yes | — | — | — | **Yes (stdio + HTTP/SSE)** |
| Hooks | Yes (limited) | — | — | — | **15 event types** |
| Skills | Yes (store) | Yes | Yes | Yes | **Yes (+ OpenClaw Store)** |
| Memory/RAG | Plugin | — | Per-group | SQLite + FTS | **SQLite + FTS + vector** |

\* `unsafe` is denied workspace-wide. The only exceptions are opt-in FFI wrappers behind the `local-embeddings` feature flag, not part of the core.

> [Full comparison with benchmarks →](https://docs.moltis.org/comparison.html)

## Architecture — Crate Map

**Core** (always compiled):

| Crate | LoC | Role |
|-------|-----|------|
| `moltis` (cli) | 2.4K | Entry point, CLI commands |
| `moltis-agents` | 20.1K | LLM providers, agent loop, streaming |
| `moltis-gateway` | 29.2K | HTTP/WS server, RPC, auth |
| `moltis-chat` | 10.2K | Chat engine, agent orchestration |
| `moltis-tools` | 13.4K | Tool execution, sandbox |
| `moltis-config` | 5.1K | Configuration, validation |
| `moltis-sessions` | 2.7K | Session persistence |
| `moltis-plugins` | 1.4K | Hook dispatch, plugin formats |
| `moltis-common` | 0.8K | Shared utilities |

**Optional** (feature-gated or additive):

| Category | Crates | Combined LoC |
|----------|--------|-------------|
| Web UI | `moltis-web` | 4.3K |
| Voice | `moltis-voice` | 4.7K |
| Memory | `moltis-memory`, `moltis-qmd` | 5.8K |
| Channels | `moltis-telegram`, `moltis-msteams`, `moltis-discord`, `moltis-channels` | 6.4K |
| Browser | `moltis-browser` | 4.8K |
| Scheduling | `moltis-cron` | 3.8K |
| Extensibility | `moltis-mcp`, `moltis-skills` | 7.4K |
| Auth/OAuth | `moltis-oauth`, `moltis-onboarding`, `moltis-vault` | 2.8K |
| Metrics | `moltis-metrics` | 1.7K |
| Other | `moltis-projects`, `moltis-routing`, `moltis-protocol`, `moltis-media`, `moltis-canvas`, `moltis-auto-reply` | 2.4K |

Use `--no-default-features --features lightweight` for constrained devices (Raspberry Pi, etc.).

## Security

- **Zero `unsafe` code\*** — denied workspace-wide; only opt-in FFI behind `local-embeddings` flag
- **Sandboxed execution** — Docker + Apple Container, per-session isolation
- **Secret handling** — `secrecy::Secret`, zeroed on drop, redacted from tool output
- **Authentication** — password + passkey (WebAuthn), rate-limited, per-IP throttle
- **SSRF protection** — DNS-resolved, blocks loopback/private/link-local
- **Origin validation** — rejects cross-origin WebSocket upgrades
- **Hook gating** — `BeforeToolCall` hooks can inspect/block any tool invocation

See [Security Architecture](https://docs.moltis.org/security.html) for details.

## Features

- **AI Gateway** — Multi-provider LLM support (OpenAI Codex, GitHub Copilot, Local), streaming responses, agent loop with sub-agent delegation, parallel tool execution
- **Communication** — Web UI, Telegram, Microsoft Teams, Discord, API access, voice I/O (8 TTS + 7 STT providers), mobile PWA with push notifications
- **Memory & Context** — Per-agent memory workspaces, embeddings-powered long-term memory, hybrid vector + full-text search, session persistence with auto-compaction, project context
- **Extensibility** — MCP servers (stdio + HTTP/SSE), skill system, 15 lifecycle hook events with circuit breaker, destructive command guard
- **Security** — Encryption-at-rest vault (XChaCha20-Poly1305 + Argon2id), password + passkey + API key auth, sandbox isolation, SSRF/CSWSH protection
- **Operations** — Cron scheduling, OpenTelemetry tracing, Prometheus metrics, cloud deploy (Fly.io, DigitalOcean), Tailscale integration

## How It Works

Moltis is a **local-first AI gateway** — a single Rust binary that sits
between you and multiple LLM providers. Everything runs on your machine; no
cloud relay required.

```
┌─────────────┐  ┌─────────────┐  ┌─────────────┐
│   Web UI    │  │  Telegram   │  │  Discord    │
└──────┬──────┘  └──────┬──────┘  └──────┬──────┘
       │                │                │
       └────────┬───────┴────────┬───────┘
                │   WebSocket    │
                ▼                ▼
        ┌─────────────────────────────────┐
        │          Gateway Server         │
        │   (Axum · HTTP · WS · Auth)     │
        ├─────────────────────────────────┤
        │        Chat Service             │
        │  ┌───────────┐ ┌─────────────┐  │
        │  │   Agent   │ │    Tool     │  │
        │  │   Runner  │◄┤   Registry  │  │
        │  └─────┬─────┘ └─────────────┘  │
        │        │                        │
        │  ┌─────▼─────────────────────┐  │
        │  │    Provider Registry      │  │
        │  │  Multiple providers       │  │
        │  │  (Codex · Copilot · Local)│  │
        │  └───────────────────────────┘  │
        ├─────────────────────────────────┤
        │  Sessions  │ Memory  │  Hooks   │
        │  (JSONL)   │ (SQLite)│ (events) │
        └─────────────────────────────────┘
                       │
               ┌───────▼───────┐
               │    Sandbox    │
               │ Docker/Apple  │
               │  Container    │
               └───────────────┘
```

See [Quickstart](https://docs.moltis.org/quickstart.html) for gateway startup, message flow, sessions, and memory details.

## Getting Started

### Build & Run

```bash
git clone https://github.com/moltis-org/moltis.git
cd moltis
cargo build --release
cargo run --release
```

Open `https://moltis.localhost:3000`. On first run, a setup code is printed to
the terminal — enter it in the web UI to set your password or register a passkey.

Optional flags: `--config-dir /path/to/config --data-dir /path/to/data`

### Docker

```bash
# Docker / OrbStack
docker run -d \
  --name moltis \
  -p 13131:13131 \
  -p 13132:13132 \
  -v moltis-config:/home/moltis/.config/moltis \
  -v moltis-data:/home/moltis/.moltis \
  -v /var/run/docker.sock:/var/run/docker.sock \
  ghcr.io/moltis-org/moltis:latest
```

Open `https://localhost:13131` and complete the setup. See [Docker docs](https://docs.moltis.org/docker.html) for Podman, OrbStack, TLS trust, and persistence details.

### Cloud Deployment

| Provider | Deploy |
|----------|--------|
| DigitalOcean | [![Deploy to DO](https://www.deploytodo.com/do-btn-blue.svg)](https://cloud.digitalocean.com/apps/new?repo=https://github.com/moltis-org/moltis/tree/main) |

**Fly.io** (CLI):

```bash
fly launch --image ghcr.io/moltis-org/moltis:latest
fly secrets set MOLTIS_PASSWORD="your-password"
```

All cloud configs use `--no-tls` because the provider handles TLS termination.
See [Cloud Deploy docs](https://docs.moltis.org/cloud-deploy.html) for details.

## Star History

[![Star History Chart](https://api.star-history.com/svg?repos=moltis-org/moltis&type=date&legend=top-left)](https://www.star-history.com/#moltis-org/moltis&type=date&legend=top-left)

## License

MIT
