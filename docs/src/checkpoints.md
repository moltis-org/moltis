# Checkpoints

Moltis now creates automatic checkpoints before built-in file mutations that
change personal skills or agent memory files.

## What Gets Checkpointed

Current built-in checkpoint coverage includes:

- `create_skill`
- `update_skill`
- `delete_skill`
- `write_skill_files`
- `memory_save`
- `memory_forget`
- `memory_delete`
- the silent pre-compaction memory flush

Each mutation creates a manifest-backed snapshot in `~/.moltis/checkpoints/`
before the write or delete happens.

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
