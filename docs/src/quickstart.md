# Quickstart

Get Moltis running in under 5 minutes.

## 1. Install

```bash
curl -fsSL https://www.moltis.org/install.sh | sh
```

Or via Homebrew:

```bash
brew install moltis-org/tap/moltis
```

## 2. Start

```bash
moltis
```

You'll see output like:

```
🚀 Moltis gateway starting...
🌐 Open http://localhost:13131 in your browser
```

## 3. Configure a Provider

You need an LLM API key to chat. The easiest options:

### Option A: Anthropic (Recommended)

1. Get an API key from [console.anthropic.com](https://console.anthropic.com)
2. In Moltis, go to **Settings** → **Providers**
3. Click **Anthropic** → Enter your API key → **Save**

### Option B: OpenAI

1. Get an API key from [platform.openai.com](https://platform.openai.com)
2. In Moltis, go to **Settings** → **Providers**
3. Click **OpenAI** → Enter your API key → **Save**

### Option C: Local Model (Free)

1. Install [Ollama](https://ollama.ai): `curl -fsSL https://ollama.ai/install.sh | sh`
2. Pull a model: `ollama pull llama3.2`
3. In Moltis, configure Ollama in **Settings** → **Providers**

## 4. Chat!

Go to the **Chat** tab and start a conversation:

```
You: Write a Python function to check if a number is prime

Agent: Here's a Python function to check if a number is prime:

def is_prime(n):
    if n < 2:
        return False
    for i in range(2, int(n ** 0.5) + 1):
        if n % i == 0:
            return False
    return True
```

## What's Next?

### Enable Tool Use

Moltis can execute code, browse the web, and more. Tools are enabled by default with sandbox protection.

Try:

```
You: Create a hello.py file that prints "Hello, World!" and run it
```

### Connect Telegram

Chat with your agent from anywhere:

1. Create a bot via [@BotFather](https://t.me/BotFather)
2. Copy the bot token
3. In Moltis: **Settings** → **Telegram** → Enter token → **Save**
4. Message your bot!

### Add MCP Servers

Extend capabilities with [MCP servers](mcp.md):

```toml
# In moltis.toml
[[mcp.servers]]
name = "github"
command = "npx"
args = ["-y", "@modelcontextprotocol/server-github"]
env = { GITHUB_TOKEN = "ghp_..." }
```

### Set Up Memory

Enable long-term memory for context across sessions:

```toml
# In moltis.toml
[memory]
enabled = true
```

Add knowledge by placing Markdown files in `~/.moltis/memory/`.

## Useful Commands

| Command | Description |
|---------|-------------|
| `/new` | Start a new session |
| `/model <name>` | Switch models |
| `/clear` | Clear chat history |
| `/help` | Show available commands |

## File Locations

| Path | Contents |
|------|----------|
| `~/.config/moltis/moltis.toml` | Configuration |
| `~/.config/moltis/provider_keys.json` | API keys |
| `~/.moltis/` | Data (sessions, memory, logs) |

## Getting Help

- **Documentation**: [docs.moltis.org](https://docs.moltis.org)
- **GitHub Issues**: [github.com/moltis-org/moltis/issues](https://github.com/moltis-org/moltis/issues)
- **Discussions**: [github.com/moltis-org/moltis/discussions](https://github.com/moltis-org/moltis/discussions)
