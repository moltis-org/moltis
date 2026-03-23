# Skill Self-Extension

Moltis can create, update, and delete personal skills at runtime through agent
tools, enabling the system to extend its own capabilities during a conversation.

## Overview

Three agent tools manage personal skills by default:

| Tool | Description |
|------|-------------|
| `create_skill` | Write a new `SKILL.md` to `<data_dir>/skills/<name>/` |
| `update_skill` | Overwrite an existing skill's `SKILL.md` |
| `delete_skill` | Remove a skill directory |

When `skills.enable_agent_sidecar_files = true`, a fourth tool becomes
available:

| Tool | Description |
|------|-------------|
| `write_skill_files` | Write supplementary UTF-8 text files inside an existing personal skill directory |

Skills created this way are personal and stored in the configured data
directory's `skills/` folder. They become available on the next message
automatically thanks to the skill watcher.

## Skill Watcher

The skill watcher (`crates/skills/src/watcher.rs`) monitors skill directories
for filesystem changes using debounced notifications. When a `SKILL.md` file is
created, modified, or deleted, the watcher emits a `skills.changed` event via
the WebSocket event bus so the UI can refresh. Supplementary file writes do not
change discovery on their own, so the watcher intentionally stays focused on
`SKILL.md`.

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

This writes `<data_dir>/skills/summarize-pr/SKILL.md` with the provided content.
The skill discoverer picks it up on the next message.

## Writing Supplementary Files

When `skills.enable_agent_sidecar_files = true`, the agent can add sidecar
files such as shell scripts, templates, `_meta.json`, or `Dockerfile`:

```json
{
  "name": "summarize-pr",
  "files": [
    {
      "path": "script.sh",
      "content": "#!/usr/bin/env bash\necho summarize\n"
    },
    {
      "path": "templates/prompt.txt",
      "content": "Summarize the pull request with risks first.\n"
    }
  ]
}
```

Safety rules:

- only writes inside `<data_dir>/skills/<name>/`
- only relative UTF-8 text files
- rejects `..`, absolute paths, hidden path components, and `SKILL.md`
- rejects symlink escapes and oversized batches
- appends an audit entry to `~/.moltis/logs/security-audit.jsonl`

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

This removes the entire `<data_dir>/skills/summarize-pr/` directory, including
any supplementary files written alongside `SKILL.md`.

```admonish warning
Deleted skills cannot be recovered. The agent should confirm with the user
before deleting a skill.
```
