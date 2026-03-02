# Memory: Moltis vs OpenClaw

This page provides a detailed comparison of the memory systems in Moltis and
[OpenClaw](https://github.com/openclaw/openclaw). Both projects share the
same core architecture for long-term memory, but differ in implementation
details, tool surface, and configuration.

## Overview

Both systems follow the same fundamental approach: **plain Markdown files are
the source of truth** for agent memory, indexed for semantic search using
hybrid vector + keyword retrieval. The agent reads from memory via search
tools and writes to memory via file-writing tools (either dedicated or
general-purpose).

## Feature Comparison

### Storage and Indexing

| Feature | Moltis | OpenClaw |
|---------|--------|----------|
| **Storage format** | Markdown files on disk | Markdown files on disk |
| **Index storage** | SQLite (per data dir) | SQLite (per agent) |
| **Default backend** | Built-in (SQLite + FTS5 + vector) | Built-in (SQLite + BM25 + vector) |
| **Alternative backend** | QMD (sidecar, BM25 + vector + reranking) | QMD (sidecar, BM25 + vector + reranking) |
| **Keyword search** | FTS5 | BM25 |
| **Vector search** | Cosine similarity | Cosine similarity |
| **Hybrid scoring** | Configurable vector/keyword weights | Configurable vector/text weights |
| **Chunking** | Markdown-aware (~400 tokens, configurable) | Markdown-aware (~400 tokens, 80-token overlap) |
| **Embedding cache** | SQLite with LRU eviction | SQLite, chunk-level |
| **File watching** | Real-time sync via `notify` | File watcher with 1.5s debounce |
| **Auto-reindex on provider change** | No (manual) | Yes (fingerprint-based) |

### Embedding Providers

| Provider | Moltis | OpenClaw |
|----------|--------|----------|
| **Local GGUF** | EmbeddingGemma-300M via llama-cpp-2 | Auto-download GGUF (~0.6 GB) |
| **Ollama** | nomic-embed-text | Not listed |
| **OpenAI** | text-embedding-3-small | Via API key |
| **Gemini** | Not available | Via API key |
| **Voyage** | Not available | Via API key |
| **Custom endpoint** | OpenAI-compatible | Not listed |
| **Batch embedding** | OpenAI batch API (50% cost saving) | OpenAI, Gemini, Voyage batch |
| **Fallback chain** | Auto-detect + circuit breaker | Auto-select in priority order |
| **Offline support** | Yes (local embeddings) | Yes (local embeddings) |

### Memory Files

| Aspect | Moltis | OpenClaw |
|--------|--------|----------|
| **Data directory** | `~/.moltis/` (configurable) | `~/.openclaw/workspace/` |
| **Long-term memory** | `MEMORY.md` | `MEMORY.md` |
| **Daily logs** | `memory/YYYY-MM-DD.md` | `memory/YYYY-MM-DD.md` |
| **Session transcripts** | `memory/sessions/*.md` | Session JSONL files (separate) |
| **Extra paths** | Via `memory_dirs` config | Via `memorySearch.extraPaths` |
| **MEMORY.md loading** | Always available in system prompt | Only in private sessions (not group chats) |

### Agent Tools

This is where the two systems differ most significantly in approach.

| Tool | Moltis | OpenClaw |
|------|--------|----------|
| **memory_search** | Dedicated tool, hybrid search | Dedicated tool, hybrid search |
| **memory_get** | Dedicated tool, by chunk ID | Dedicated tool, by path + optional line range |
| **memory_save** | Dedicated tool with path validation | No dedicated tool |
| **General file writing** | `exec` tool (shell commands) | Generic `write_file` tool |
| **Silent memory turn** | Pre-compaction flush via `MemoryWriter` | Pre-compaction flush via `write_file` |

#### How "Remember X" Works

When a user says "remember that I prefer dark mode", here is how each system
handles it:

**Moltis:**
The agent calls the `memory_save` tool directly:
```json
{
  "content": "User prefers dark mode.",
  "file": "MEMORY.md",
  "append": true
}
```
The `memory_save` tool validates the path, writes the file, and re-indexes it
so the content is immediately searchable. The agent does not need shell access
or a generic file-writing tool.

**OpenClaw:**
The agent calls the generic `write_file` tool (which is also used for writing
code, configs, and any other file):
```json
{
  "path": "MEMORY.md",
  "content": "User prefers dark mode.",
  "append": true
}
```
The system prompt instructs the agent which paths are for memory. The tool
itself has no special memory awareness -- it is a general-purpose file writer.
The memory indexer's file watcher detects the change and re-indexes
asynchronously (1.5s debounce).

**Key difference:** Moltis uses a purpose-built `memory_save` tool with
built-in path validation (only `MEMORY.md` and `memory/*.md` are writable)
and immediate re-indexing. OpenClaw uses a general-purpose `write_file` tool
that can write anywhere, relying on the system prompt to guide the agent to
memory paths and the file watcher to re-index.

### Session Memory and Compaction

| Feature | Moltis | OpenClaw |
|---------|--------|----------|
| **Session storage** | SQLite database | JSONL files (append-only, tree structure) |
| **Auto-compaction** | Yes, near context window limit | Yes, near context window limit |
| **Manual compaction** | Not yet | `/compact` command with optional instructions |
| **Pre-compaction memory flush** | Silent turn via `MemoryWriter` trait | Silent turn via `write_file` tool |
| **Flush visibility** | Completely hidden from user | Hidden via `NO_REPLY` convention |
| **Session export to memory** | Markdown files in `memory/sessions/` | Optional (`sessionMemory` experimental flag) |
| **Session pruning** | Not yet | Cache-TTL based, trims old tool results |
| **Session transcript indexing** | Via session export | Experimental, async delta-based |

### Pre-Compaction Memory Flush: Detailed Comparison

Both systems run a hidden LLM turn before compaction to persist important
context. The implementation differs:

**Moltis:**
- The gateway detects that compaction is needed
- A `run_silent_memory_turn()` call creates a temporary agent loop with a
  `write_file` tool backed by `MemoryWriter`
- The `MemoryWriter` trait is implemented by `MemoryManager`, which validates
  paths and re-indexes after writing
- The LLM's response text is discarded
- Written file paths are returned to the caller for logging

**OpenClaw:**
- A soft threshold (default 4000 tokens below compaction trigger) activates
  the flush
- The flush executes as a regular turn with `NO_REPLY` prefix to suppress
  user-facing output
- The agent writes memory files via the same `write_file` tool used during
  normal conversation
- Flush state is tracked in `sessions.json` (`memoryFlushAt`,
  `memoryFlushCompactionCount`) to run once per compaction cycle
- Skipped for read-only workspaces

### Write Path Security

| Aspect | Moltis | OpenClaw |
|--------|--------|----------|
| **Path validation** | Strict allowlist (MEMORY.md, memory.md, memory/*.md) | No special memory path restrictions |
| **Traversal prevention** | Rejects `..`, absolute paths, non-.md extensions | Relies on workspace sandboxing |
| **Size limit** | 50 KB per write | No documented limit |
| **Write scope** | Only memory files | Any file in workspace |
| **Mechanism** | `validate_memory_path()` in `MemoryWriter` | Workspace access mode (rw/ro/none) |

### Search Features

| Feature | Moltis | OpenClaw |
|---------|--------|----------|
| **LLM reranking** | Optional (configurable) | Built-in with QMD |
| **Citations** | Configurable (auto/on/off) | Configurable (auto/on/off) |
| **Result format** | Chunk ID, path, source, line range, score, text | Path, line range, score, snippet (~700 chars) |
| **Fallback** | Keyword-only if no embeddings | BM25-only if no embeddings |

### Configuration

| Setting | Moltis (`moltis.toml`) | OpenClaw (`openclaw.json`) |
|---------|------------------------|---------------------------|
| **Backend** | `memory.backend = "builtin"` | `memory.backend = "builtin"` |
| **Provider** | `memory.provider = "local"` | Auto-detect from available keys |
| **Citations** | `memory.citations = "auto"` | `memory.citations = "auto"` |
| **LLM reranking** | `memory.llm_reranking = false` | Via QMD config |
| **Session export** | `memory.session_export = true` | `memorySearch.experimental.sessionMemory` |
| **UI configuration** | Settings > Memory page | Config file only |
| **QMD settings** | `[memory.qmd]` section | `memory.backend = "qmd"` |

### CLI Commands

| Command | Moltis | OpenClaw |
|---------|--------|----------|
| **Status** | Settings > Memory (web UI) | `openclaw memory status [--deep]` |
| **Index/reindex** | Automatic on startup | `openclaw memory index [--verbose]` |
| **Search** | Via agent tool only | `openclaw memory search "query"` |
| **Per-agent scoping** | Single agent | `--agent <id>` flag |

### Architecture

| Aspect | Moltis | OpenClaw |
|--------|--------|----------|
| **Language** | Rust | TypeScript/Node.js |
| **Memory crate/module** | `moltis-memory` crate | `memory-core` plugin |
| **Write abstraction** | `MemoryWriter` trait (shared by tools and silent turn) | Direct file I/O via `write_file` tool |
| **Plugin system** | Memory is a core crate | Memory is a swappable plugin slot |
| **Multi-agent** | Single agent | Per-agent memory isolation |

## What Moltis Has That OpenClaw Does Not

- **Dedicated `memory_save` tool** with path validation and immediate
  re-indexing, reducing reliance on the system prompt for write guidance
- **Ollama embedding support** as a provider option
- **Custom OpenAI-compatible embedding endpoints**
- **Circuit breaker** with automatic fallback chain for embedding providers
- **Web UI for memory configuration** (Settings > Memory page)
- **Pure Rust implementation** with zero external runtime dependencies

## What OpenClaw Has That Moltis Does Not (Yet)

- **Manual `/compact` command** with user-specified instructions
- **CLI memory commands** (`status`, `index`, `search`) for debugging
- **Session pruning** (cache-TTL based trimming of old tool results)
- **Gemini and Voyage embedding providers**
- **Per-agent memory isolation** for multi-agent setups
- **Automatic re-indexing on embedding provider/model change** (fingerprint
  detection)
- **Memory plugin slot** allowing third-party memory implementations
- **Flush-once-per-compaction tracking** to avoid redundant silent turns
- **Configurable flush threshold** (soft threshold tokens before compaction)

## Summary

The two systems are architecturally equivalent -- both use Markdown files,
hybrid search, and pre-compaction memory flushes. The main differences are:

1. **Tool approach**: Moltis provides a purpose-built `memory_save` tool with
   security validation; OpenClaw uses a general-purpose `write_file` tool
   guided by the system prompt.

2. **Write safety**: Moltis validates write paths at the tool level (allowlist
   + traversal checks); OpenClaw relies on workspace-level access control.

3. **Implementation**: Moltis is pure Rust with a `MemoryWriter` trait
   abstraction; OpenClaw is TypeScript with direct file I/O through a plugin
   system.

4. **Maturity**: OpenClaw has more CLI tooling and configuration knobs for
   advanced memory management; Moltis has a simpler, more opinionated setup
   with a web UI.
