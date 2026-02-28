---
description: "Moltis engineering guide for Claude/Codex agents: Rust architecture, testing, security, and release workflows"
alwaysApply: true
---

# CLAUDE.md

Rust version of openclaw ([docs](https://docs.openclaw.ai), [code](https://github.com/openclaw/openclaw)).
All code must have tests with high coverage. Always check for security.

## Cargo Features

Enable new feature flags **by default** in `crates/cli/Cargo.toml` (opt-out, not opt-in):
```toml
[features]
default = ["foo", ...]
foo = ["moltis-gateway/foo"]
```

## Workspace Dependencies

Add new crates to `[workspace.dependencies]` in root `Cargo.toml`, reference with `{ workspace = true }`.
Never add versions directly in crate `Cargo.toml`. Use latest stable crates.io version.

## Config Schema and Validation

When adding/renaming fields in `MoltisConfig` (`crates/config/src/schema.rs`), also update
`build_schema_map()` in `crates/config/src/validate.rs`. New enum variants for string-typed
fields need updates in `check_semantic_warnings()`.

## Rust Style and Idioms

- Use traits for behaviour boundaries. Prefer generics for hot paths, `dyn Trait` for heterogeneous/runtime dispatch.
- Derive `Default` when all fields have sensible defaults.
- Use concrete types (`struct`/`enum`) over `serde_json::Value` wherever shape is known.
- **Match on types, never strings.** Only convert to strings at serialization/display boundaries.
- Prefer `From`/`Into`/`TryFrom`/`TryInto` over manual conversions. Ask before adding manual conversion paths.
- Prefer streaming over non-streaming API calls.
- Run independent async work concurrently (`tokio::join!`, `futures::join_all`).
- Never use `block_on` inside async context.
- **Forbidden:** `Mutex<()>` / `Arc<Mutex<()>>` — mutex must guard actual state.
- Use `anyhow::Result` for app errors, `thiserror` for library errors. Propagate with `?`.
- **Never `.unwrap()`/`.expect()` in production.** Workspace lints deny these. Use `?`, `ok_or_else`, `unwrap_or_default`, `unwrap_or_else(|e| e.into_inner())` for locks.
- Use `time` crate (workspace dep) for date/time — no manual epoch math or magic constants like `86400`.
- Prefer `chrono` only if already imported in the crate; default to `time` for new code.
- Prefer crates over subprocesses (`std::process::Command`). Use subprocesses only when no mature crate exists.
- Prefer guard clauses (early returns) over nested `if` blocks.
- Prefer iterators/combinators over manual loops. Use `Cow<'_, str>` when allocation is conditional.
- Keep public API surfaces small. Use `#[must_use]` where return values matter.

### Tracing and Metrics

All crates must have `tracing` and `metrics` features, gated with `#[cfg(feature = "...")]`.
Use `tracing::instrument` on async functions. Record metrics at key points (counts, durations, errors).
See `docs/metrics-and-tracing.md`.

## Build Commands

```bash
cargo build                  # Debug build
cargo build --release        # Release build
cargo run / cargo run --release
```

## Web UI Assets

Assets in `crates/web/src/assets/` (JS, CSS, HTML). Dev mode serves from disk (edit and reload);
release mode embeds via `include_dir!` with versioned URLs.

- Run `biome check --write` after editing JS files.
- Avoid creating HTML from JS — add hidden elements in `index.html`, toggle visibility. Preact/HTM exceptions allowed.
- **Always use Tailwind classes** instead of inline `style="..."`.
- Reuse CSS classes from `components.css`: `provider-btn`, `provider-btn-secondary`, `provider-btn-danger`.
- Match button heights/text sizes when elements sit together.
- **Rebuild Tailwind** after adding new classes:
  ```bash
  cd crates/web/ui && npx tailwindcss -i input.css -o ../src/assets/style.css --minify
  ```

### Selection Cards

Use clickable cards (`.model-card`, `.backend-card` in `input.css`) instead of dropdowns for option selection.
States: `.selected`, `.disabled`, default. Badges: `.recommended-badge`, `.tier-badge`.

### Provider Config Storage

Provider keys in `~/.config/moltis/provider_keys.json` via `KeyStore` in `provider_setup.rs`.
When adding fields, update: `ProviderConfig` struct, `available()` response, `save_key()`.

### Server-Injected Data (gon pattern)

For server data needed at page load: add to `GonData` in `server.rs` / `build_gon_data()`.
JS side: `import * as gon from "./gon.js"` — use `gon.get()`, `gon.onChange()`, `gon.refresh()`.
Never inject inline `<script>` tags or build HTML in Rust.

### Event Bus

Server events via WebSocket: `import { onEvent } from "./events.js"`. Returns unsubscribe function.
Do **not** use `window.addEventListener`/`CustomEvent` for server events.

## API Namespace Convention

Each UI tab gets its own API namespace: REST `/api/<feature>/...` and RPC `<feature>.*`.
Never merge features into a single endpoint.

## Channel Message Handling

**Always respond to approved senders** — no silent failures. Send error/fallback messages
for LLM failures, transcription failures, unhandled message types. Access control via
allowlist/OTP flow.

## Authentication Architecture

Password + passkey (WebAuthn) auth in `crates/gateway/src/auth.rs`, routes in `auth_routes.rs`,
middleware in `auth_middleware.rs`. Setup code printed to terminal on first run.
`RequireAuth` middleware protects `/api/*` except `/api/auth/*` and `/api/gon`.
`CredentialStore` persists argon2-hashed passwords, passkeys, API keys, sessions to JSON.

CLI: `moltis auth reset-password`, `moltis auth reset-identity`.

## Testing

```bash
cargo test                           # All tests
cargo test <test_name>               # Specific test
cargo test -- --nocapture            # With stdout
```

### E2E Tests (Web UI)

**Every web UI change needs E2E tests.** Tests in `crates/web/ui/e2e/specs/` using Playwright.
Helpers in `e2e/helpers.js`.

```bash
cd crates/web/ui
npx playwright test                              # All
npx playwright test e2e/specs/chat-input.spec.js # Specific
```

Rules: use `getByRole()`/`getByText({ exact: true })` selectors, shared helpers
(`navigateAndWait`, `waitForWsConnected`, `watchPageErrors`), assert no JS errors,
avoid `waitForTimeout()`.

## Code Quality

- Never run `cargo fmt` on stable in this repo. Always use the pinned nightly rustfmt (`just format`, `just format-check`, or `cargo +nightly-2025-11-30 fmt ...`).

```bash
just format              # Format Rust (pinned nightly)
just format-check        # CI format check
just release-preflight   # fmt + clippy gates
cargo check              # Fast compile check
taplo fmt                # Format TOML files
biome check --write      # Lint/format JS
```

## Sandbox Architecture

Containers (Docker or Apple Container) in `crates/tools/src/sandbox.rs` (trait + impls),
`exec.rs` (ExecTool), `crates/cli/src/sandbox_commands.rs` (CLI), `crates/config/src/schema.rs` (config).

Pre-built images use deterministic hash tags from base image + packages. Default packages
in `default_sandbox_packages()`. CLI: `moltis sandbox {list,build,remove,clean}`.

## Logging Levels

- `error!` — unrecoverable. `warn!` — unexpected but recoverable. `info!` — operational milestones.
- `debug!` — detailed diagnostics. `trace!` — very verbose per-item data.
- **Common mistake:** `warn!` for unconfigured providers — use `debug!` for expected "not configured" states.

## Security

- **WebSocket Origin validation**: `server.rs` rejects cross-origin WS upgrades (403). Loopback variants equivalent.
- **SSRF protection**: `web_fetch.rs` blocks loopback/private/link-local/CGNAT IPs. Preserve this on changes.
- **Secrets**: Use `secrecy::Secret<String>` for all passwords/keys/tokens. `expose_secret()` only at consumption point. Manual `Debug` impl with `[REDACTED]`. Scope `RwLock` read guards in blocks to avoid deadlocks. See `crates/oauth/src/types.rs` for serde helpers.
- **Never commit** passwords, credentials, `.env` with real values, or PII.
- If secrets accidentally committed: `git reset HEAD~1`, remove, re-commit. If pushed, rotate immediately.

## Data and Config Directories

- **Config**: `moltis_config::config_dir()` (`~/.moltis/`). Contains `moltis.toml`, `credentials.json`, `mcp-servers.json`.
- **Data**: `moltis_config::data_dir()` (`~/.moltis/`). Contains DBs, sessions, logs, memory files.
- **Never** use `directories::BaseDirs` outside `moltis-config`. Never use `std::env::current_dir()` for storage.
- Workspace-scoped files (`MEMORY.md`, `memory/*.md`, etc.) resolve relative to `data_dir()`.
- Gateway resolves `data_dir` once at startup; prefer that value over repeated calls.

## Database Migrations

sqlx migrations, each crate owns its `migrations/` directory. See `docs/sqlite-migration.md`.

| Crate | Tables |
|-------|--------|
| `moltis-projects` | `projects` |
| `moltis-sessions` | `sessions`, `channel_sessions` |
| `moltis-cron` | `cron_jobs`, `cron_runs` |
| `moltis-gateway` | `auth_*`, `passkeys`, `api_keys`, `env_variables`, `message_log`, `channels` |
| `moltis-memory` | `files`, `chunks`, `embedding_cache`, `chunks_fts` |

New migration: `crates/<crate>/migrations/YYYYMMDDHHMMSS_description.sql` (use `IF NOT EXISTS`).
New crate: add `run_migrations()` to `lib.rs`, call from `server.rs` in dependency order.

## Provider Implementation

- **Async all the way down** — never `block_on` in async context. All HTTP/IO must be async.
- Make model lists broad (API errors handle unavailable models). Check `../clawdbot/` for reference.
- BYOM providers (OpenRouter, Ollama): require user config, don't hardcode models.

## Changelog

Update `[Unreleased]` in `CHANGELOG.md` ([Keep a Changelog](https://keepachangelog.com/en/1.1.0/))
for user-facing changes: Added, Changed, Deprecated, Removed, Fixed, Security.

## Git Workflow

Conventional commits: `feat|fix|docs|style|refactor|test|chore(scope): description`
**No `Co-Authored-By` trailers.** Update `README.md` features list with `feat` commits.

### Releases

- Never overwrite tags — always create new version. `[workspace.package].version` must match tag.
- Use `./scripts/prepare-release.sh <version> [date]` for release prep.
- Deploy template tags updated automatically by CI — don't manually update.

### Lockfile

- `cargo fetch` to sync (not `cargo update`). Verify with `cargo fetch --locked`. `local-validate.sh` auto-handles.
- `cargo update --workspace` only for intentional upgrades.

### Local Validation

**Always** run `./scripts/local-validate.sh <PR_NUMBER>` when a PR exists.

Exact commands (must match `local-validate.sh`):
- Fmt: `cargo +nightly-2025-11-30 fmt --all -- --check`
- Clippy: `cargo +nightly-2025-11-30 clippy -Z unstable-options --workspace --all-features --all-targets --timings -- -D warnings`
- macOS without `nvcc`: clippy without `--all-features`
- macOS app (Darwin hosts): `./scripts/build-swift-bridge.sh && ./scripts/generate-swift-project.sh && ./scripts/lint-swift.sh && xcodebuild -project apps/macos/Moltis.xcodeproj -scheme Moltis -configuration Release -destination "platform=macOS" -derivedDataPath apps/macos/.derivedData-local-validate build`
- iOS app (Darwin hosts): `cargo run -p moltis-schema-export -- apps/ios/GraphQL/Schema/schema.graphqls && ./scripts/generate-ios-graphql.sh && ./scripts/generate-ios-project.sh && xcodebuild -project apps/ios/Moltis.xcodeproj -scheme Moltis -configuration Debug -destination "generic/platform=iOS" CODE_SIGNING_ALLOWED=NO build`

### PR Descriptions

Required sections: `## Summary`, `## Validation` (checkboxes, split into `### Completed` / `### Remaining`
with exact commands), `## Manual QA`. Include concrete test steps.

## Code Quality Checklist

**Run before every commit:**
- [ ] No secrets or private tokens (CRITICAL)
- [ ] `taplo fmt` (TOML changes)
- [ ] `biome check --write` (JS changes)
- [ ] Rust fmt passes (exact command above)
- [ ] Rust clippy passes (exact command above)
- [ ] `just release-preflight` passes
- [ ] `cargo test` passes
- [ ] Conventional commit message
- [ ] No debug code or temp files

## Documentation

Source in `docs/src/` (mdBook). Auto-deployed to docs.moltis.org on push to main.
Update `docs/src/SUMMARY.md` when adding pages. Preview: `cd docs && mdbook serve`.

## Session Completion

**Work is NOT complete until `git push` succeeds.** Mandatory steps:
1. File issues for remaining work
2. Run quality gates
3. Update issue status
4. **Push**: `git pull --rebase && bd sync && git push && git status`
5. Clean up stashes/branches
6. Hand off context

## Issue Tracking

Uses **bd (beads)**: `bd ready`, `bd create "Title" --type task --priority 2`,
`bd close <id>`, `bd sync` (run at session end). Full details: `bd prime`.

## Plans and Session History

Plans in `prompts/`. After significant work, write summary to
`prompts/session-YYYY-MM-DD-<topic>.md`.
