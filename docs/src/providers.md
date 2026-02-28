# LLM Providers

Moltis supports multiple LLM providers through a trait-based architecture.
Configure providers through the web UI or directly in configuration files.

## Currently Available Providers*

| Provider | Auth | Notes |
|----------|------|-------|
| **OpenAI Codex** | OAuth | Codex-focused cloud models |
| **GitHub Copilot** | OAuth | Requires active Copilot subscription |
| **Local LLM** | Local runtime | Runs models on your machine |

\*More providers are coming soon.

## Configuration

### Via Web UI (Recommended)

1. Open Moltis in your browser.
2. Go to **Settings** -> **Providers**.
3. Choose a provider card.
4. Complete OAuth or enter your API key.
5. Select your preferred model.

### Via Configuration Files

Provider credentials are stored in `~/.config/moltis/provider_keys.json`:

```json
{
  "openai-codex": {
    "model": "gpt-5.2-codex"
  }
}
```

Enable providers in `moltis.toml`:

```toml
[providers]
offered = ["openai-codex", "github-copilot", "local-llm"]

[providers.local-llm]
enabled = true
models = ["qwen2.5-coder-7b-q4_k_m"]

[chat]
priority_models = ["gpt-5.2"]
```

## Provider Setup

### OpenAI Codex

OpenAI Codex uses OAuth token import and OAuth-based access.

1. Go to **Settings** -> **Providers** -> **OpenAI Codex**.
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

1. Go to **Settings** -> **Providers** -> **GitHub Copilot**.
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

### Local LLM

Local LLM runs models directly on your machine.

1. Go to **Settings** -> **Providers** -> **Local LLM**.
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
