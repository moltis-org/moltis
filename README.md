<div align="center">

<a href="https://moltis.org"><img src="https://raw.githubusercontent.com/moltis-org/moltis/main/website/favicon.svg" alt="Moltis" width="64"></a>

# Moltis — A secure persistent personal agent server in Rust

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

**Full-featured** — Voice, memory, cross-session recall, automatic edit checkpoints, scheduling, Telegram, Discord, browser automation, MCP servers, SSH or node-backed remote exec, managed deploy keys with host pinning in the web UI, a live Settings → Tools inventory, Cursor-compatible project context, and context-file threat scanning — all built-in. No plugin marketplace to get supply-chain attacked through.

**Auditable** — The agent runner and model interface fit in ~7.5K lines, with providers in ~19K more. The Rust workspace is ~270K lines across 59 modular crates you can audit independently, with 470+ Rust files containing tests. Unsafe code is isolated to FFI and precompiled runtime boundaries, not the core agent loop.

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

| | OpenClaw | Hermes Agent | **Moltis** |
|---|---|---|---|
| Primary stack | TypeScript + Swift/Kotlin companion apps | Python + TypeScript TUI/web surfaces | **Rust** |
| Runtime | Node.js + npm/pnpm/bun | Python + uv/pip, optional Node UI pieces | **Single Rust binary** |
| Local checkout size\* | ~1.1M app LoC | ~152K app LoC | **~270K Rust LoC** |
| Architecture | Broad gateway, channel, node, and app ecosystem | CLI/gateway agent with learning loop and research tooling | **Persistent personal agent server with modular crates** |
| Crates/modules | npm packages, extensions, apps | Python packages, plugins, tools, TUI | **59 Rust workspace crates** |
| Sandbox/backends | App-level permissions, browser/node tools | Local, Docker, SSH, Daytona, Singularity, Modal | **Docker/Podman + Apple Container + WASM** |
| Auth/access | Pairing and local gateway controls | CLI and messaging gateway setup | **Password + Passkey + API keys + Vault** |
| Voice I/O | Voice wake and talk modes | Voice memo transcription | **Built-in STT + TTS providers** |
| MCP | Plugin/integration support | MCP integration | **stdio + HTTP/SSE** |
| Skills | Bundled, managed, and workspace skills | Self-improving skills and Skills Hub support | **Bundled/workspace skills + autonomous improvement + OpenClaw import** |
| Memory/RAG | Plugin-backed memory and context engine | Agent-curated memory, session search, user modeling | **SQLite + FTS + vector memory** |

\* LoC measured with `tokei`, excluding `node_modules`, generated build output, `dist`, and `target`.

> [Full comparison in the docs →](https://docs.moltis.org/comparison.html)

## Architecture — Crate Map

Current Rust workspace: ~270K LoC across 59 crates. The table below groups the main crates by role so the architecture stays scannable.

**Core runtime**:

| Crate | LoC | Role |
|-------|-----|------|
| `moltis-gateway` | 37.4K | HTTP/WS server, RPC, auth, startup wiring |
| `moltis-tools` | 37.0K | Tool execution, sandboxing, WASM tools |
| `moltis-providers` | 18.9K | LLM provider implementations |
| `moltis-agents` | 14.5K | Agent loop, streaming, prompt assembly |
| `moltis-chat` | 14.2K | Chat engine, agent orchestration |
| `moltis-config` | 10.3K | Configuration, validation |
| `moltis-httpd` | 9.9K | HTTP server primitives and middleware |
| `moltis` (CLI) | 4.7K | Entry point, CLI commands |
| `moltis-sessions` | 3.5K | Session persistence |
| `moltis-common` | 1.5K | Shared utilities |
| `moltis-service-traits` | 1.2K | Shared service interfaces |
| `moltis-protocol` | 0.7K | Wire protocol types |

**Feature and integration crates**:

| Category | Crates | Combined LoC |
|----------|--------|-------------|
| Channels | `moltis-telegram`, `moltis-whatsapp`, `moltis-discord`, `moltis-msteams`, `moltis-matrix`, `moltis-slack`, `moltis-nostr`, `moltis-channels` | 34.0K |
| Web and APIs | `moltis-web`, `moltis-graphql`, `moltis-webhooks` | 10.8K |
| Extensibility | `moltis-mcp`, `moltis-mcp-agent-bridge`, `moltis-skills`, `moltis-plugins` | 11.5K |
| Memory and context | `moltis-memory`, `moltis-qmd`, `moltis-code-index`, `moltis-projects` | 11.7K |
| Voice and browser | `moltis-voice`, `moltis-browser` | 9.2K |
| Auth and security | `moltis-auth`, `moltis-oauth`, `moltis-vault`, `moltis-secret-store`, `moltis-network-filter`, `moltis-tls` | 8.5K |
| Scheduling and automation | `moltis-cron`, `moltis-caldav`, `moltis-auto-reply` | 4.7K |
| Setup and import | `moltis-provider-setup`, `moltis-openclaw-import`, `moltis-onboarding` | 11.7K |
| Native and node hosts | `moltis-swift-bridge`, `moltis-node-host`, `moltis-courier` | 5.7K |
| WASM tools | `moltis-wasm-precompile`, `moltis-wasm-calc`, `moltis-wasm-web-fetch`, `moltis-wasm-web-search` | 1.4K |
| Supporting crates | `moltis-media`, `moltis-metrics`, `moltis-tailscale`, `moltis-routing`, `moltis-canvas`, `moltis-schema-export`, `benchmarks` | 2.1K |

Use `--no-default-features --features lightweight` for constrained devices (Raspberry Pi, etc.).

## Security

- **Small unsafe surface** — core agent/gateway code stays safe Rust; unsafe is isolated to Swift FFI, local model wrappers, and precompiled WASM boundaries
- **Sandboxed execution** — Docker + Apple Container, per-session isolation
- **Secret handling** — `secrecy::Secret`, zeroed on drop, redacted from tool output
- **Authentication** — password + passkey (WebAuthn), rate-limited, per-IP throttle
- **SSRF protection** — DNS-resolved, blocks loopback/private/link-local
- **Origin validation** — rejects cross-origin WebSocket upgrades
- **Hook gating** — `BeforeToolCall` hooks can inspect/block any tool invocation
- **Supply chain integrity** — [artifact attestations](https://github.com/moltis-org/moltis/attestations), Sigstore keyless signing, GPG signing (YubiKey), SHA-256/SHA-512 checksums

See [Security Architecture](https://docs.moltis.org/security.html) for details.
Verify releases with `gh attestation verify <artifact> -R moltis-org/moltis` or see [Release Verification](https://docs.moltis.org/release-verification.html).

## Features

- **AI Gateway** — Multi-provider LLM support (OpenAI Codex, GitHub Copilot, Local), streaming responses, agent loop with sub-agent delegation, parallel tool execution
- **Communication** — Web UI, Telegram, Microsoft Teams, Discord, API access, voice I/O (8 TTS + 7 STT providers), mobile PWA with push notifications
- **Memory & Recall** — Per-agent memory workspaces, embeddings-powered long-term memory, hybrid vector + full-text search, session persistence with auto-compaction, cross-session recall, Cursor-compatible project context, context-file safety scanning
- **Safer Agent Editing** — Automatic checkpoints before built-in skill and memory mutations, restore tooling, session branching
- **Extensibility** — MCP servers (stdio + HTTP/SSE), skill system, 15 lifecycle hook events with circuit breaker, destructive command guard
- **Security** — Encryption-at-rest vault (XChaCha20-Poly1305 + Argon2id), password + passkey + API key auth, sandbox isolation, SSRF/CSWSH protection
- **Operations** — Cron scheduling, OpenTelemetry tracing, Prometheus metrics, cloud deploy (Fly.io, DigitalOcean), Tailscale integration, managed SSH deploy keys, host-pinned remote targets, live tool inventory in Settings, and CLI/web remote-exec doctor flows

## How It Works

Moltis is a **local-first persistent agent server** — a single Rust binary that
sits between you and multiple LLM providers, keeps durable session state, and
can meet you across channels without handing your data to a cloud relay.

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

Requires [just](https://github.com/casey/just) (command runner) and Node.js (for Tailwind CSS).

```bash
git clone https://github.com/moltis-org/moltis.git
cd moltis
just build-css                  # Build Tailwind CSS for the web UI
just build-release              # Build in release mode
cargo run --release --bin moltis
```

For a full release build including WASM sandbox tools:

```bash
just build-release-with-wasm    # Builds WASM artifacts + release binary
cargo run --release --bin moltis
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
  -p 1455:1455 \
  -v moltis-config:/home/moltis/.config/moltis \
  -v moltis-data:/home/moltis/.moltis \
  -v /var/run/docker.sock:/var/run/docker.sock \
  ghcr.io/moltis-org/moltis:latest
```

Open `https://localhost:13131` and complete the setup. For unattended Docker
deployments, set `MOLTIS_PASSWORD`, `MOLTIS_PROVIDER`, and `MOLTIS_API_KEY`
before first boot to skip the setup wizard. See [Docker docs](https://docs.moltis.org/docker.html)
for Podman, OrbStack, TLS trust, and persistence details.

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
