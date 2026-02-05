# SQLite Database Migrations

Moltis uses [sqlx](https://github.com/launchbadge/sqlx) for database access and its
built-in migration system for schema management. Each crate owns its migrations,
keeping schema definitions close to the code that uses them.

## Architecture

Each crate that uses SQLite has its own `migrations/` directory and exposes a
`run_migrations()` function. The gateway orchestrates running all migrations at
startup in the correct dependency order.

```
crates/
├── projects/
│   ├── migrations/
│   │   └── 20240205100000_init.sql   # projects table
│   └── src/lib.rs                     # run_migrations()
├── sessions/
│   ├── migrations/
│   │   └── 20240205100001_init.sql   # sessions, channel_sessions
│   └── src/lib.rs                     # run_migrations()
├── cron/
│   ├── migrations/
│   │   └── 20240205100002_init.sql   # cron_jobs, cron_runs
│   └── src/lib.rs                     # run_migrations()
├── gateway/
│   ├── migrations/
│   │   └── 20240205100003_init.sql   # auth, message_log, channels
│   └── src/server.rs                  # orchestrates moltis.db migrations
└── memory/
    ├── migrations/
    │   └── 20240205100004_init.sql   # files, chunks, embedding_cache, FTS
    └── src/lib.rs                     # run_migrations() (separate memory.db)
```

## How It Works

### Migration Ownership

Each crate is autonomous and owns its schema:

| Crate | Database | Tables | Migration File |
|-------|----------|--------|----------------|
| `moltis-projects` | `moltis.db` | `projects` | `20240205100000_init.sql` |
| `moltis-sessions` | `moltis.db` | `sessions`, `channel_sessions` | `20240205100001_init.sql` |
| `moltis-cron` | `moltis.db` | `cron_jobs`, `cron_runs` | `20240205100002_init.sql` |
| `moltis-gateway` | `moltis.db` | `auth_*`, `passkeys`, `api_keys`, `env_variables`, `message_log`, `channels` | `20240205100003_init.sql` |
| `moltis-memory` | `memory.db` | `files`, `chunks`, `embedding_cache`, `chunks_fts` | `20240205100004_init.sql` |

### Startup Sequence

The gateway runs migrations in dependency order:

```rust
// server.rs
moltis_projects::run_migrations(&db_pool).await?;   // 1. projects first
moltis_sessions::run_migrations(&db_pool).await?;   // 2. sessions (FK → projects)
moltis_cron::run_migrations(&db_pool).await?;       // 3. cron (independent)
sqlx::migrate!("./migrations").run(&db_pool).await?; // 4. gateway tables
```

Sessions depends on projects due to a foreign key (`sessions.project_id` references
`projects.id`), so projects must migrate first.

### Version Tracking

sqlx tracks applied migrations in the `_sqlx_migrations` table:

```sql
SELECT version, description, installed_on, success FROM _sqlx_migrations;
```

Migrations are identified by their timestamp prefix (e.g., `20240205100000`), which
must be globally unique across all crates.

## Database Files

| Database | Location | Crates |
|----------|----------|--------|
| `moltis.db` | `~/.moltis/moltis.db` | projects, sessions, cron, gateway |
| `memory.db` | `~/.moltis/memory.db` | memory (separate, managed internally) |

## Adding New Migrations

### Adding a Column to an Existing Table

1. Create a new migration file in the owning crate:

```bash
# Example: adding tags to sessions
touch crates/sessions/migrations/20240301120000_add_tags.sql
```

2. Write the migration SQL:

```sql
-- 20240301120000_add_tags.sql
ALTER TABLE sessions ADD COLUMN tags TEXT;
CREATE INDEX IF NOT EXISTS idx_sessions_tags ON sessions(tags);
```

3. Rebuild to embed the migration:

```bash
cargo build
```

### Adding a New Table to an Existing Crate

1. Create the migration file with a new timestamp:

```bash
touch crates/sessions/migrations/20240302100000_session_bookmarks.sql
```

2. Write the CREATE TABLE statement:

```sql
-- 20240302100000_session_bookmarks.sql
CREATE TABLE IF NOT EXISTS session_bookmarks (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    session_key TEXT NOT NULL,
    name       TEXT NOT NULL,
    message_id INTEGER NOT NULL,
    created_at INTEGER NOT NULL
);
```

### Adding Tables to a New Crate

1. Create the migrations directory:

```bash
mkdir -p crates/new-feature/migrations
```

2. Create the migration file with a globally unique timestamp:

```bash
touch crates/new-feature/migrations/20240401100000_init.sql
```

3. Add `run_migrations()` to the crate's `lib.rs`:

```rust
pub async fn run_migrations(pool: &sqlx::SqlitePool) -> anyhow::Result<()> {
    sqlx::migrate!("./migrations").run(pool).await?;
    Ok(())
}
```

4. Call it from `server.rs` in the appropriate order:

```rust
moltis_new_feature::run_migrations(&db_pool).await?;
```

## Timestamp Convention

Use `YYYYMMDDHHMMSS` format for migration filenames:

- `YYYY` - 4-digit year
- `MM` - 2-digit month
- `DD` - 2-digit day
- `HH` - 2-digit hour (24h)
- `MM` - 2-digit minute
- `SS` - 2-digit second

This ensures global uniqueness across crates. When adding migrations, use the
current timestamp to avoid collisions.

## SQLite Limitations

### ALTER TABLE

SQLite has limited `ALTER TABLE` support:

- **ADD COLUMN**: Supported ✓
- **DROP COLUMN**: SQLite 3.35+ only
- **Rename column**: Requires table recreation
- **Change column type**: Requires table recreation

For complex schema changes, use the table recreation pattern:

```sql
-- Create new table with desired schema
CREATE TABLE sessions_new (
    -- new schema
);

-- Copy data (map old columns to new)
INSERT INTO sessions_new SELECT ... FROM sessions;

-- Swap tables
DROP TABLE sessions;
ALTER TABLE sessions_new RENAME TO sessions;

-- Recreate indexes
CREATE INDEX idx_sessions_created_at ON sessions(created_at);
```

### Foreign Keys

SQLite foreign keys are checked at insert/update time, not migration time. Ensure
migrations run in dependency order (parent table first).

## Testing

Unit tests use in-memory databases with the crate's `init()` method:

```rust
#[tokio::test]
async fn test_session_operations() {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();

    // Create schema for tests (init() retained for this purpose)
    SqliteSessionMetadata::init(&pool).await.unwrap();

    let meta = SqliteSessionMetadata::new(pool);
    // ... test code
}
```

The `init()` methods are retained (marked `#[doc(hidden)]`) specifically for tests.
In production, migrations handle schema creation.

## Troubleshooting

### "failed to run migrations"

1. Check file permissions on `~/.moltis/`
2. Ensure the database file isn't locked by another process
3. Check for syntax errors in migration SQL files

### Migration Order Issues

If you see foreign key errors, verify the migration order in `server.rs`. Parent
tables must be created before child tables with FK references.

### Checking Migration Status

```bash
sqlite3 ~/.moltis/moltis.db "SELECT version, description, success FROM _sqlx_migrations ORDER BY version"
```

### Resetting Migrations (Development Only)

```bash
# Backup first!
rm ~/.moltis/moltis.db
cargo run  # Creates fresh database with all migrations
```

## Best Practices

### DO

- Use timestamp-based version numbers for global uniqueness
- Keep each crate's migrations in its own directory
- Use `IF NOT EXISTS` for idempotent initial migrations
- Test migrations on a copy of production data before deploying
- Keep migrations small and focused

### DON'T

- Modify existing migration files after deployment
- Reuse timestamps across crates
- Put multiple crates' tables in one migration file
- Skip the dependency order in `server.rs`
