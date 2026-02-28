# LLM Providers

Moltis supports multiple LLM providers through a trait-based architecture.
Configure providers through the web UI or directly in configuration files.

## Available Providers

### API Key Providers

| Provider | Config Name | Env Variable | Features |
|----------|-------------|--------------|----------|
| **Anthropic** | `anthropic` | `ANTHROPIC_API_KEY` | Streaming, tools, vision |
| **OpenAI** | `openai` | `OPENAI_API_KEY` | Streaming, tools, vision, model discovery |
| **Google Gemini** | `gemini` | `GEMINI_API_KEY` | Streaming, tools, vision, model discovery |
| **DeepSeek** | `deepseek` | `DEEPSEEK_API_KEY` | Streaming, tools, model discovery |
| **Mistral** | `mistral` | `MISTRAL_API_KEY` | Streaming, tools, model discovery |
| **Groq** | `groq` | `GROQ_API_KEY` | Streaming |
| **xAI (Grok)** | `xai` | `XAI_API_KEY` | Streaming |
| **OpenRouter** | `openrouter` | `OPENROUTER_API_KEY` | Streaming, tools, model discovery |
| **Cerebras** | `cerebras` | `CEREBRAS_API_KEY` | Streaming, tools, model discovery |
| **MiniMax** | `minimax` | `MINIMAX_API_KEY` | Streaming, tools |
| **Moonshot (Kimi)** | `moonshot` | `MOONSHOT_API_KEY` | Streaming, tools, model discovery |
| **Venice** | `venice` | `VENICE_API_KEY` | Streaming, tools, model discovery |
| **Z.AI (Zhipu)** | `zai` | `Z_API_KEY` | Streaming, tools, model discovery |

### OAuth Providers

| Provider | Config Name | Notes |
|----------|-------------|-------|
| **OpenAI Codex** | `openai-codex` | OAuth flow via web UI |
| **GitHub Copilot** | `github-copilot` | Requires active Copilot subscription |

### Local

| Provider | Config Name | Notes |
|----------|-------------|-------|
| **Ollama** | `ollama` | Local or remote Ollama instance |
| **Local LLM** | `local-llm` | Runs GGUF models directly on your machine |

### Custom OpenAI-Compatible

Any OpenAI-compatible endpoint can be added with a `custom-` prefix:

```toml
[providers.custom-myservice]
enabled = true
api_key = "..."
base_url = "https://my-service.example.com/v1"
models = ["my-model"]
```

## Configuration

### Via Web UI (Recommended)

1. Open Moltis in your browser.
2. Go to **Settings** → **Providers**.
3. Choose a provider card.
4. Complete OAuth or enter your API key.
5. Select your preferred model.

### Via Configuration Files

Configure providers in `moltis.toml`:

```toml
[providers]
offered = ["anthropic", "openai", "gemini"]

[providers.anthropic]
enabled = true

[providers.openai]
enabled = true
models = ["gpt-5.3", "gpt-5.2"]
stream_transport = "sse"              # "sse", "websocket", or "auto"

[providers.gemini]
enabled = true
models = ["gemini-2.5-flash-preview-05-20", "gemini-2.0-flash"]
# api_key = "..."                     # Or set GEMINI_API_KEY / GOOGLE_API_KEY env var
# fetch_models = true                 # Discover models from the API
# base_url = "https://generativelanguage.googleapis.com/v1beta/openai"

[chat]
priority_models = ["gpt-5.2"]
```

### Provider Entry Options

Each provider supports these options:

| Option | Default | Description |
|--------|---------|-------------|
| `enabled` | `true` | Enable or disable the provider |
| `api_key` | — | API key (overrides env var) |
| `base_url` | — | Override API endpoint URL |
| `models` | `[]` | Preferred models shown first in the picker |
| `fetch_models` | `true` | Discover available models from the API |
| `stream_transport` | `"sse"` | `"sse"`, `"websocket"`, or `"auto"` |
| `alias` | — | Custom label for metrics |
| `tool_mode` | `"auto"` | `"auto"`, `"native"`, `"text"`, or `"off"` |

## Provider Setup

### Google Gemini

Google Gemini uses an API key from [Google AI Studio](https://aistudio.google.com/).

1. Get an API key from Google AI Studio.
2. Set `GEMINI_API_KEY` in your environment (or use `GOOGLE_API_KEY`).
3. Gemini models appear automatically in the model picker.

```toml
[providers.gemini]
enabled = true
models = ["gemini-2.5-flash-preview-05-20", "gemini-2.0-flash"]
```

Gemini supports native tool calling, vision/multimodal inputs, streaming, and automatic model discovery.

### Anthropic

1. Get an API key from [console.anthropic.com](https://console.anthropic.com/).
2. Set `ANTHROPIC_API_KEY` in your environment.

### OpenAI

1. Get an API key from [platform.openai.com](https://platform.openai.com/).
2. Set `OPENAI_API_KEY` in your environment.

### OpenAI Codex

OpenAI Codex uses OAuth-based access.

1. Go to **Settings** → **Providers** → **OpenAI Codex**.
2. Click **Connect** and complete the auth flow.
3. Choose a Codex model.

```admonish note title="Docker and cloud deployments"
The OAuth flow redirects your browser to `localhost:1455`. In Docker, make sure
port 1455 is published (`-p 1455:1455`). On cloud platforms where `localhost`
cannot reach the server, authenticate via the CLI instead:

~~~bash
# Docker
docker exec -it moltis moltis auth login --provider openai-codex

# Fly.io
fly ssh console -C "moltis auth login --provider openai-codex"
~~~

The CLI opens a browser on your machine and handles the callback locally.
Tokens are saved to the config volume and picked up by the gateway automatically.
```

### GitHub Copilot

GitHub Copilot uses OAuth authentication.

1. Go to **Settings** → **Providers** → **GitHub Copilot**.
2. Click **Connect**.
3. Complete the GitHub OAuth flow.

```admonish note title="Docker and cloud deployments"
GitHub Copilot uses device-flow authentication (a code you enter on github.com),
so it works from the web UI without extra port configuration. If you prefer the
CLI:

~~~bash
docker exec -it moltis moltis auth login --provider github-copilot
~~~
```

```admonish info
Requires an active GitHub Copilot subscription.
```

### Ollama

Ollama auto-detects when running at `http://127.0.0.1:11434`. No API key needed.

```toml
[providers.ollama]
enabled = true
# base_url = "http://127.0.0.1:11434/v1"  # Override for remote Ollama
```

### Local LLM

Local LLM runs GGUF models directly on your machine.

1. Go to **Settings** → **Providers** → **Local LLM**.
2. Choose a model from the local registry or download one.
3. Save and select it as your active model.

## Switching Models

- **Per session**: Use the model selector in the chat UI.
- **Per message**: Use `/model <name>` in chat.
- **Global defaults**: Use `[providers].offered`, provider `models = [...]`, and
  `[chat].priority_models` in `moltis.toml`.

## Troubleshooting

### "Model not available"

- Check provider auth is still valid.
- Check model ID spelling.
- Check account access for that model.

### "Rate limited"

- Retry after a short delay.
- Switch provider/model.
- Upgrade provider quota if needed.

### "Invalid API key"

- Verify the key has no extra spaces.
- Verify it is active and has required permissions.
