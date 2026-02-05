# Memory System

Moltis provides a powerful memory system that enables the agent to recall past conversations, notes, and context across sessions. This document explains the available backends, features, and configuration options.

## Backends

Moltis supports two memory backends:

| Feature | Built-in | QMD |
|---------|----------|-----|
| **Search Type** | Hybrid (vector + FTS5 keyword) | Hybrid (BM25 + vector + LLM reranking) |
| **Local Embeddings** | GGUF models via llama-cpp-2 | GGUF models |
| **Remote Embeddings** | OpenAI, Ollama, custom endpoints | Built-in |
| **Embedding Cache** | SQLite with LRU eviction | Built-in |
| **Batch API** | OpenAI batch (50% cost saving) | No |
| **Circuit Breaker** | Fallback chain with auto-recovery | No |
| **LLM Reranking** | Optional (configurable) | Built-in with `query` command |
| **File Watching** | Real-time sync via notify | Built-in |
| **External Dependency** | None (pure Rust) | Requires QMD binary (Node.js/Bun) |
| **Offline Support** | Yes (with local embeddings) | Yes |

### Built-in Backend

The default backend uses SQLite for storage with FTS5 for keyword search and optional vector embeddings for semantic search. Key advantages:

- **Zero external dependencies**: Everything is embedded in the moltis binary
- **Fallback chain**: Automatically switches between embedding providers if one fails
- **Batch embedding**: Reduces OpenAI API costs by 50% for large sync operations
- **Embedding cache**: Avoids re-embedding unchanged content

### QMD Backend

QMD is an optional external sidecar that provides enhanced search capabilities:

- **BM25 keyword search**: Fast, instant results (similar to Elasticsearch)
- **Vector search**: Semantic similarity using local GGUF models
- **Hybrid search with LLM reranking**: Combines both methods with an LLM pass for optimal relevance

To use QMD:
1. Install QMD separately from [github.com/qmd/qmd](https://github.com/qmd/qmd)
2. Enable it in Settings > Memory > Backend

## Features

### Citations

Citations append source file and line number information to search results:

```
Some important content from your notes.

Source: memory/notes.md#42
```

**Configuration options:**
- `auto` (default): Include citations when results come from multiple files
- `on`: Always include citations
- `off`: Never include citations

### Session Export

When enabled, session transcripts are automatically exported to the memory system for cross-run recall. This allows the agent to remember past conversations even after restarts.

Exported sessions are:
- Stored in `memory/sessions/` as markdown files
- Sanitized to remove sensitive tool results and system messages
- Automatically cleaned up based on age/count limits

### LLM Reranking

LLM reranking uses the configured language model to re-score and reorder search results based on semantic relevance to the query. This provides better results than keyword or vector matching alone, at the cost of additional latency.

**How it works:**
1. Initial search returns candidate results
2. LLM evaluates each result's relevance (0.0-1.0 score)
3. Results are reordered by combined score (70% LLM, 30% original)

## Configuration

Memory settings can be configured in `moltis.toml`:

```toml
[memory]
# Backend: "builtin" (default) or "qmd"
backend = "builtin"

# Embedding provider: "local", "ollama", "openai", "custom", or auto-detect
provider = "local"

# Citation mode: "on", "off", or "auto"
citations = "auto"

# Enable LLM reranking for hybrid search
llm_reranking = false

# Export sessions to memory for cross-run recall
session_export = true

# QMD-specific settings (only used when backend = "qmd")
[memory.qmd]
command = "qmd"
max_results = 10
timeout_ms = 30000
```

Or via the web UI: **Settings > Memory**

## Embedding Providers

The built-in backend supports multiple embedding providers:

| Provider | Model | Dimensions | Notes |
|----------|-------|------------|-------|
| Local (GGUF) | EmbeddingGemma-300M | 768 | Offline, ~300MB download |
| Ollama | nomic-embed-text | 768 | Requires Ollama running |
| OpenAI | text-embedding-3-small | 1536 | Requires API key |
| Custom | Configurable | Varies | OpenAI-compatible endpoint |

The system auto-detects available providers and creates a fallback chain:
1. Try configured provider first
2. Fall back to other available providers if it fails
3. Use keyword-only search if no embedding provider is available

## Memory Directories

By default, moltis indexes markdown files from:

- `~/.moltis/MEMORY.md` - Main long-term memory file
- `~/.moltis/memory/*.md` - Additional memory files
- `~/.moltis/memory/sessions/*.md` - Exported session transcripts

## Tools

The memory system exposes two agent tools:

### memory_search

Search memory with a natural language query.

```json
{
  "query": "what did we discuss about the API design?",
  "limit": 5
}
```

### memory_get

Retrieve a specific chunk by ID.

```json
{
  "chunk_id": "memory/notes.md:42"
}
```

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                     Memory Manager                          │
├─────────────────────────────────────────────────────────────┤
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────────────┐  │
│  │   Chunker   │  │   Search    │  │  Session Export     │  │
│  │ (markdown)  │  │  (hybrid)   │  │  (transcripts)      │  │
│  └─────────────┘  └─────────────┘  └─────────────────────┘  │
├─────────────────────────────────────────────────────────────┤
│                    Storage Backend                          │
│  ┌────────────────────────┐  ┌────────────────────────┐    │
│  │   Built-in (SQLite)    │  │   QMD (sidecar)        │    │
│  │  - FTS5 keyword        │  │  - BM25 keyword        │    │
│  │  - Vector similarity   │  │  - Vector similarity   │    │
│  │  - Embedding cache     │  │  - LLM reranking       │    │
│  └────────────────────────┘  └────────────────────────┘    │
├─────────────────────────────────────────────────────────────┤
│                  Embedding Providers                        │
│  ┌─────────┐  ┌─────────┐  ┌─────────┐  ┌───────────────┐  │
│  │  Local  │  │ Ollama  │  │ OpenAI  │  │ Batch/Fallback│  │
│  │  (GGUF) │  │         │  │         │  │               │  │
│  └─────────┘  └─────────┘  └─────────┘  └───────────────┘  │
└─────────────────────────────────────────────────────────────┘
```

## Troubleshooting

### Memory not working

1. Check status in Settings > Memory
2. Ensure at least one embedding provider is available:
   - Local: Requires `local-embeddings` feature enabled at build
   - Ollama: Must be running at `localhost:11434`
   - OpenAI: Requires `OPENAI_API_KEY` environment variable

### Search returns no results

1. Check that memory files exist in the expected directories
2. Trigger a manual sync by restarting moltis
3. Check logs for sync errors

### QMD not available

1. Verify QMD is installed: `qmd --version`
2. Check that the path is correct in settings
3. Ensure QMD has indexed your collections: `qmd stats`
