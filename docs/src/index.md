# Moltis

```admonish warning title="Alpha software: use with care"
Running an AI assistant on your own machine or server is still new territory. Treat Moltis as alpha software: run it in isolated environments, review enabled tools/providers, keep secrets scoped and rotated, and avoid exposing it publicly without strong authentication and network controls.
```

<div style="text-align: center; margin: 2em 0;">
<strong style="font-size: 1.2em;">A personal AI gateway written in Rust.<br>One binary, no runtime, no npm.</strong>
</div>

Moltis compiles your entire AI gateway — web UI, LLM providers, tools, and all assets — into a single self-contained executable. There's no Node.js to babysit, no `node_modules` to sync, no V8 garbage collector introducing latency spikes.

```bash
# Quick install (macOS / Linux)
curl -fsSL https://www.moltis.org/install.sh | sh
```

## Why Moltis?

| Feature | Moltis | Other Solutions |
|---------|--------|-----------------|
| **Deployment** | Single binary | Node.js + dependencies |
| **Memory Safety** | Rust ownership | Garbage collection |
| **Secret Handling** | Zeroed on drop | "Eventually collected" |
| **Sandbox** | Docker + Apple Container | Docker only |
| **Startup** | Milliseconds | Seconds |

## Key Features

- **30+ LLM Providers** — Anthropic, OpenAI, Google, Mistral, local models, and more
- **Streaming-First** — Responses appear as tokens arrive, not after completion
- **Sandboxed Execution** — Commands run in isolated containers (Docker or Apple Container)
- **MCP Support** — Connect to Model Context Protocol servers for extended capabilities
- **Multi-Channel** — Web UI, Telegram, API access with synchronized responses
- **Long-Term Memory** — Embeddings-powered knowledge base with hybrid search
- **Hook System** — Observe, modify, or block actions at any lifecycle point
- **Compile-Time Safety** — Misconfigurations caught by `cargo check`, not runtime crashes

## Quick Start

```bash
# Install
curl -fsSL https://www.moltis.org/install.sh | sh

# Run
moltis
```

On first launch:
1. Open the URL shown in your browser (e.g., `http://localhost:13131`)
2. Add your LLM API key
3. Start chatting!

```admonish note
Authentication is only required when accessing Moltis from a non-localhost address. On localhost, you can start using it immediately.
```

→ [Full Quickstart Guide](quickstart.md)

## How It Works

```
┌─────────────┐  ┌─────────────┐  ┌─────────────┐
│   Web UI    │  │  Telegram   │  │     API     │
└──────┬──────┘  └──────┬──────┘  └──────┬──────┘
       │                │                │
       └────────────────┴────────────────┘
                        │
                        ▼
        ┌───────────────────────────────┐
        │       Moltis Gateway          │
        │   ┌─────────┐ ┌───────────┐   │
        │   │  Agent  │ │   Tools   │   │
        │   │  Loop   │◄┤  Registry │   │
        │   └────┬────┘ └───────────┘   │
        │        │                      │
        │   ┌────▼────────────────┐     │
        │   │  Provider Registry  │     │
        │   │ Claude · GPT · Gemini │   │
        │   └─────────────────────┘     │
        └───────────────────────────────┘
                        │
                ┌───────▼───────┐
                │    Sandbox    │
                │ Docker/Apple  │
                └───────────────┘
```

## Documentation

### Getting Started
- **[Quickstart](quickstart.md)** — Up and running in 5 minutes
- **[Installation](installation.md)** — All installation methods
- **[Configuration](configuration.md)** — `moltis.toml` reference

### Features
- **[Providers](providers.md)** — Configure LLM providers
- **[MCP Servers](mcp.md)** — Extend with Model Context Protocol
- **[Hooks](hooks.md)** — Lifecycle hooks for customization
- **[Local LLMs](local-llm.md)** — Run models on your machine

### Deployment
- **[Docker](docker.md)** — Container deployment

### Architecture
- **[Streaming](streaming.md)** — How real-time streaming works
- **[Metrics & Tracing](metrics-and-tracing.md)** — Observability

## Security

Moltis applies defense in depth:

- **Authentication** — Password or passkey (WebAuthn) required for non-localhost access
- **SSRF Protection** — Blocks requests to internal networks
- **Secret Handling** — `secrecy::Secret` zeroes memory on drop
- **Sandboxed Execution** — Commands never run on the host
- **Origin Validation** — Prevents Cross-Site WebSocket Hijacking
- **No Unsafe Code** — `unsafe` is denied workspace-wide

## Community

- **GitHub**: [github.com/moltis-org/moltis](https://github.com/moltis-org/moltis)
- **Issues**: [Report bugs](https://github.com/moltis-org/moltis/issues)
- **Discussions**: [Ask questions](https://github.com/moltis-org/moltis/discussions)

## License

MIT — Free for personal and commercial use.
