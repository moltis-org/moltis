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

- **Always** run `biome check --write` when JS files change.
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

## Adding Channels

When adding a new channel or extending one, follow `docs/channel-integration-checklist.md`.

Minimum bar before shipping:
- Settings reachable from the web UI, with onboarding coverage if the channel is offered there
- Advanced JSON config escape hatch for settings without dedicated HTML fields yet
- Prefer declarative channel field definitions that can drive both HTML forms and advanced JSON guidance
- Storage behavior explained clearly, web UI channel settings live in `data_dir()/moltis.db`, not `moltis.toml`
- Config template, validation, docs, and tests updated in the same PR
- No silent access-control failures, OTP and allowlist behavior must be user-visible

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

- Do **not** add manual `CHANGELOG.md` entries in normal PRs.
- `CHANGELOG.md` entries are generated from commit history via `git-cliff` (`cliff.toml`).
- Use conventional commits and preview unreleased notes with `just changelog-unreleased`.
- PR CI enforces this via `scripts/check-changelog-guard.sh`.

## Git Workflow

Conventional commits: `feat|fix|docs|style|refactor|test|chore(scope): description`
**No `Co-Authored-By` trailers.** Update `README.md` features list with `feat` commits.

### Releases

- Date-based versioning: `YYYYMMDD.NN` (e.g., `20260311.01`). Cargo.toml stays at static `0.1.0`; real version injected via `MOLTIS_VERSION` env var at build time.
- Never overwrite tags — always create new version.
- Use `./scripts/prepare-release.sh [YYYYMMDD.NN]` for release prep (auto-computes next version if omitted).
- Deploy template tags updated automatically by CI — don't manually update.

**Release workflow is two phases:**

1. **Prepare & publish** (can be done in a session):
   ```bash
   ./scripts/prepare-release.sh          # generates changelog, syncs lockfile
   git add -A && git commit -m "chore: prepare release YYYYMMDD.NN"
   git tag YYYYMMDD.NN && git push --follow-tags
   ```
   CI then builds artifacts, generates checksums, Sigstore signatures, and creates the GitHub release. This takes time.

2. **GPG-sign** (must happen later, after CI completes):
   ```bash
   ./scripts/gpg-sign-release.sh [VERSION]
   ```
   This downloads artifacts from the published release, verifies SHA256 checksums, signs each artifact with the maintainer's YubiKey-resident GPG key, and uploads `.asc` files back to the release. **Requires YubiKey tap.**

   Users verify signatures with:
   ```bash
   ./scripts/verify-release.sh --version YYYYMMDD.NN
   ```

**Important:** When asked to create a release, complete phase 1 and remind the maintainer to run `gpg-sign-release.sh` after CI finishes. Do not attempt to run the signing script in the same session — the release artifacts won't exist yet.

### Lockfile

- `cargo fetch` to sync (not `cargo update`). Verify with `cargo fetch --locked`. `local-validate.sh` auto-handles.
- `cargo update --workspace` only for intentional upgrades.

### Local Validation

**Always** run `./scripts/local-validate.sh <PR_NUMBER>` when a PR exists.

For incremental local edits before full validation:
- JS changed: run `biome check --write`.
- Rust changed: run `cargo +nightly-2025-11-30 fmt --all -- --check`.
- JS + Rust changed: run both.

Exact commands (must match `local-validate.sh`):
- Fmt: `cargo +nightly-2025-11-30 fmt --all -- --check`
- Clippy: `just lint` (OS-aware: on macOS excludes CUDA features, on Linux uses `--all-features`)
- Tests: `just test` (OS-aware: on macOS uses nextest without CUDA features, on Linux uses `--all-features`)
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
- [ ] `just lint` passes (OS-aware clippy)
- [ ] `just release-preflight` passes
- [ ] `just test` passes
- [ ] Conventional commit message
- [ ] No debug code or temp files

## Documentation

Source in `docs/src/` (mdBook). Auto-deployed to docs.moltis.org on push to main.
Update `docs/src/SUMMARY.md` when adding pages. Preview: `cd docs && mdbook serve`.

**Keep docs in sync with code.** When adding or changing user-facing features
(config fields, CLI commands, channel behavior, API endpoints, tools), update
the relevant `docs/src/` pages and the config template (`crates/config/src/template.rs`)
in the same PR. Documentation drift causes real user confusion — treat outdated
docs as a bug.

## Session Completion

**Work is NOT complete until `git push` succeeds.** Mandatory steps:
1. File issues for remaining work
2. Run quality gates
3. Update issue status
4. **Push**: `git pull --rebase && bd dolt commit && git push && git status`
   If this repo uses a Dolt remote for beads, also run `bd dolt pull` / `bd dolt push`.
5. Clean up stashes/branches
6. Hand off context

## Plans and Session History

Plans in `prompts/`. After significant work, write summary to
`prompts/session-YYYY-MM-DD-<topic>.md`.

<!-- BEGIN BEADS INTEGRATION -->
## Issue Tracking with bd (beads)

**IMPORTANT**: This project uses **bd (beads)** for ALL issue tracking. Do NOT use markdown TODOs, task lists, or other tracking methods.

### Why bd?

- Dependency-aware: Track blockers and relationships between issues
- Git-friendly: Dolt-powered version control with native sync
- Agent-optimized: JSON output, ready work detection, discovered-from links
- Prevents duplicate tracking systems and confusion

### Quick Start

**Check for ready work:**

```bash
bd ready --json
```

**Create new issues:**

```bash
bd create "Issue title" --description="Detailed context" -t bug|feature|task -p 0-4 --json
bd create "Issue title" --description="What this issue is about" -p 1 --deps discovered-from:bd-123 --json
```

**Claim and update:**

```bash
bd update <id> --claim --json
bd update bd-42 --priority 1 --json
```

**Complete work:**

```bash
bd close bd-42 --reason "Completed" --json
```

### Issue Types

- `bug` - Something broken
- `feature` - New functionality
- `task` - Work item (tests, docs, refactoring)
- `epic` - Large feature with subtasks
- `chore` - Maintenance (dependencies, tooling)

### Priorities

- `0` - Critical (security, data loss, broken builds)
- `1` - High (major features, important bugs)
- `2` - Medium (default, nice-to-have)
- `3` - Low (polish, optimization)
- `4` - Backlog (future ideas)

### Workflow for AI Agents

1. **Check ready work**: `bd ready` shows unblocked issues
2. **Claim your task atomically**: `bd update <id> --claim`
3. **Work on it**: Implement, test, document
4. **Discover new work?** Create linked issue:
   - `bd create "Found bug" --description="Details about what was found" -p 1 --deps discovered-from:<parent-id>`
5. **Complete**: `bd close <id> --reason "Done"`

### Auto-Sync

bd automatically syncs via Dolt:

- Each write auto-commits to Dolt history
- Use `bd dolt push`/`bd dolt pull` for remote sync
- No manual export/import needed!

### Worktrees

If you create a git worktree with plain `git worktree add`, Beads will not
automatically share the main checkout's `.beads` state. For an existing
worktree, run:

```bash
./scripts/bd-worktree-attach.sh
```

This writes `.beads/redirect` so the worktree uses the main repository's Beads
database. If you create worktrees through `bd worktree create`, it should set
up the redirect for you automatically.

### Important Rules

- ✅ Use bd for ALL task tracking
- ✅ Always use `--json` flag for programmatic use
- ✅ Link discovered work with `discovered-from` dependencies
- ✅ Check `bd ready` before asking "what should I work on?"
- ❌ Do NOT create markdown TODO lists
- ❌ Do NOT use external issue trackers
- ❌ Do NOT duplicate tracking systems

For more details, see README.md and docs/QUICKSTART.md.

## Landing the Plane (Session Completion)

**When ending a work session**, you MUST complete ALL steps below. Work is NOT complete until `git push` succeeds.

**MANDATORY WORKFLOW:**

1. **File issues for remaining work** - Create issues for anything that needs follow-up
2. **Run quality gates** (if code changed) - Tests, linters, builds
3. **Update issue status** - Close finished work, update in-progress items
4. **PUSH TO REMOTE** - This is MANDATORY:
   ```bash
   git pull --rebase
   bd sync
   git push
   git status  # MUST show "up to date with origin"
   ```
5. **Clean up** - Clear stashes, prune remote branches
6. **Verify** - All changes committed AND pushed
7. **Hand off** - Provide context for next session

**CRITICAL RULES:**
- Work is NOT complete until `git push` succeeds
- NEVER stop before pushing - that leaves work stranded locally
- NEVER say "ready to push when you are" - YOU must push
- If push fails, resolve and retry until it succeeds

<!-- END BEADS INTEGRATION -->
