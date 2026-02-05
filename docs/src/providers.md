# LLM Providers

Moltis supports 30+ LLM providers through a trait-based architecture. Configure providers through the web UI or directly in configuration files.

## Supported Providers

### Tier 1 (Full Support)

| Provider | Models | Tool Calling | Streaming |
|----------|--------|--------------|-----------|
| **Anthropic** | Claude 4, Claude 3.5, Claude 3 | ✅ | ✅ |
| **OpenAI** | GPT-4o, GPT-4, o1, o3 | ✅ | ✅ |
| **Google** | Gemini 2.0, Gemini 1.5 | ✅ | ✅ |
| **GitHub Copilot** | GPT-4o, Claude | ✅ | ✅ |

### Tier 2 (Good Support)

| Provider | Models | Tool Calling | Streaming |
|----------|--------|--------------|-----------|
| **Mistral** | Mistral Large, Codestral | ✅ | ✅ |
| **Groq** | Llama 3, Mixtral | ✅ | ✅ |
| **Together** | Various open models | ✅ | ✅ |
| **Fireworks** | Various open models | ✅ | ✅ |
| **DeepSeek** | DeepSeek V3, Coder | ✅ | ✅ |

### Tier 3 (Basic Support)

| Provider | Notes |
|----------|-------|
| **OpenRouter** | Aggregator for 100+ models |
| **Ollama** | Local models |
| **Venice** | Privacy-focused |
| **Cerebras** | Fast inference |
| **SambaNova** | Enterprise |
| **Cohere** | Command models |
| **AI21** | Jamba models |

## Configuration

### Via Web UI (Recommended)

1. Open Moltis in your browser
2. Go to **Settings** → **Providers**
3. Click on a provider card
4. Enter your API key
5. Select your preferred model

### Via Configuration Files

Provider credentials are stored in `~/.config/moltis/provider_keys.json`:

```json
{
  "anthropic": {
    "apiKey": "sk-ant-...",
    "model": "claude-sonnet-4-20250514"
  },
  "openai": {
    "apiKey": "sk-...",
    "model": "gpt-4o"
  }
}
```

Enable providers in `moltis.toml`:

```toml
[providers]
default = "anthropic"

[providers.anthropic]
enabled = true
models = [
    "claude-sonnet-4-20250514",
    "claude-opus-4-20250514",
]

[providers.openai]
enabled = true
```

## Provider-Specific Setup

### Anthropic

1. Get an API key from [console.anthropic.com](https://console.anthropic.com)
2. Enter it in Settings → Providers → Anthropic

```admonish tip
Claude Sonnet 4 offers the best balance of capability and cost for most coding tasks.
```

### OpenAI

1. Get an API key from [platform.openai.com](https://platform.openai.com)
2. Enter it in Settings → Providers → OpenAI

### GitHub Copilot

GitHub Copilot uses OAuth authentication:

1. Click **Connect** in Settings → Providers → GitHub Copilot
2. Complete the GitHub OAuth flow
3. Authorize Moltis to access Copilot

```admonish info
Requires an active GitHub Copilot subscription.
```

### Google (Gemini)

1. Get an API key from [aistudio.google.com](https://aistudio.google.com)
2. Enter it in Settings → Providers → Google

### Ollama (Local Models)

Run models locally with [Ollama](https://ollama.ai):

1. Install Ollama: `curl -fsSL https://ollama.ai/install.sh | sh`
2. Pull a model: `ollama pull llama3.2`
3. Configure in Moltis:

```json
{
  "ollama": {
    "baseUrl": "http://localhost:11434",
    "model": "llama3.2"
  }
}
```

### OpenRouter

Access 100+ models through one API:

1. Get an API key from [openrouter.ai](https://openrouter.ai)
2. Enter it in Settings → Providers → OpenRouter
3. Specify the model ID you want to use

```json
{
  "openrouter": {
    "apiKey": "sk-or-...",
    "model": "anthropic/claude-3.5-sonnet"
  }
}
```

## Custom Base URLs

For providers with custom endpoints (enterprise, proxies):

```json
{
  "openai": {
    "apiKey": "sk-...",
    "baseUrl": "https://your-proxy.example.com/v1",
    "model": "gpt-4o"
  }
}
```

## Switching Providers

### Per-Session

In the chat interface, use the model selector dropdown to switch providers/models for the current session.

### Per-Message

Use the `/model` command to switch models mid-conversation:

```
/model claude-opus-4-20250514
```

### Default Provider

Set the default in `moltis.toml`:

```toml
[providers]
default = "anthropic"

[agent]
model = "claude-sonnet-4-20250514"
```

## Model Capabilities

Different models have different strengths:

| Use Case | Recommended Model |
|----------|-------------------|
| General coding | Claude Sonnet 4, GPT-4o |
| Complex reasoning | Claude Opus 4, o1 |
| Fast responses | Claude Haiku, GPT-4o-mini |
| Long context | Claude (200k), Gemini (1M+) |
| Local/private | Llama 3 via Ollama |

## Troubleshooting

### "Model not available"

The model may not be enabled for your account or region. Check:
- Your API key has access to the model
- The model ID is spelled correctly
- Your account has sufficient credits

### "Rate limited"

You've exceeded the provider's rate limits. Solutions:
- Wait and retry
- Use a different provider
- Upgrade your API plan

### "Invalid API key"

- Verify the key is correct (no extra spaces)
- Check the key hasn't expired
- Ensure the key has the required permissions
