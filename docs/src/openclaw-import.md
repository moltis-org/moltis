# OpenClaw Import

Moltis can automatically detect and import data from an existing [OpenClaw](https://docs.openclaw.ai) installation. This lets you migrate to Moltis without losing your provider keys, memory files, skills, sessions, personality, or channel configuration.

> **Your OpenClaw installation is never modified.** The import is strictly read-only — Moltis copies data into its own directory and does not write to, move, or delete anything under `~/.openclaw/`. You can safely keep using OpenClaw alongside Moltis, and re-import at any time to pick up new data.

## How Detection Works

Moltis checks for an OpenClaw installation in two locations:

1. The path set in the `OPENCLAW_HOME` environment variable
2. `~/.openclaw/` (default)

If the directory exists and contains recognizable OpenClaw files (`openclaw.json`, agent directories, etc.), Moltis considers it detected. The workspace directory respects the `OPENCLAW_PROFILE` environment variable for multi-profile setups.

## What Gets Imported

| Category | Source | Destination | Notes |
|----------|--------|-------------|-------|
| **Identity** | `openclaw.json` agent name, theme, and timezone | `moltis.toml` identity section | Preserves existing Moltis identity if already configured |
| **Providers** | Agent auth-profiles (API keys) | `~/.moltis/provider_keys.json` | Maps OpenClaw provider names to Moltis equivalents (e.g., `google` becomes `gemini`) |
| **Skills** | `skills/` directories with `SKILL.md` | `~/.moltis/skills/` | Copies entire skill directories; skips duplicates |
| **Memory** | `MEMORY.md` and all `memory/*.md` files | `~/.moltis/MEMORY.md` and `~/.moltis/memory/` | Imports daily logs, project notes, and all other markdown memory files. Appends with `<!-- Imported from OpenClaw -->` separator for idempotency |
| **Channels** | Telegram and Discord bot configuration in `openclaw.json` | `moltis.toml` channels section | Supports both flat and multi-account Telegram configs |
| **Sessions** | JSONL conversation files under `agents/*/sessions/` | `~/.moltis/sessions/` and `~/.moltis/memory/sessions/` | Converts OpenClaw message format to Moltis format; prefixes keys with `oc:`. Also generates markdown transcripts for memory search indexing |
| **MCP Servers** | `mcp-servers.json` | `~/.moltis/mcp-servers.json` | Merges with existing servers; skips duplicates by name |
| **Workspace Files** | `SOUL.md`, `IDENTITY.md`, `USER.md`, `TOOLS.md`, `AGENTS.md`, `HEARTBEAT.md`, `BOOT.md` | `~/.moltis/` (root) or `~/.moltis/agents/<id>/` | Copies raw workspace files; skips if destination already has user content. Replaces auto-seeded defaults |

### Workspace files explained

These markdown files shape your agent's personality and behavior. Moltis uses them in the same way OpenClaw does:

- **`SOUL.md`** — personality directives (tone, style, boundaries)
- **`IDENTITY.md`** — agent name, emoji, creature/vibe theme
- **`USER.md`** — user profile (name, preferences, context the agent should know about you)
- **`TOOLS.md`** — tool usage guidelines and constraints
- **`AGENTS.md`** — global workspace rules injected into every conversation
- **`HEARTBEAT.md`** — periodic heartbeat prompt (what to check on each scheduled tick)
- **`BOOT.md`** — startup context injected when the gateway starts

If you customized any of these files in OpenClaw, they will carry over. If the destination already has user content, the import skips the file to avoid overwriting your work. Auto-seeded defaults (like the template `SOUL.md`) are replaced with your imported content.

### Multi-agent support

If your OpenClaw installation has multiple agents (defined in `openclaw.json`'s `agents.list` or detected from `agents/` directories), all of them are imported:

- The **default agent** becomes Moltis's `main` agent
- **Non-default agents** are created as separate agent personas with their name, theme, and emoji
- **Per-agent workspace files** (`SOUL.md`, `IDENTITY.md`, etc.) are copied to `~/.moltis/agents/<id>/`, giving each agent its own personality
- **Per-agent sessions** are prefixed with `oc:<agent_id>:` so they appear under the correct agent
- Agents without per-agent workspace files inherit from the root files automatically

## Importing via Web UI

### During Onboarding

If Moltis detects an OpenClaw installation at first launch, an **Import** step appears in the onboarding wizard before the identity and provider steps. You can select which categories to import using checkboxes, then proceed with the rest of setup.

### From Settings

1. Go to **Settings** (gear icon)
2. Select **OpenClaw Import** from the sidebar
3. Click **Scan** to see what data is available
4. Check the categories you want to import
5. Click **Import Selected**

The import section only appears when an OpenClaw installation is detected.

## Importing via CLI

The `moltis import` command provides three subcommands:

### Detect

Check whether an OpenClaw installation exists and preview what can be imported:

```bash
moltis import detect
```

Example output:

```
OpenClaw installation detected at /Users/you/.openclaw

  Identity:        available (agent: "friday")
  Providers:       available (2 auth profiles)
  Skills:          3 skills found
  Memory:          available (MEMORY.md + 12 memory files)
  Channels:        available (1 Telegram account)
  Sessions:        47 session files across 2 agents
  MCP Servers:     4 servers configured
  Workspace Files: SOUL.md, IDENTITY.md, USER.md, TOOLS.md, HEARTBEAT.md
```

Use `--json` for machine-readable output:

```bash
moltis import detect --json
```

### Import All

Import everything at once:

```bash
moltis import all
```

Preview what would happen without writing anything:

```bash
moltis import all --dry-run
```

### Import Selected Categories

Import only specific categories:

```bash
moltis import select -c providers,skills,memory
```

Valid category names: `identity`, `providers`, `skills`, `memory`, `channels`, `sessions`, `mcp_servers`, `workspace-files`.

Combine with `--dry-run` to preview:

```bash
moltis import select -c sessions --dry-run
```

## Importing via RPC

Three RPC methods are available for programmatic access:

| Method | Description |
|--------|-------------|
| `openclaw.detect` | Returns detection and scan results (what data is available) |
| `openclaw.scan` | Alias for `openclaw.detect` |
| `openclaw.import` | Performs the import with a selection object |

Example `openclaw.import` params:

```json
{
  "identity": true,
  "providers": true,
  "skills": true,
  "memory": true,
  "channels": false,
  "sessions": false,
  "mcp_servers": true,
  "workspace_files": true
}
```

The response includes a report with per-category status (`imported`, `skipped`, `error`) and counts.

## Incremental Session Import

If you continue using OpenClaw after the initial import, Moltis will detect new messages when you re-import. Sessions are compared by source file line count — if the source JSONL has grown since the last import, Moltis re-converts the full session and updates the metadata.

On incremental update:
- The session's original `id` and `created_at` are preserved
- The `version` field is bumped
- The markdown transcript is regenerated with all messages
- The CLI report shows updated sessions separately: `2 imported, 1 updated, 3 skipped`

Legacy metadata (from imports before incremental support) will trigger a one-time catch-up re-import to establish the baseline line count.

## Automatic Background Syncing

When the `file-watcher` feature is enabled (default), Moltis automatically watches the OpenClaw sessions directory for changes. Any new or appended session files are synced incrementally within seconds, without requiring a manual re-import.

**How it works:**

- Moltis uses OS-level file notifications (FSEvents on macOS, inotify on Linux) to detect `.jsonl` file changes in the OpenClaw sessions directory
- Events are debounced with a 5-second window to batch rapid writes during active conversations
- A 60-second periodic fallback ensures changes are caught even if file notifications are missed
- Only sessions are synced automatically — provider keys, memory, skills, and other categories are handled by the manual import or their own dedicated watchers

**What gets synced:**

- New session files are imported with `oc:` prefixed keys
- Existing sessions that have grown (new messages appended) are re-converted and updated
- Markdown transcripts are regenerated for updated sessions so they remain searchable
- Session metadata (`id`, `created_at`) is preserved across updates

The watcher starts automatically at gateway startup when an OpenClaw installation is detected. You can see the status in the startup logs:

```
openclaw: session watcher started
```

To disable automatic syncing, compile without the `file-watcher` feature.

## Idempotency

Running the import multiple times is safe:

- **Memory** uses an `<!-- Imported from OpenClaw -->` marker to avoid duplicating `MEMORY.md` content. Individual memory files skip if they already exist at the destination
- **Skills** skip directories that already exist in the Moltis skills folder
- **MCP servers** skip entries with matching names
- **Sessions** use `oc:` prefixed keys that won't collide with native Moltis sessions. Unchanged sessions (same line count) are skipped; grown sessions are re-converted
- **Provider keys** merge with existing keys without overwriting
- **Workspace files** skip if the destination already has user content; replace only auto-seeded defaults

## Provider Name Mapping

OpenClaw and Moltis use different names for some providers:

| OpenClaw Name | Moltis Name |
|---------------|-------------|
| `google` | `gemini` |
| `anthropic` | `anthropic` |
| `openai` | `openai` |
| `openrouter` | `openrouter` |

Unmapped provider names are passed through as-is.

## Unsupported Channels

Currently only Telegram and Discord channels are imported. If your OpenClaw configuration includes other channel types (Slack, WhatsApp, etc.), they will appear as warnings in the scan output but will not be imported.

## Troubleshooting

### Import not detected

- Verify the OpenClaw directory exists: `ls ~/.openclaw/`
- If using a custom path, set `OPENCLAW_HOME=/path/to/openclaw`
- If using profiles, set `OPENCLAW_PROFILE=your-profile`

### Provider keys not working after import

OpenClaw stores API keys in agent auth-profiles. If the key was rotated or expired in OpenClaw, the imported key will also be invalid. Re-enter the key in **Settings** > **Providers**.

### Memory import appears incomplete

The import brings over `MEMORY.md` and all `.md` files from the `memory/` directory (daily logs, project notes, custom files). Non-markdown files are skipped. OpenClaw's SQLite vector database is not imported because embeddings are not portable across models — Moltis will re-index the imported files automatically.

### Session transcripts

When sessions are imported, Moltis also generates markdown transcripts in `~/.moltis/memory/sessions/`. These contain the user/assistant conversation text and are indexed by the memory system, making your imported OpenClaw conversations searchable.

### Workspace files not appearing

If a workspace file wasn't imported, it may already exist at the destination with custom content. The import never overwrites user-customized files. Check `~/.moltis/SOUL.md` (or `~/.moltis/agents/<id>/SOUL.md` for non-default agents) to see what's there. You can delete it and re-import to get the OpenClaw version.
