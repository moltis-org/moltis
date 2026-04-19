# Choosing a Provider

Not sure which LLM provider to use? This page compares the providers
supported by Moltis so you can pick the best fit for your use case.

## Quick Recommendations

| Goal | Provider | Why |
|------|----------|-----|
| **Best overall quality** | Anthropic | Claude Opus 4.6 and Sonnet 4.6 excel at tool use, long context, and instruction following |
| **Widest model range** | OpenAI | GPT-5.2, o3/o4-mini reasoning models, image generation |
| **Largest context window** | Google Gemini | Up to 1M tokens with Gemini 2.5 Pro / 3.1 Pro |
| **Best value** | DeepSeek | DeepSeek Chat and Reasoner offer strong performance at low cost |
| **Fast inference** | Groq | Hardware-accelerated inference, very low latency |
| **Free / offline** | Ollama | Run open models locally, no API key needed |
| **Rising stars** | MiniMax, Z.AI | MiniMax M2.7 and Z.AI GLM-5 models are gaining traction for quality and price |

## Provider Comparison

| Provider | Top Models | Tool Use | Streaming | Context | Price Tier | Speed | Notes |
|----------|-----------|----------|-----------|---------|------------|-------|-------|
| **Anthropic** | Claude Opus 4.6, Sonnet 4.6, Opus 4.5, Sonnet 4.5 | Full | Yes | 200K | $$ | Fast | Best tool-use reliability |
| **OpenAI** | GPT-5.2, GPT-5 Mini, o3, o4-mini | Full | Yes | 128K–200K | $$ | Fast | Widest ecosystem, reasoning models |
| **Google Gemini** | Gemini 3.1 Pro, 2.5 Pro, 2.5 Flash | Full | Yes | 1M | $ | Fast | Largest context, competitive pricing |
| **DeepSeek** | DeepSeek Chat, DeepSeek Reasoner | Full | Yes | 128K | $ | Medium | Excellent quality-to-price ratio |
| **Groq** | Llama 4 Scout, various | Full | Yes | 128K | $ | Very fast | Speed-optimized hardware inference |
| **xAI** | Grok 4, Grok 3 | Full | Yes | 128K | $$ | Fast | Strong reasoning capabilities |
| **Mistral** | Mistral Large, Codestral | Full | Yes | 128K–256K | $$ | Fast | European provider, multilingual |
| **OpenRouter** | Any (aggregator) | Varies | Yes | Varies | Varies | Varies | Access 100+ models with one key |
| **Cerebras** | Llama 4 Scout | Full | Yes | 128K | $ | Very fast | Wafer-scale inference hardware |
| **MiniMax** | M2.7, M2.5, M2.1 | Full | Yes | 204K | $ | Fast | Strong multilingual, long context |
| **Z.AI (Zhipu)** | GLM-5, GLM-4.7, GLM-4.6 | Full | Yes | 128K | $ | Fast | GLM-5 series, competitive quality |
| **Z.AI Coding** | GLM-5, GLM-4.7 | Full | Yes | 128K | $ | Fast | Optimized for code tasks |
| **Moonshot** | Kimi K2.5 | Full | Yes | 128K | $ | Medium | Long context, Chinese/English |
| **Venice** | Various | Varies | Yes | Varies | $ | Medium | Privacy-focused, uncensored models |
| **Ollama** | Any GGUF model | Varies | Yes | Varies | Free | Varies | Local inference, no API key |
| **Local LLM** | Any GGUF model | Varies | Yes | Varies | Free | Varies | Built-in GGUF runner, no server needed |
| **GitHub Copilot** | GPT-4o, Claude (via Copilot) | Full | Yes | Varies | Subscription | Fast | Uses existing Copilot subscription |
| **OpenAI Codex** | Codex models | Full | Yes | Varies | $$ | Fast | OAuth-based, code-focused |
| **Fireworks** | Kimi K2.5, DeepSeek V3p2, Qwen3 | Full | Yes | Varies | $ | Fast | Fast inference, diverse model catalog |
| **Alibaba Coding** | Qwen3 Coder, GLM-5, MiniMax M2.5 | Full | Yes | Varies | $ | Fast | Alibaba Cloud coding-focused plan |
| **LMStudio** | Any GGUF model | Varies | Yes | Varies | Free | Varies | Local inference via LM Studio server |

### Price Tier Legend

| Symbol | Meaning |
|--------|---------|
| **Free** | No cost (local inference) |
| **$** | Budget-friendly (< $1/M input tokens) |
| **$$** | Standard pricing ($1-15/M input tokens) |
| **$$$** | Premium pricing (> $15/M input tokens) |
| **Subscription** | Flat monthly fee |

## How to Choose

### For personal projects or experimentation

Start with **Google Gemini** (generous free tier, large context) or
**Ollama** (completely free, runs locally). Both are easy to set up and
let you explore without cost pressure.

### For production agent workflows

**Anthropic** and **OpenAI** are the most battle-tested for tool use and
complex multi-step tasks. Anthropic's Claude models tend to follow
instructions more precisely; OpenAI offers a broader model range
including reasoning models (o3, o4-mini).

### For cost-sensitive workloads

**DeepSeek** offers the best quality-to-price ratio for most tasks.
**Groq** and **Cerebras** provide extremely fast inference at low cost,
though model selection is more limited.

### For local / offline use

**Ollama** is the easiest path --- install it, pull a model, and Moltis
auto-detects it. **Local LLM** runs GGUF models directly without a
separate server. Both require sufficient RAM (8GB+ for small models,
16GB+ recommended).

### For access to many models

**OpenRouter** aggregates 100+ models behind a single API key. Useful if
you want to experiment across providers without managing multiple
accounts.

## Setting Up a Provider

See the [LLM Providers](providers.md) page for step-by-step setup
instructions for each provider, including configuration file options and
environment variables.
