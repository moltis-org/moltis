# Skill Self-Extension

Moltis can create, update, and delete personal skills at runtime through agent
tools, enabling the system to extend its own capabilities during a conversation.

## Overview

Four agent tools manage personal skills:

| Tool | Description |
|------|-------------|
| `create_skill` | Write a new `SKILL.md` to `<data_dir>/skills/<name>/` |
| `update_skill` | Overwrite an existing skill's `SKILL.md` |
| `delete_skill` | Remove a skill directory |
| `read_skill` | Load a skill's content or access its linked sidecar files |

A fifth tool is available when `skills.enable_agent_sidecar_files = true`:

| Tool | Description |
|------|-------------|
| `write_skill_files` | Write supplementary UTF-8 text files inside an existing personal skill directory |

All mutation tools (`create_skill`, `update_skill`, `delete_skill`,
`write_skill_files`) create an automatic checkpoint before executing.
Tool results include a `checkpointId` for restoration via `checkpoint_restore`.

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

## Reading a Skill

The `read_skill` tool loads a skill's full content. The primary call (with
just `name`) returns the SKILL.md body plus:

- Frontmatter metadata (`description`, `display_name`, `license`, `homepage`,
  `compatibility`, `allowed_tools`, `source`)
- A `linked_files` list of sidecar files under `references/`, `templates/`,
  `assets/`, and `scripts/`
- The skill body is scanned for prompt-injection patterns (warn-only)

Call again with `file_path` to read a specific sidecar file. Nested paths
are supported (e.g. `references/subdir/deep.md`). Binary files return
`{ is_binary: true, bytes }`.

Size limits apply: individual skill bodies are capped at 256 KiB, sidecar
files at 128 KiB each. The sidecar listing is capped at 32 files total
(8 per subdirectory).

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

Deleted skills can be restored from the returned `checkpointId` with
`checkpoint_restore`, as long as the checkpoint still exists.
