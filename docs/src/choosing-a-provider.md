# Choosing a Provider

Not sure which LLM provider to use? This page compares the providers
supported by Moltis so you can pick the best fit for your use case.

## Quick Recommendations

| Goal | Provider | Why |
|------|----------|-----|
| **Best overall quality** | Anthropic | Claude Sonnet 4 and Opus 4 excel at tool use, long context, and instruction following |
| **Widest model range** | OpenAI | GPT-4.1, o3/o4-mini reasoning models, image generation |
| **Largest context window** | Google Gemini | Up to 1M tokens with Gemini 2.5 Pro |
| **Best value** | DeepSeek | DeepSeek V3 and R1 offer strong performance at low cost |
| **Fast inference** | Groq | Hardware-accelerated inference, very low latency |
| **Free / offline** | Ollama | Run open models locally, no API key needed |
| **Rising stars** | MiniMax, Z.AI | MiniMax and GLM-4 models are gaining traction for quality and price |

## Provider Comparison

| Provider | Top Models | Tool Use | Streaming | Context | Price Tier | Speed | Notes |
|----------|-----------|----------|-----------|---------|------------|-------|-------|
| **Anthropic** | Claude Sonnet 4, Opus 4 | Full | Yes | 200K | $$ | Fast | Best tool-use reliability |
| **OpenAI** | GPT-4.1, o3, o4-mini | Full | Yes | 128K-1M | $$ | Fast | Widest ecosystem, reasoning models |
| **Google Gemini** | Gemini 2.5 Pro, 2.5 Flash | Full | Yes | 1M | $ | Fast | Largest context, competitive pricing |
| **DeepSeek** | V3, R1 | Full | Yes | 128K | $ | Medium | Excellent quality-to-price ratio |
| **Groq** | Llama 3, Mixtral, Gemma | Partial | Yes | 128K | $ | Very fast | Speed-optimized hardware inference |
| **xAI** | Grok 3, Grok 3 Mini | Yes | Yes | 128K | $$ | Fast | Strong reasoning capabilities |
| **Mistral** | Mistral Large, Medium | Full | Yes | 128K | $$ | Fast | European provider, multilingual |
| **OpenRouter** | Any (aggregator) | Varies | Yes | Varies | Varies | Varies | Access 100+ models with one key |
| **Cerebras** | Llama 3 | Partial | Yes | 128K | $ | Very fast | Wafer-scale inference hardware |
| **MiniMax** | MiniMax-Text-01, abab7 | Full | Yes | 1M | $ | Fast | Strong multilingual, long context |
| **Z.AI (Zhipu)** | GLM-4, GLM-4 Air | Full | Yes | 128K | $ | Fast | GLM-4 series, competitive quality |
| **Z.AI Coding** | CodeGeeX, GLM-4 Code | Full | Yes | 128K | $ | Fast | Optimized for code tasks |
| **Moonshot** | Kimi | Full | Yes | 200K | $ | Medium | Long context, Chinese/English |
| **Venice** | Various | Varies | Yes | Varies | $ | Medium | Privacy-focused, uncensored models |
| **Ollama** | Any GGUF model | Varies | Yes | Varies | Free | Varies | Local inference, no API key |
| **Local LLM** | Any GGUF model | Varies | Yes | Varies | Free | Varies | Built-in GGUF runner, no server needed |
| **GitHub Copilot** | GPT-4o, Claude (via Copilot) | Full | Yes | Varies | Subscription | Fast | Uses existing Copilot subscription |
| **OpenAI Codex** | Codex models | Full | Yes | Varies | $$ | Fast | OAuth-based, code-focused |

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
