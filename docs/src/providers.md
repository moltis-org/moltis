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
default = "openai-codex"

[providers.openai-codex]
enabled = true

[providers.github-copilot]
enabled = true

[providers.local]
enabled = true
model = "qwen2.5-coder-7b-q4_k_m"
```

## Provider Setup

### OpenAI Codex

OpenAI Codex uses OAuth token import and OAuth-based access.

1. Go to **Settings** -> **Providers** -> **OpenAI Codex**.
2. Click **Connect** and complete the auth flow.
3. Choose a Codex model.

### GitHub Copilot

GitHub Copilot uses OAuth authentication.

1. Go to **Settings** -> **Providers** -> **GitHub Copilot**.
2. Click **Connect**.
3. Complete the GitHub OAuth flow.

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
- **Global default**: Set `[providers].default` and `[agent].model` in `moltis.toml`.

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
