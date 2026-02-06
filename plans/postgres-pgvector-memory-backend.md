# PostgreSQL + pgvector Memory Backend

**Status:** Not implemented
**Priority:** Medium
**Complexity:** Medium
**Platform:** All (requires external Postgres instance)

## Overview

Add PostgreSQL with pgvector as a feature-gated alternative to the SQLite
memory backend. The main database (`moltis.db`) stays SQLite — only the memory
system (`memory.db`) gets a Postgres option. This gives power users native
vector similarity search (HNSW indexes), real concurrency, and multi-user
support while preserving the zero-dependency default.

### Why Postgres + pgvector?

| Aspect | SQLite (current) | Postgres + pgvector |
|--------|-------------------|---------------------|
| Vector search | O(n) full scan in Rust | HNSW index, sub-linear |
| Keyword search | FTS5 | tsvector/tsquery + GIN |
| Concurrency | Single writer (WAL) | Full MVCC |
| Multi-user | No | Yes |
| Deployment | Zero-dependency | Requires Postgres container |
| Scale sweet spot | <5k chunks | 5k–1M+ chunks |

Current vector search loads **all** chunks into memory and computes cosine
similarity in Rust. Fine for a single user's memory files (~1k–5k chunks)
but doesn't scale. pgvector's HNSW index pushes similarity into the DB
engine with sub-linear performance.

### Why Not Replace SQLite Entirely?

- The main DB (projects, sessions, cron, auth) is low-volume CRUD — SQLite
  is ideal and adds zero operational overhead.
- Session JSONL files are append-only and work well as-is.
- The "just run the binary" UX is the biggest differentiator vs clawdbot.
- Only the memory/embedding workload benefits from Postgres.

## Architecture

```
MemoryStore trait (unchanged)
├── SqliteMemoryStore  (default, existing)
└── PgMemoryStore      (new, behind `postgres` feature)
```

Backend selected at startup via `[memory] backend` config. Both can be
compiled into the same binary (runtime selection).

## Changes

### 1. Workspace dependencies — `Cargo.toml` (root)

Split sqlx features so sqlite and postgres are independently selectable:

```toml
[workspace.dependencies]
sqlx = { version = "0.8", features = ["migrate", "runtime-tokio"] }
```

Individual crates opt into `sqlx/sqlite` and/or `sqlx/postgres` via their
own feature flags.

### 2. Feature flags — `crates/memory/Cargo.toml`

```toml
[features]
default = ["sqlite"]
sqlite = ["sqlx/sqlite"]
postgres = ["sqlx/postgres"]
```

Gate `SqliteMemoryStore` behind `#[cfg(feature = "sqlite")]` and the new
`PgMemoryStore` behind `#[cfg(feature = "postgres")]`.

### 3. Feature forwarding — `crates/cli/Cargo.toml`

```toml
[features]
default = ["memory-sqlite", ...]
memory-sqlite = ["moltis-memory/sqlite"]
memory-postgres = ["moltis-memory/postgres"]
```

Both can be enabled simultaneously; the user picks at runtime via config.

### 4. Configuration — `crates/config/src/schema.rs`

Add memory backend config:

```toml
[memory]
backend = "sqlite"                              # or "postgres"
database_url = "postgresql://user:pass@localhost/moltis"  # required for postgres
```

Add `MemoryBackend` enum (`Sqlite | Postgres`) and
`database_url: Option<String>` to the memory config section. Validate at
startup: if backend is `postgres`, `database_url` must be set.

### 5. Postgres memory store — `crates/memory/src/store_postgres.rs`

New file implementing `MemoryStore` for `PgPool`:

| Operation | SQLite | Postgres |
|-----------|--------|----------|
| Embeddings | BLOB (le f32) | `vector(N)` column (pgvector) |
| Vector search | Load all + Rust cosine | `ORDER BY embedding <=> $1` (HNSW) |
| Keyword search | FTS5 `MATCH` | `to_tsvector` / `to_tsquery` |
| FTS sync | Triggers on insert/update/delete | Generated `tsvector` column |
| Cache eviction | `DELETE ... ORDER BY updated_at` | Same, Postgres syntax |

Implementation notes:

- Use `sqlx::query()` (runtime) rather than `sqlx::query!()` (compile-time)
  since compile-time checking requires a live DB at build time.
- pgvector `vector(N)` requires a fixed dimension. Store dimension in config
  and validate on startup. If the embedding model changes dimensions, require
  explicit re-index (`moltis memory reindex`).
- HNSW index creation:
  `CREATE INDEX ON chunks USING hnsw (embedding vector_cosine_ops)`.

### 6. Postgres migrations — `crates/memory/migrations_pg/`

Separate migration directory (SQLite and Postgres SQL are incompatible):

```sql
CREATE EXTENSION IF NOT EXISTS vector;

CREATE TABLE files (
    path TEXT PRIMARY KEY,
    source TEXT,
    hash TEXT,
    mtime BIGINT,
    size BIGINT
);

CREATE TABLE chunks (
    id TEXT PRIMARY KEY,
    path TEXT REFERENCES files(path) ON DELETE CASCADE,
    source TEXT,
    start_line INTEGER,
    end_line INTEGER,
    hash TEXT,
    model TEXT,
    text TEXT,
    embedding vector(1536),
    updated_at TIMESTAMPTZ DEFAULT now()
);

CREATE INDEX idx_chunks_path ON chunks(path);
CREATE INDEX idx_chunks_embedding ON chunks
    USING hnsw (embedding vector_cosine_ops);

ALTER TABLE chunks ADD COLUMN tsv tsvector
    GENERATED ALWAYS AS (to_tsvector('english', coalesce(text, ''))) STORED;
CREATE INDEX idx_chunks_fts ON chunks USING gin(tsv);

CREATE TABLE embedding_cache (
    provider TEXT,
    model TEXT,
    provider_key TEXT,
    hash TEXT,
    embedding vector(1536),
    dims INTEGER,
    updated_at TIMESTAMPTZ DEFAULT now(),
    PRIMARY KEY (provider, model, provider_key, hash)
);
```

The `vector(1536)` dimension matches OpenAI `text-embedding-3-small`. For
other models (e.g. local GGUF at 768 dims), either:
- Use untyped `vector` (works but no HNSW without fixed dimension)
- Require dimension in config and create the table dynamically
- Use `halfvec` for 50% memory savings on large datasets

### 7. Backend selection — `crates/gateway/src/server.rs`

```rust
let memory_store: Arc<dyn MemoryStore> = match config.memory.backend {
    MemoryBackend::Sqlite => {
        let pool = SqlitePool::connect(&sqlite_url).await?;
        // run sqlite migrations
        Arc::new(SqliteMemoryStore::new(pool))
    }
    #[cfg(feature = "postgres")]
    MemoryBackend::Postgres => {
        let url = config.memory.database_url
            .as_ref()
            .ok_or_else(|| anyhow!("memory.database_url required"))?;
        let pool = PgPool::connect(url).await?;
        // run postgres migrations
        Arc::new(PgMemoryStore::new(pool))
    }
};
```

### 8. Embedding dimension handling

pgvector needs a known dimension for HNSW indexes:

- Store the embedding dimension in a `memory_meta` table (key-value):
  `("embedding_dims", "1536")`
- On first sync, record the dimension from the configured provider
- On subsequent syncs, if the provider dimension changes, warn and require
  explicit re-index via `moltis memory reindex`
- Without HNSW (no fixed dimension), pgvector still works via sequential
  scan — faster than loading all BLOBs into Rust

### 9. Testing

- Unit tests for `PgMemoryStore` using `sqlx::test` with a test Postgres
  instance
- Use `testcontainers` crate to spin up Postgres+pgvector in CI
- Mirror all existing `SqliteMemoryStore` tests for the Postgres backend
- Integration test: full sync + search cycle against Postgres

### 10. Documentation

- Add `docs/src/postgres-memory.md`:
  - How to set up Postgres + pgvector (Docker one-liner)
  - Configuration in `moltis.toml`
  - When to use Postgres vs SQLite (scale guidelines)
  - Migration from SQLite to Postgres (export/import)
- Update `docs/src/SUMMARY.md`

### 11. Changelog

Under `[Unreleased]`:

```markdown
### Added
- Optional PostgreSQL + pgvector backend for memory system (`memory-postgres`)
- HNSW vector index support for sub-linear similarity search
- `[memory] backend` and `[memory] database_url` configuration options
```

## Risks and Mitigations

| Risk | Mitigation |
|------|------------|
| sqlx compile-time checks don't work for two backends | Use runtime `query()` for Postgres store |
| Embedding dimension changes break HNSW index | Detect mismatch at startup, require explicit reindex |
| CI needs a Postgres instance | `testcontainers` or GitHub Actions service container |
| Postgres adds build-time dep on libpq | Feature-gated; default build doesn't link libpq |
| Migration SQL diverges over time | Keep migrations minimal; complex logic in Rust |

## Out of Scope

- Migrating main DB (`moltis.db`) to Postgres
- Migrating session JSONL storage to Postgres
- Automatic SQLite-to-Postgres data migration tool (future work)
- Multi-tenancy / row-level security

## Implementation Order

1. Gate existing SQLite code behind `#[cfg(feature = "sqlite")]`
2. Add config schema changes (`MemoryBackend` enum, `database_url`)
3. Create `store_postgres.rs` with `PgMemoryStore`
4. Create Postgres migrations in `migrations_pg/`
5. Wire up backend selection in `server.rs`
6. Add feature flags to `memory`, `gateway`, and `cli` crates
7. Write tests (unit + integration)
8. Documentation and changelog
9. CI: add Postgres service container for tests
