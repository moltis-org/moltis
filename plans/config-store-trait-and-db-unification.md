# Config Store Trait + DB Unification Plan

## Goal

Introduce a storage abstraction for configuration and credentials so Moltis can:

1. Stop depending on file-only config writes for mutable runtime settings.
2. Use SQLite as the primary durable config backend.
3. Prepare a clean migration path to PostgreSQL later (without rewriting business logic).

## Scope

In scope:

- New `ConfigStore` trait and implementations.
- Migration of mutable config/credential paths to the trait.
- Backward-compatible bootstrap from `moltis.toml` and existing JSON files.
- Tests and migration tooling.

Out of scope (for this branch):

- Full PostgreSQL implementation.
- Removal of `moltis.toml` support.
- Reworking session/message/memory storage formats.

## Current State (Baseline)

- `crates/config/src/loader.rs` handles discovery, load, save, and update directly against files.
- Provider keys are file-backed in `provider_setup::KeyStore` (`provider_keys.json`).
- Auth credentials are already SQLite-backed via `CredentialStore`, but are not behind a reusable trait.
- Several crates already use trait-based persistence (`ProjectStore`, `CronStore`, `MemoryStore`, `ChannelStore`, `MetricsStore`).

## Proposed Architecture

### 1) New trait

Add `ConfigStore` trait in `crates/config` (or a new small crate if needed):

- `load_effective() -> MoltisConfig` (resolved view)
- `load_raw() -> MoltisConfig` (without env overrides)
- `save_raw(&MoltisConfig)`
- `update_raw(fn(&mut MoltisConfig))`
- `source_metadata()` (where values came from, optional)

Keep env overrides as a separate layer, applied after store reads.

### 2) Implementations

- `FileConfigStore` (wrap current behavior; default for compatibility)
- `SqliteConfigStore` (new)

For credentials/provider settings:

- Add trait wrappers (`ProviderConfigStore`, optionally `AuthStore`) or fold into a unified settings store module.
- First implementation for provider config: SQLite table + migration from `provider_keys.json`.

### 3) Composition pattern

Use a deterministic precedence model:

1. DB/file raw config
2. Env overrides
3. CLI flags

This keeps existing behavior while changing persistence internals.

## Data Model (SQLite, initial)

Minimal first model:

- `config_documents`
  - `id` (singleton key)
  - `format_version`
  - `payload_json` (serialized `MoltisConfig`)
  - `updated_at`

- `provider_configs`
  - `provider` (pk)
  - `api_key` (encrypted/secret handling policy unchanged for now)
  - `base_url`
  - `model`
  - `updated_at`

Optional follow-up:

- Split `payload_json` into normalized tables once write-paths stabilize.

## Migration Strategy

### Phase A: Introduce trait without behavior change

- Add `ConfigStore` + `FileConfigStore`.
- Refactor `discover_and_load/save/update` to call trait-backed internals.
- Keep current external APIs intact.

### Phase B: Add SQLite config store behind feature flag or config toggle

- Add schema migration.
- Add startup selection logic:
  - default: file
  - optional: sqlite (`[config] backend = "sqlite"` or env var)

### Phase C: Migrate provider key storage

- Replace direct `KeyStore` file logic with trait-backed provider config store.
- One-time importer: `provider_keys.json` -> SQLite table.
- Keep file read fallback for one release window.

### Phase D: Harden and make SQLite default (optional decision point)

- After stability + telemetry + docs, switch default backend to sqlite.
- Keep export/import and rollback path to file backend.

## Testing Plan

- Unit tests for `ConfigStore` contract (shared test suite applied to file + sqlite impls).
- Migration tests:
  - file -> sqlite bootstrap
  - provider_keys JSON -> sqlite rows
- Concurrency tests for `update_raw` lock semantics.
- Integration tests in gateway startup path ensuring same effective config behavior as today.

## Rollout and Safety

- Add explicit backup/export command before first migration.
- Make migration idempotent.
- On migration failure: continue in read-only file mode with warning, do not crash boot.

## PostgreSQL Readiness Notes

To ease later move to PostgreSQL:

- Keep trait methods async and backend-agnostic.
- Avoid SQLite-specific SQL in business logic.
- Keep serialization boundary at store layer.
- Prefer sqlx query files or repository module boundaries so backend-specific SQL is isolated.

## Suggested Branch Execution Breakdown

1. `feat(config): introduce ConfigStore trait with file implementation`
2. `feat(config): add sqlite config store and migrations`
3. `refactor(gateway): route config loader through store abstraction`
4. `feat(provider): migrate provider key storage behind trait`
5. `test(config): add contract and migration coverage`
6. `docs(config): document backend selection and migration/rollback`

## Open Decisions

- Should sqlite become default immediately or after one release cycle?
- Do we keep full `MoltisConfig` blob storage long-term, or normalize early?
- Where to keep secret material policy for provider keys (existing file parity vs encrypted at rest)?
