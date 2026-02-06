# Skill Self-Extension

Moltis can create, update, and delete skills at runtime through agent tools,
enabling the system to extend its own capabilities during a conversation.

## Overview

Three agent tools manage project-local skills:

| Tool | Description |
|------|-------------|
| `create_skill` | Write a new `SKILL.md` to `.moltis/skills/<name>/` |
| `update_skill` | Overwrite an existing skill's `SKILL.md` |
| `delete_skill` | Remove a skill directory |

Skills created this way are project-local and stored in the working directory's
`.moltis/skills/` folder. They become available on the next message
automatically thanks to the skill watcher.

## Skill Watcher

The skill watcher (`crates/skills/src/watcher.rs`) monitors skill directories
for filesystem changes using debounced notifications. When a `SKILL.md` file is
created, modified, or deleted, the watcher emits a `skills.changed` event via
the WebSocket event bus so the UI can refresh.

```admonish tip
The watcher uses debouncing to avoid firing multiple events for rapid
successive edits (e.g. an editor writing a temp file then renaming).
```

## Creating a Skill

The agent can create a skill by calling the `create_skill` tool:

```json
{
  "name": "summarize-pr",
  "content": "# summarize-pr\n\nSummarize a GitHub pull request...",
  "description": "Summarize GitHub PRs with key changes and review notes"
}
```

This writes `.moltis/skills/summarize-pr/SKILL.md` with the provided content.
The skill discoverer picks it up on the next message.

## Updating a Skill

```json
{
  "name": "summarize-pr",
  "content": "# summarize-pr\n\nUpdated instructions..."
}
```

## Deleting a Skill

```json
{
  "name": "summarize-pr"
}
```

This removes the entire `.moltis/skills/summarize-pr/` directory.

```admonish warning
Deleted skills cannot be recovered. The agent should confirm with the user
before deleting a skill.
```
