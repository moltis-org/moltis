# Checkpoints

Moltis automatically snapshots files before the agent modifies them. If the
agent breaks something, use [`/rollback`](commands.md#rollback) to restore files
to their pre-turn state.

## Automatic Per-Turn Checkpointing

An `AutoCheckpointHook` runs before every file-mutating tool call (`Write`,
`Edit`, `MultiEdit`) and snapshots the target file. Checkpoints are grouped by
**turn** (one user message = one turn), so `/rollback 1` undoes all file changes
from that turn at once.

The hook also fires on skill and memory mutations:

- `create_skill`, `update_skill`, `delete_skill`, `write_skill_files`
- `memory_save`, `memory_forget`, `memory_delete`
- the silent pre-compaction memory flush

Each mutation creates a manifest-backed snapshot in `~/.moltis/checkpoints/`
before the write or delete happens.

## /rollback Command

Available in the web UI, all channels (Telegram, Discord, Slack, etc.), and CLI.

```
/rollback           # list recent turns with file changes
/rollback 1         # restore all files from turn 1
/rollback diff 1    # preview which files were changed in turn 1
```

Turns are session-scoped — you only see checkpoints from your current session.

## Cleanup

When checkpoint count exceeds 500, the oldest 20% are automatically pruned.
Cleanup runs lazily on each new checkpoint creation.

## Tool Surface

### `checkpoints_list`

List recent automatic checkpoints.

```json
{
  "limit": 20,
  "path_contains": "skills/my-skill"
}
```

### `checkpoint_restore`

Restore a checkpoint by ID.

```json
{
  "id": "3c7c6f2f8b7c4d8c8b8cdb91d9161f59"
}
```

## Mutation Results

Checkpointed tools return a `checkpointId` field in their result payload. That
gives agents and users a direct restore handle without first listing every
checkpoint.

## Behavior

- If the target existed, Moltis snapshots the file or directory first.
- If the target did not exist yet, restore removes the later-created path.
- Restore replaces the current target state with the checkpoint snapshot.
- Checkpoints are internal safety artifacts, they do not touch the user’s git
  history.
