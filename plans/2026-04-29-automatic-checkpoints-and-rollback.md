# Automatic Checkpoints and /rollback

Date: 2026-04-29

## Problem

The agent modifies files (code, configs, moltis.toml) and sometimes breaks things.
Today there's no user-facing way to undo what the agent did. The `CheckpointManager`
and `checkpoint_restore` tool exist but they're agent-initiated — the user has to
ask the agent to checkpoint, and then ask it to restore. That defeats the purpose
when the agent is the one that broke things.

Hermes solves this with automatic per-turn checkpointing and a `/rollback` command.

## Goals

1. Automatic snapshots before every file-mutating tool call — no user action needed
2. `/rollback` command available on all channels (Telegram, Discord, Slack, web, etc.)
3. Covers moltis.toml changes so users can revert broken config
4. Automatic cleanup of old checkpoints to avoid unbounded disk growth
5. Minimal overhead — don't snapshot unchanged files, don't slow down tool execution

## Design

### Turn-based checkpointing

Group checkpoints by **turn** (one user message = one turn). A turn may trigger
many tool calls that each modify files, but the user thinks in terms of "undo
what you just did" — not individual tool calls.

```
Turn 7:  user says "refactor the auth module"
  → write_file: src/auth.rs        (checkpoint before)
  → edit: src/main.rs              (checkpoint before)
  → write_file: src/auth/mod.rs    (checkpoint before)
  → edit: moltis.toml              (checkpoint before)

/rollback 7  →  restores all 4 files to their pre-turn-7 state
```

### What gets checkpointed

Any tool that modifies the filesystem:
- `write` / `edit` / `multi_edit` — already have `with_checkpoint_manager()` builder
- `create_skill` / `update_skill` / `delete_skill` — already checkpoint manually
- `exec` / `shell` — **cannot reliably detect** which files a shell command modifies

For `exec`, we can't know what `rm -rf` or `sed -i` will touch. Two options:
- **Option A**: Don't checkpoint exec. Accept the gap. `/rollback` covers agent-native
  file ops but not shell commands. Document this clearly.
- **Option B**: Snapshot the entire working directory before exec. Too expensive for
  large repos.

**Recommendation**: Option A. The exec tool already goes through approval gates.
Shell commands are the user's responsibility. `/rollback` covers the common case
(agent writes/edits files via its native tools).

### Storage format

Reuse the existing `CheckpointManager` storage:

```
~/.moltis/checkpoints/
  {checkpoint-id}/
    manifest.json        # CheckpointRecord (id, created_at, reason, source_path, existed, backup_path)
    snapshot/
      {file-or-dir}      # copy of original content
```

Add a **turn index** file that groups checkpoint IDs by turn:

```
~/.moltis/checkpoints/turns.jsonl
```

Each line:
```json
{"turn_id": "run-abc123", "session_key": "main", "created_at": 1714400000, "checkpoint_ids": ["cp-1", "cp-2", "cp-3"]}
```

### Implementation: automatic checkpointing via HookHandler

The hook system already fires `BeforeToolCall` with `{ tool_name, arguments }` before
every tool execution. Register a `CheckpointHookHandler` that:

1. Subscribes to `HookEvent::BeforeToolCall`
2. Checks if the tool is file-mutating (`write`, `edit`, `multi_edit`)
3. Extracts the target file path from the tool arguments
4. Calls `CheckpointManager::checkpoint_path()` to snapshot the file
5. Records the checkpoint ID in the current turn's group

```rust
pub struct AutoCheckpointHook {
    manager: Arc<CheckpointManager>,
    /// Current turn's checkpoint IDs, keyed by session.
    active_turns: Arc<RwLock<HashMap<String, TurnCheckpoints>>>,
}

struct TurnCheckpoints {
    run_id: String,
    checkpoint_ids: Vec<String>,
    created_at: i64,
}
```

**Registration**: In gateway startup (`prepare_core/post_state.rs`), after creating
the `CheckpointManager`, register `AutoCheckpointHook` with the `HookRegistry`.

**Turn boundary detection**: A new turn starts when `run_id` changes (each
`chat.send` generates a unique `run_id`). The `run_id` is available in the
tool context (`_run_id` field) passed to tool arguments.

### Protecting moltis.toml

The config file lives at `~/.config/moltis/moltis.toml` (or `MOLTIS_CONFIG_DIR`).
Two scenarios where it gets modified:

1. **Agent edits it** via `write` or `edit` tool — caught by the auto-checkpoint hook
2. **Web UI settings pages** write it via the config save RPC — need to add a
   checkpoint call in the config save path (`crates/gateway/src/methods/services/core.rs`
   or wherever `config.save()` is called)

For case 2, add a direct `CheckpointManager::checkpoint_path()` call before
any config file write. This doesn't need the hook system — it's a one-line addition
at the save callsite.

### /rollback command

Register `/rollback` in the channel command registry. Subcommands:

```
/rollback              — list recent turns with file changes (last 10)
/rollback <N>          — restore all files from turn N to their pre-turn state
/rollback diff <N>     — show which files changed in turn N (paths + sizes)
/rollback <N> <file>   — restore a single file from turn N
```

**Handler** (`session_handlers.rs` or new `rollback_handler.rs`):

- `/rollback` (no args): Read `turns.jsonl`, show last 10 entries with turn number,
  timestamp, file count, session key. Format like:
  ```
  Recent turns with file changes:
  1. 2 min ago — 3 files (src/auth.rs, src/main.rs, moltis.toml)
  2. 15 min ago — 1 file (src/lib.rs)
  3. 1 hour ago — 5 files (...)
  ```

- `/rollback <N>`: Load turn N's checkpoint IDs, call `CheckpointManager::restore()`
  for each. Report what was restored. Broadcast a session event so the web UI
  can refresh.

- `/rollback diff <N>`: List files from turn N's checkpoints without restoring.
  Show original size vs current size, or "file did not exist" vs "now exists".

### Automatic cleanup

Old checkpoints must be pruned. Two strategies, both configurable:

```toml
[tools.checkpoints]
enabled = true                    # default: true
max_age_days = 7                  # delete checkpoints older than this
max_total_size_mb = 500           # delete oldest when total exceeds this
cleanup_interval_hours = 6        # how often to run cleanup
```

**Cleanup logic** (runs on a timer in the gateway, or lazily on checkpoint creation):

1. Scan `~/.moltis/checkpoints/` for all checkpoint dirs
2. Read each `manifest.json` for `created_at`
3. Delete checkpoints older than `max_age_days`
4. If total size still exceeds `max_total_size_mb`, delete oldest first until under budget
5. Remove corresponding entries from `turns.jsonl`
6. Log cleanup stats at `info!` level

**Lazy cleanup** (simpler, recommended for v1): Run cleanup at the start of each
checkpoint creation. If the total checkpoint count exceeds a threshold (e.g., 1000),
prune the oldest 20%. This avoids needing a background timer.

### Files to modify

| File | Change |
|------|--------|
| `crates/tools/src/checkpoints.rs` | Add `TurnIndex` (append/read/prune turns.jsonl), add `cleanup()` method |
| `crates/common/src/hooks.rs` | No changes needed — hook system already supports this |
| `crates/tools/src/lib.rs` | New `AutoCheckpointHook` struct implementing `HookHandler` |
| `crates/channels/src/commands.rs` | Register `/rollback` command |
| `crates/gateway/src/channel_events/commands/dispatch.rs` | Add dispatch arm |
| `crates/gateway/src/channel_events/commands/session_handlers.rs` | Add `handle_rollback()` |
| `crates/gateway/src/server/prepare_core/post_state.rs` | Register `AutoCheckpointHook` with `HookRegistry` |
| `crates/config/src/schema/tools.rs` | Add `CheckpointsConfig` (enabled, max_age_days, max_total_size_mb) |
| `crates/config/src/validate/schema_map.rs` | Register `[tools.checkpoints]` |
| `crates/config/src/template.rs` | Document `[tools.checkpoints]` |
| Config save callsite | Add checkpoint before moltis.toml writes |

### What this does NOT cover

- **Shell command rollback** (`exec` tool): Can't reliably detect which files
  a shell command modifies. Users who run destructive shell commands through
  the agent should use git.
- **Database rollback**: SQLite changes (sessions, memory) are not checkpointed.
  This is file-level only.
- **Undo chat history**: Hermes also undoes the chat turn on rollback. We could
  add this (truncate session JSONL to pre-turn state) but it's a separate concern.
  Start without it — file rollback is the high-value feature.

### Implementation order

1. **Turn index** — `TurnIndex` struct in `checkpoints.rs` (append, read, prune)
2. **Auto-checkpoint hook** — `AutoCheckpointHook` implementing `HookHandler`
3. **Hook registration** — wire into gateway startup
4. **Cleanup** — lazy cleanup on checkpoint creation
5. **`/rollback` command** — register + handler + all subcommands
6. **Config protection** — checkpoint before moltis.toml saves
7. **Config fields** — `[tools.checkpoints]` section

Estimated scope: ~500-800 lines of new code across 10 files.
