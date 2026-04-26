# moltis-mini

A stripped-down build of [Moltis](https://github.com/moltis-org/moltis) for personal use.

Removes unused integrations (Matrix, Signal, Slack, WhatsApp, Nostr, CalDAV, Tailscale, etc.) from the workspace and default feature set, producing a smaller binary and faster builds.

**You probably want the full project:** [moltis-org/moltis](https://github.com/moltis-org/moltis)

If you think you want my version anyway, go right ahead.

## What's Removed

| Crate | Reason |
|---|---|
| `apps/courier` | Companion app, unused |
| `moltis-auto-reply` | Auto-reply, unused |
| `moltis-benchmarks` | Dev-only |
| `moltis-caldav` | Calendar sync, unused |
| `moltis-graphql` | GraphQL endpoint, unused |
| `moltis-matrix` | Matrix channel, unused |
| `moltis-network-filter` | Network proxy, unused |
| `moltis-nostr` | Nostr channel, unused |
| `moltis-openclaw-import` | Migration tool, unused |
| `moltis-qmd` | QMD tools, unused |
| `moltis-signal` | Signal channel, unused |
| `moltis-slack` | Slack channel, unused |
| `moltis-swift-bridge` | iOS/macOS bridge, unused on Linux |
| `moltis-tailscale` | Tailscale, unused |
| `moltis-whatsapp` | WhatsApp channel, unused |

## What's Kept

Discord, Telegram, MS Teams (always-compiled), Home Assistant, web UI, TLS, vault, WASM tools, code-indexing, memory, skills, cron, MCP, metrics, file-watcher.

## Default Features

```
agent, bundled-skills, code-splitter, file-watcher, fs-tools,
jemalloc, tls, vault, wasm, web-ui, home-assistant, metrics
```

## Staying Current

```bash
git fetch upstream main
git rebase upstream/main
git push origin main --force-with-lease
```

CI auto-builds a new container image on every push to `main`.

## Container Image

```bash
docker pull ghcr.io/cstewart-hc/moltis-mini:latest
```

## Building Locally

```bash
rustup toolchain install nightly-2025-11-30
rustup target add wasm32-wasip2 --toolchain nightly-2025-11-30
cargo build --release
```

## License

MIT — same as [upstream](https://github.com/moltis-org/moltis).
