# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## General

This is doing a Rust version of openclaw. Openclaw documentation is available at
https://docs.openclaw.ai and its code is at https://github.com/openclaw/openclaw

Dig this repo and documentation to figure out how moltbot is working and how
many features it has. `../clawdbot/HOWITWORKS.md` has explaination of how it
works. But feel free to do any improvement and change the way it is to make
it more Rustacean.

All code you write must have tests with high coverage.

## Rust Style and Idioms

Write idiomatic, Rustacean code. Prioritize clarity, modularity, and
zero-cost abstractions.

### Traits and generics

- Always use traits to define behaviour boundaries — this allows alternative
  implementations (e.g. swapping MCP transports, storage backends, provider
  SDKs) and makes testing with mocks straightforward.
- Prefer generic parameters (`fn foo<T: MyTrait>(t: T)`) for hot paths where
  monomorphization matters. Use `dyn Trait` (behind `Arc` / `Box`) when you
  need heterogeneous collections or the concrete type isn't known until
  runtime.
- Derive `Default` on structs whenever all fields have sensible defaults — it
  pairs well with struct update syntax and `unwrap_or_default()`.

### Typed data over loose JSON

Use concrete Rust types (`struct`, `enum`) instead of `serde_json::Value`
wherever the shape is known. This gives compile-time guarantees, better
documentation, and avoids stringly-typed field access. Reserve
`serde_json::Value` for truly dynamic / schema-less data.

### Concurrency

- Always prefer streaming over non-streaming API calls when possible.
  Streaming provides a better, friendlier user experience by showing
  responses as they arrive.
- Run independent async work concurrently with `tokio::join!`,
  `futures::join_all`, or `FuturesUnordered` instead of sequential `.await`
  loops. Sequential awaits are fine when each step depends on the previous
  result.
- Never use `block_on` or any blocking call inside an async context (see
  "Async all the way down" below).

### Error handling

- Use `anyhow::Result` for application-level errors and `thiserror` for
  library-level errors that callers need to match on.
- Propagate errors with `?`; avoid `.unwrap()` outside of tests.

### General style

- Prefer iterators and combinators (`.map()`, `.filter()`, `.collect()`)
  over manual loops when they express intent more clearly.
- Use `Cow<'_, str>` when a function may or may not need to allocate.
- Keep public API surfaces small: expose only what downstream crates need
  via `pub use` re-exports in `lib.rs`.
- Prefer `#[must_use]` on functions whose return value should not be
  silently ignored.

### Tracing and Metrics

**All crates must include tracing and metrics instrumentation.** This is
critical for telemetry, debugging, and production observability.

- Add `tracing` feature to crate's `Cargo.toml` and gate instrumentation
  with `#[cfg(feature = "tracing")]`
- Add `metrics` feature and gate counters/gauges/histograms with
  `#[cfg(feature = "metrics")]`
- Use `tracing::instrument` on async functions for automatic span creation
- Record metrics at key points: operation counts, durations, errors, and
  resource usage

```rust
#[cfg(feature = "tracing")]
use tracing::{debug, instrument, warn};

#[cfg(feature = "metrics")]
use moltis_metrics::{counter, histogram, labels};

#[cfg_attr(feature = "tracing", instrument(skip(self)))]
pub async fn process_request(&self, req: Request) -> Result<Response> {
    #[cfg(feature = "metrics")]
    let start = std::time::Instant::now();

    // ... do work ...

    #[cfg(feature = "metrics")]
    {
        counter!("my_crate_requests_total").increment(1);
        histogram!("my_crate_request_duration_seconds")
            .record(start.elapsed().as_secs_f64());
    }

    Ok(response)
}
```

See `docs/metrics-and-tracing.md` for the full list of available metrics,
Prometheus endpoint configuration, and best practices.

## Build and Development Commands

```bash
cargo build              # Build the project
cargo build --release    # Build with optimizations
cargo run                # Run the project
cargo run --release      # Run with optimizations
```

## Web UI Assets

Assets live in `crates/gateway/src/assets/` (JS, CSS, HTML). The gateway
serves them in two modes:

- **Dev (filesystem)**: When `cargo run` detects the source tree, assets are
  served directly from disk. Edit JS/CSS and reload the browser — no Rust
  recompile needed. You can also set `MOLTIS_ASSETS_DIR` to point elsewhere.
- **Release (embedded)**: When the binary runs outside the repo, assets are
  served from the copy embedded at compile time via `include_dir!`. URLs are
  versioned (`/assets/v/<hash>/...`) with immutable caching; the hash changes
  automatically on each build.

When editing JavaScript files, run `biome check --write` to lint and format
them. No separate asset build step is required.

**HTML in JS**: Avoid creating HTML elements from JavaScript. Instead, add
hidden elements in `index.html` (with `style="display:none"`) and have JS
toggle their visibility. This keeps markup in HTML where it belongs and makes
the structure easier to inspect. Preact components (HTM templates) are the
exception — they use `html` tagged templates by design.

### Styling and UI Consistency

**Always use Tailwind utility classes instead of inline `style="..."` attributes.**
This applies to all properties — spacing (`p-4`, `gap-3`), colors
(`text-[var(--muted)]`, `bg-[var(--surface)]`), typography (`font-mono`,
`text-xs`, `font-medium`), layout (`flex`, `grid`, `items-center`), borders
(`border`, `rounded-md`), and anything else Tailwind covers. Only fall back to
inline styles for truly one-off values that have no Tailwind equivalent (e.g. a
specific `max-width` or `grid-template-columns` pattern).

Keep buttons, links, and other interactive elements visually consistent with
the existing UI. Reuse the shared CSS classes defined in `components.css`:

- **Primary action**: `provider-btn` (green background, white text).
- **Secondary action**: `provider-btn provider-btn-secondary` (surface
  background, border).
- **Destructive action**: `provider-btn provider-btn-danger` (red background,
  white text). Never combine `provider-btn` with inline color overrides for
  destructive buttons.

When buttons or selects sit next to each other (e.g. in a header row), they
must share the same height and text size so they look like a cohesive group.
Use `provider-btn` variants for all of them rather than mixing ad-hoc Tailwind
button styles with different padding/font sizes.

Before creating a new CSS class, check whether an existing one already covers
the use case. Duplicating styles (e.g. a second green-button class) leads to
drift — consolidate instead.

**Building Tailwind**: After adding or changing Tailwind utility classes in JS
or HTML files, you **MUST** rebuild the CSS for the changes to take effect.
Tailwind only generates CSS for classes it finds in the source files at build
time — new classes won't work until CSS is rebuilt:

```bash
cd crates/gateway/ui
npm install              # first time only
npx tailwindcss -i input.css -o ../src/assets/style.css --minify
```

Use `npm run watch` during development for automatic rebuilds on file changes.
If styles don't appear after adding new Tailwind classes, this rebuild step was
likely missed.

### Selection Card UI Pattern

When presenting users with a choice between options (backends, models, plans),
use **clickable cards** instead of dropdowns. Cards provide better UX because:
- Users can see all options at once with descriptions
- Visual feedback (selected state) is clearer
- Badges can highlight recommended options or availability status

**Card structure** (see `.model-card`, `.backend-card` in `input.css`):
```html
<div class="backend-card selected">
  <div class="flex items-center justify-between">
    <span class="text-sm font-medium">Option Name</span>
    <div class="flex gap-2">
      <span class="recommended-badge">Recommended</span>
    </div>
  </div>
  <div class="text-xs text-[var(--muted)] mt-1">Description text</div>
</div>
```

**States**:
- `.selected` — highlighted with accent border/background
- `.disabled` — dimmed, cursor not-allowed, not clickable
- Default — hover shows border-strong and bg-hover

**Badges**:
- `.recommended-badge` — accent color, for the suggested option
- `.tier-badge` — muted color, for metadata (RAM requirements, "Not installed")

**Install hints**: When an option requires installation, show clear instructions:
```html
<div class="install-hint">Install with: <code>pip install mlx-lm</code></div>
```

### Provider Configuration Storage

Provider credentials and settings are stored in `~/.config/moltis/provider_keys.json`.
The `KeyStore` in `provider_setup.rs` manages this with:

- **Per-provider config object**: `{ "apiKey": "...", "baseUrl": "...", "model": "..." }`
- **Backward compatibility**: Migrates from old string-only format automatically
- **Partial updates**: `save_config()` preserves existing fields when updating

When adding new provider fields, update both:
1. `ProviderConfig` struct in `provider_setup.rs`
2. `available()` response to expose the field to the frontend
3. `save_key()` to accept and persist the new field

### Server-Injected Data (gon pattern)

When the frontend needs server-side data **at page load** (before any async
fetch completes), use the gon pattern instead of inline `<script>` DOM
manipulation or extra API calls:

**Rust side** — add a field to `GonData` in `server.rs` and populate it in
`build_gon_data()`. The struct is serialized and injected into `<head>` as
`<script>window.__MOLTIS__={...};</script>` on every page serve. Only put
request-independent data here (no cookies, no sessions — those still need
`/api/auth/status`).

```rust
// server.rs
#[derive(serde::Serialize)]
struct GonData {
    identity: moltis_config::ResolvedIdentity,
    // add new fields here
}
```

**JS side** — import `gon.js`:

```js
import * as gon from "./gon.js";

// Read server-injected data synchronously at module load.
var identity = gon.get("identity");

// React to changes (from set() or refresh()).
gon.onChange("identity", (id) => { /* update DOM */ });

// After a mutation (e.g. saving identity), refresh all gon data
// from the server. This re-fetches /api/gon and notifies all
// onChange listeners — no need to update specific fields manually.
gon.refresh();
```

**Do NOT**: inject inline `<script>` tags with `document.getElementById`
calls, build HTML strings in Rust, or use `body.replace` for DOM side effects.
All of those are fragile. The gon blob is the single injection point.
When data changes at runtime, call `gon.refresh()` instead of manually
updating individual fields — it keeps everything consistent.

### Event Bus (WebSocket events in JS)

Server-side broadcasts reach the UI via WebSocket frames. The JS event bus
lives in `events.js`:

```js
import { onEvent } from "./events.js";

// Subscribe to a named event. Returns an unsubscribe function.
var off = onEvent("mcp.status", (payload) => {
  // payload is the deserialized JSON from the broadcast
});

// In a Preact useEffect, return the unsubscribe for cleanup:
useEffect(() => {
  var off = onEvent("some.event", handler);
  return off;
}, []);
```

The WebSocket reader in `websocket.js` dispatches incoming event frames to
all registered listeners via `eventListeners[frame.event]`. Do **not** use
`window.addEventListener` / `CustomEvent` for server events — use this bus.

## API Namespace Convention

Each navigation tab in the UI should have its own API namespace, both for
REST endpoints (`/api/<feature>/...`) and RPC methods (`<feature>.*`). This
keeps concerns separated and makes it straightforward to gate each feature
behind a cargo feature flag (e.g. `#[cfg(feature = "skills")]`).

Examples: `/api/skills`, `/api/plugins`, `/api/channels`, with RPC methods
`skills.list`, `plugins.install`, `channels.status`, etc. Never merge
multiple features into a single endpoint.
## Authentication Architecture

The gateway supports password and passkey (WebAuthn) authentication, managed
in `crates/gateway/src/auth.rs` with routes in `auth_routes.rs` and middleware
in `auth_middleware.rs`.

Key concepts:

- **Setup code**: On first run (no password set), a random code is printed to
  the terminal. The user enters it on the `/setup` page to set a password or
  register a passkey. The code is single-use and cleared after setup.
- **Auth states**: `auth_disabled` (explicit `[auth] disabled = true` in
  config) and localhost-no-password (safe default) are distinct states.
  `auth_disabled` is a deliberate user choice; localhost-no-password is the
  initial state before setup.
- **Session cookies**: HTTP-only `moltis_session` cookie, validated by the
  auth middleware.
- **API keys**: Bearer token auth via `Authorization: Bearer <key>` header,
  managed through the settings UI.
- **Credential store**: `CredentialStore` in `auth.rs` persists passwords
  (argon2 hashed), passkeys, API keys, and session tokens to a JSON file.

The auth middleware (`RequireAuth`) protects all `/api/*` routes except
`/api/auth/*` and `/api/gon`.

## Testing

```bash
cargo test                           # Run all tests
cargo test <test_name>               # Run a specific test
cargo test <module>::               # Run all tests in a module
cargo test -- --nocapture            # Run tests with stdout visible
```

## Code Quality

```bash
cargo +nightly fmt       # Format code (uses nightly)
cargo +nightly clippy    # Run linter (uses nightly)
cargo check              # Fast compile check without producing binary
taplo fmt                # Format TOML files (Cargo.toml, etc.)
biome check --write      # Lint & format JavaScript files (installed via mise)
```

When editing `Cargo.toml` or other TOML files, run `taplo fmt` to format them
according to the project's `taplo.toml` configuration.

## Sandbox Architecture

The gateway runs user commands inside isolated containers (Docker or Apple
Container). Key files:

- `crates/tools/src/sandbox.rs` — `Sandbox` trait, `DockerSandbox`,
  `AppleContainerSandbox`, `SandboxRouter`, image build/list/clean helpers
- `crates/tools/src/exec.rs` — `ExecTool` that routes commands through the
  sandbox
- `crates/cli/src/sandbox_commands.rs` — `moltis sandbox` CLI subcommands
- `crates/config/src/schema.rs` — `SandboxConfig` with default packages list

### Pre-built images

Both backends support `build_image`: generate a Dockerfile with `FROM <base>`
+ `RUN apt-get install ...`, then run `docker build` / `container build`.
The image tag is a deterministic hash of the base image + sorted package
list (`sandbox_image_tag`). The gateway pre-builds at startup; if the image
already exists it's a no-op.

### Config-driven packages

Default packages are defined in `default_sandbox_packages()` in `schema.rs`.
On first run (no config file), a `moltis.toml` is written with all defaults
including the full packages list. Users edit that file to add/remove packages
and restart — the image tag changes automatically, triggering a rebuild.

### Shared helpers

`sandbox_image_tag`, `sandbox_image_exists`, `list_sandbox_images`,
`remove_sandbox_image`, `clean_sandbox_images` are module-level public
functions in `sandbox.rs`, parameterised by CLI binary name. The
`SandboxConfig::from(&config_schema::SandboxConfig)` impl converts the
config-crate types to tools-crate types — use it instead of manual
field-by-field conversion.

## Security

### WebSocket Origin validation (CSWSH protection)

The WebSocket upgrade handler in `server.rs` validates the `Origin` header.
Cross-origin requests are rejected with 403. Loopback variants (`localhost`,
`127.0.0.1`, `::1`) are treated as equivalent. Non-browser clients (no
Origin header) are allowed through.

This prevents the attack class from GHSA-g8p2-7wf7-98mq where a malicious
webpage could connect to the local gateway WebSocket from the victim's
browser.

### SSRF protection

`web_fetch.rs` resolves DNS and checks the resulting IP against blocked
ranges (loopback, private, link-local, CGNAT) before making HTTP requests.
Any changes to web_fetch must preserve this check.

## CLI Auth Commands

The `auth` subcommand (`crates/cli/src/auth_commands.rs`) provides:

- `moltis auth reset-password` — clear the stored password
- `moltis auth reset-identity` — clear identity and user profile (triggers
  onboarding on next load)

## CLI Sandbox Commands

The `sandbox` subcommand (`crates/cli/src/sandbox_commands.rs`) provides:

- `moltis sandbox list` — list pre-built `moltis-sandbox:*` images
- `moltis sandbox build` — build image from config (base + packages)
- `moltis sandbox remove <tag>` — remove a specific image
- `moltis sandbox clean` — remove all sandbox images

## Sensitive Data Handling

Never use plain `String` for passwords, API keys, tokens, or any secret
material. Use `secrecy::Secret<String>` instead — it redacts `Debug` output,
prevents accidental `Display`, and zeroes memory on drop.

```rust
use secrecy::{ExposeSecret, Secret};

// Store secrets wrapped
struct Config {
    api_key: Secret<String>,
}

// Construct: wrap at the boundary
let cfg = Config { api_key: Secret::new(raw_key) };

// Use: expose only at the point of consumption
req.header("Authorization", format!("Bearer {}", cfg.api_key.expose_secret()));
```

Rules:
- **Struct fields** holding secrets must be `Secret<String>` (or
  `Option<Secret<String>>`).
- **Function parameters** can stay `&str`; call `.expose_secret()` at the call
  site.
- **Serde deserialize** works automatically (secrecy's `serde` feature).
- **Serde serialize** requires a custom helper when round-tripping is needed
  (config files, token storage). See `serialize_secret` /
  `serialize_option_secret` in `crates/oauth/src/types.rs`.
- **Debug impls**: replace `#[derive(Debug)]` with a manual impl that prints
  `[REDACTED]` for secret fields.
- **RwLock guards**: when a `RwLock<Option<Secret<String>>>` read guard is
  followed by a write in the same function, scope the read guard in a block
  `{ let guard = lock.read().await; ... }` to avoid deadlocks.

## Data and Config Directories

Moltis uses two directories, **never** the current working directory:

- **Config dir** (`moltis_config::config_dir()`) — `~/.moltis/` by default.
  Contains `moltis.toml`, `credentials.json`, `mcp-servers.json`.
  Overridable via `--config-dir` or `MOLTIS_CONFIG_DIR`.
- **Data dir** (`moltis_config::data_dir()`) — `~/.moltis/` by default.
  Contains `moltis.db`, `memory.db`, `sessions/`, `logs.jsonl`,
  `MEMORY.md`, `memory/*.md`.
  Overridable via `--data-dir` or `MOLTIS_DATA_DIR`.

**Rules:**
- **Never use `std::env::current_dir()`** to resolve paths for persistent
  storage (databases, memory files, config). Always use `data_dir()` or
  `config_dir()`. Writing to cwd leaks files into the user's repo.
- When a function needs a storage path, pass `data_dir` explicitly or call
  `moltis_config::data_dir()`. Don't assume the process was started from a
  specific directory.
- The gateway's `run()` function resolves `data_dir` once at startup
  (`server.rs`) and threads it through. Prefer using that resolved value
  over calling `data_dir()` repeatedly.

## Database Migrations

Schema changes are managed via **sqlx migrations**. Each crate owns its migrations
in its own `migrations/` directory. See [docs/sqlite-migration.md](docs/sqlite-migration.md)
for full documentation.

**Architecture:**

- Each crate has its own `migrations/` directory and `run_migrations()` function
- Gateway orchestrates migrations at startup in dependency order
- Timestamp-based versioning (`YYYYMMDDHHMMSS_description.sql`) for global uniqueness

**Crate ownership:**

| Crate | Tables |
|-------|--------|
| `moltis-projects` | `projects` |
| `moltis-sessions` | `sessions`, `channel_sessions` |
| `moltis-cron` | `cron_jobs`, `cron_runs` |
| `moltis-gateway` | `auth_*`, `passkeys`, `api_keys`, `env_variables`, `message_log`, `channels` |
| `moltis-memory` | `files`, `chunks`, `embedding_cache`, `chunks_fts` |

**Adding a migration to an existing crate:**

1. Create `crates/<crate>/migrations/YYYYMMDDHHMMSS_description.sql`
2. Write the SQL (use `IF NOT EXISTS` for new tables)
3. Rebuild (`cargo build`) to embed the migration

**Adding a new crate with migrations:**

1. Create `crates/new-crate/migrations/` directory
2. Add `run_migrations()` to `lib.rs`
3. Call it from `server.rs` in dependency order

## Provider Implementation Guidelines

### Async all the way down

Never use `block_on`, `std::thread::scope` + `rt.block_on`, or any blocking
call inside an async context (tokio runtime). This causes a panic:
"Cannot start a runtime from within a runtime". All token exchanges,
HTTP calls, and I/O in provider methods (`complete`, `stream`) must be `async`
and use `.await`. If a helper needs to make HTTP requests, make it `async fn`.

### Model lists for providers

When adding a new LLM provider, make the model list as complete as possible.
Models vary by plan/org and can change, so keep the list intentionally broad —
if a model isn't available the provider API will return an error and the user
can remove it from their config.

To find the correct model IDs:
- Check the upstream open-source implementations in `../clawdbot/` (TypeScript
  reference), as well as projects like OpenAI Codex CLI, Claude Code, opencode,
  etc.
- For "bring your own model" providers (OpenRouter, Venice, Ollama), don't
  hardcode a model list — require the user to specify a model via config.
- Ideally, query the provider's `/models` endpoint at registration time to
  build the list dynamically (not yet implemented).

## Plans and Session History

Plans are stored in `prompts/` (configured via `.claude/settings.json`).
When entering plan mode, plans are automatically saved there. After completing
a significant piece of work, write a brief session summary to
`prompts/session-YYYY-MM-DD-<topic>.md` capturing what was done, key decisions,
and any open items.

## Git Workflow

Follow conventional commit format: `feat|fix|refactor|docs|test|chore(scope): description`

When adding a new feature (`feat` commits), update the features list in
`README.md` as part of the same branch/PR.

**You MUST run all checks before every commit and fix any issues they report:**
1. `cargo +nightly fmt --all` — format all Rust code (CI runs `cargo fmt --all -- --check`)
2. `cargo +nightly clippy --all-targets --all-features -- -D warnings` — run linter (must pass with zero warnings)
3. `cargo test --all-features` — run all tests
4. `biome check --write` (when JS files were modified; CI runs `biome ci`)
5. `taplo fmt` (when TOML files were modified)
