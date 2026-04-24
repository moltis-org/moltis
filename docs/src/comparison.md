# Comparison

How Moltis compares to the larger open-source personal agent projects: OpenClaw
and Hermes Agent.

> **Disclaimer:** This page is based on source snapshots captured while writing:
> OpenClaw [`90eb5b0`](https://github.com/openclaw/openclaw/commit/90eb5b073fd2d7d8e94b19708e3baceeb8811ca8)
> from 2026-04-01, Hermes Agent
> [`9f22977`](https://github.com/NousResearch/hermes-agent/commit/9f22977fc0d2d6de5ff4d0a1a8e4d4ae3a00ea52)
> from 2026-04-20, and Moltis
> [`5d044c6`](https://github.com/moltis-org/moltis/commit/5d044c62fd9d19264db0fc5705065f633d10a657)
> from 2026-04-22. Projects move quickly, so check each repository for current
> behavior before making a deployment decision.

## At a Glance

| | [OpenClaw](https://github.com/openclaw/openclaw) | [Hermes Agent](https://github.com/NousResearch/hermes-agent) | **Moltis** |
|---|---|---|---|
| Primary stack | TypeScript, with Swift/Kotlin companion apps | Python, with TypeScript TUI/web surfaces | **Rust** |
| Main runtime | Node.js 22.16+/24 + npm/pnpm/bun | Python + uv/pip, optional Node UI pieces | **Single Rust binary** |
| Main shape | Broad gateway, channel, node, app, and plugin ecosystem | CLI/gateway agent with a learning loop and research tooling | **Persistent personal agent server with modular crates** |
| Local checkout size\* | ~1.1M app LoC | ~152K app LoC | **~270K Rust LoC** |
| Crates/modules | npm packages, extensions, apps | Python packages, plugins, tools, TUI | **59 Rust workspace crates** |
| Sandbox/backends | App-level permissions, browser/node tools | Local, Docker, SSH, Daytona, Singularity, Modal | **Docker/Podman + Apple Container + WASM** |
| Auth/access | Pairing and gateway controls | CLI and messaging gateway setup | **Password + Passkey + API keys + Vault** |
| Voice I/O | Voice wake and talk modes | Voice memo transcription | **Built-in STT + TTS providers** |
| MCP | Plugin/integration support | MCP integration | **stdio + HTTP/SSE** |
| Skills | Bundled, managed, and workspace skills | Self-improving skills, Skills Hub support | **Bundled/workspace skills + autonomous improvement + OpenClaw import** |
| Memory/RAG | Plugin-backed memory and context engine | Agent-curated memory, session search, user modeling | **SQLite + FTS + vector memory** |

\* LoC measured with `tokei`, excluding `node_modules`, generated build output,
`dist`, and `target`. Counts are a rough auditability signal, not a quality
metric.

## Architecture Approach

### OpenClaw, ecosystem-first personal assistant

OpenClaw is a full-featured personal assistant platform. The local checkout
shows a TypeScript gateway with macOS, iOS, and Android companion surfaces,
plus a large channel list, node tools, browser/canvas support, plugin
extensions, onboarding, and managed/workspace skills.

### Hermes Agent, learning-loop CLI and gateway

Hermes Agent is Python-first. Its README centers the agent around a terminal
interface, a messaging gateway, a closed learning loop, self-improving skills,
agent-curated memory, session search, user modeling, cron scheduling, and
cloud/serverless execution backends. Moltis has autonomous skill improvement
too, so Hermes' sharper distinction is its CLI/research loop and broad terminal
backend set. It also carries research-oriented pieces such as trajectory
generation and RL environments.

### Moltis, Rust-native persistent agent server

Moltis prioritizes a smaller trusted runtime, durable agent workflows, and
defense in depth. The Rust workspace is currently ~270K lines across 59 crates.
The agent runner and model interface are ~7.5K lines, with provider
implementations in ~19K more.

Key differences:

- **Single Rust binary** instead of a Node.js or Python application runtime
- **Built-in web UI** with streaming chat, settings, sessions, projects, and
  admin surfaces
- **Docker/Podman, Apple Container, and WASM sandboxing**
- **Password, WebAuthn passkeys, scoped API keys, and vault-backed secret
  storage**
- **Cross-session recall** without dumping raw history into every prompt
- **Autonomous skill self-improvement** with `enable_self_improvement` on by default
- **Automatic checkpoints** before built-in skill and memory mutations
- **15 lifecycle hook events** with circuit breaker and dry-run mode
- **Read-only OpenClaw import** for identity, providers, skills, memory,
  sessions, channels, and MCP config

Moltis intentionally has a small unsafe surface, not a zero-unsafe entire
workspace. Unsafe code is isolated to Swift FFI, local model wrappers, and
precompiled WASM/runtime boundaries. The core agent and gateway paths stay in
safe Rust.

## Security Model

| Aspect | OpenClaw | Hermes Agent | **Moltis** |
|--------|----------|--------------|------------|
| Code sandbox | App-level permissions and tool controls | Local/Docker/SSH/cloud terminal backends | Docker/Podman + Apple Container + WASM |
| Secret handling | Environment/config/plugin paths | Config and provider credentials | `secrecy::Secret`, encrypted vault, redaction |
| Auth/access | Pairing and gateway controls | CLI plus messaging gateway setup | Password + Passkey + scoped API keys |
| SSRF protection | Tool/plugin dependent | Tool/backend dependent | DNS-resolved, blocks loopback/private/link-local/CGNAT |
| WebSocket origin | Gateway dependent | Gateway dependent | Cross-origin rejection |
| Unsafe/native boundary | N/A for TS core, native apps exist | N/A for Python core, native deps possible | Isolated FFI/runtime unsafe islands |
| Hook gating | Plugin and runtime hooks | Hooks/plugins | `BeforeToolCall` inspect/modify/block |
| Rate limiting | Gateway dependent | Gateway dependent | Per-IP throttle, strict login limits |

## Local Checkout Snapshot

| Metric | OpenClaw | Hermes Agent | **Moltis** |
|--------|----------|--------------|------------|
| Main implementation LoC\* | ~1.0M TypeScript, ~89K Swift, ~25K Kotlin | ~144K Python, ~8K TypeScript | **~270K Rust** |
| Main install path | `npm install -g openclaw` | `curl .../install.sh \| bash`, then `hermes` | **Install script, Homebrew, Docker, or Cargo** |
| Runtime dependency | Node.js | Python environment | **Bundled binary** |
| Workspace/package count | npm packages, extensions, apps | Python package, plugins, tools, UI packages | **59 Rust crates** |
| Test surface signal | Large TS/app test tree | Python and TUI tests | **470+ Rust files containing tests** |

\* These counts are intentionally limited to app/source directories and exclude
dependency folders and build output. They are useful for spotting scale, not for
ranking projects.

## Links

- [OpenClaw](https://github.com/openclaw/openclaw) and [OpenClaw docs](https://docs.openclaw.ai)
- [Hermes Agent](https://github.com/NousResearch/hermes-agent) and [Hermes docs](https://hermes-agent.nousresearch.com/docs/)
- [Moltis](https://github.com/moltis-org/moltis) and [Moltis docs](https://docs.moltis.org)
