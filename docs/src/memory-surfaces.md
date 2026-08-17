# Memory Surfaces

Moltis has several different places where information can persist. They solve
different problems, and confusing them leads to very weird debugging sessions.

## Quick Table

| Surface | Purpose | Lifetime | Scope | Backing store |
|---------|---------|----------|-------|---------------|
| `session_state` | Short-term structured state for a session | Until the session is deleted | One session key | SQLite `session_state` table |
| Managed user profile (`USER.md`) | User name, timezone, and optional location hints | Persistent | Whole workspace | `USER.md` in `data_dir()` plus `[user]` in `moltis.toml` |
| Prompt memory (`MEMORY.md`) | High-signal facts injected into the system prompt | Persistent | Agent workspace | `MEMORY.md` in `data_dir()` or `agents/<id>/MEMORY.md` |
| Searchable memory (`memory_search`) | Long-term recall without spending prompt tokens | Persistent | Agent workspace | `MEMORY.md`, `memory/*.md`, optional exported session files |
| Managed Files | User-managed documents available to permitted agents | Persistent | Shared across all agent sessions | `<data_dir>/files` on host; `/home/sandbox/files` in supported local containers |
| Session attachments | Files attached to individual chat messages | Session/media retention | A session and message | Session media storage |
| Sandbox workspace mount | Lets sandboxed commands see Moltis files | Only while mounted | Depends on sandbox session/container | Mounted `data_dir()` path |
| Sandbox home (`/home/sandbox`) | Command-side scratch files and tool caches | Depends on `home_persistence` | Shared, per-session, or none | Sandbox home volume/dir |

## Short-Term Memory

`session_state` is Moltis's short-term memory layer. It is:

- keyed by `(session_key, namespace, key)`
- isolated between sessions
- available to tools and runtime features that need structured, mutable state
- not automatically injected into the prompt

This is the right place for session-scoped control data, counters, cursors, or
snapshots that should not become part of long-term memory.

## Long-Term Memory

Long-term memory lives in Markdown files inside the Moltis data directory.

- Main agent prompt memory:
  `~/.moltis/MEMORY.md`
- Main agent searchable memory:
  `~/.moltis/memory/*.md`
- Non-main agent prompt memory:
  `~/.moltis/agents/<agent_id>/MEMORY.md`
- Non-main agent searchable memory:
  `~/.moltis/agents/<agent_id>/memory/*.md`

`USER.md` is not part of agent long-term memory. It is a managed user profile
surface used for things like name, timezone, and cached location. Moltis also
stores the canonical user profile in `moltis.toml [user]`, then overlays
`USER.md` when that file exists.

For `main`, Moltis currently prefers `agents/main/MEMORY.md` if it exists and
falls back to the root `MEMORY.md`. Non-main agents do not fall back to the
root file.

## Prompt Memory Loading

`MEMORY.md` can reach the prompt in two modes:

```toml
[memory]
style = "hybrid"

[chat]
prompt_memory_mode = "live-reload"
```

- `hybrid` injects `MEMORY.md` into the prompt and keeps `memory_search`,
  `memory_get`, `memory_save`, `memory_forget`, and `memory_delete` available
- `prompt-only` injects `MEMORY.md` into the prompt, but hides memory tools
- `search-only` skips prompt injection and relies on memory tools for recall
- `off` disables both prompt memory injection and memory tools
- `live-reload` reads `MEMORY.md` again before each turn
- `frozen-at-session-start` captures the first `MEMORY.md` snapshot seen by a
  session and reuses it for later turns in that same session

The style and the mode control different things. Style decides whether prompt
memory exists at all, and whether memory tools are exposed. Mode only matters
when prompt memory is enabled. Frozen snapshots are stored in `session_state`,
so they are session-scoped rather than a process-global cache.

Defaults today:

- `memory.style = "hybrid"`
- `memory.agent_write_mode = "hybrid"`
- `memory.user_profile_write_mode = "explicit-and-auto"`
- `memory.backend = "builtin"`
- `memory.session_export = "on-new-or-reset"`
- `chat.prompt_memory_mode = "live-reload"`

`memory.agent_write_mode` is a third axis:

- `hybrid` allows agent-authored writes to both `MEMORY.md` and `memory/*.md`
- `prompt-only` restricts agent-authored writes to `MEMORY.md`
- `search-only` restricts agent-authored writes to `memory/*.md`
- `off` disables agent-authored memory mutations, including `memory_save`,
  `memory_forget`, `memory_delete`, and the silent pre-compaction memory flush

`memory.session_export` is separate again:

- `on-new-or-reset` exports session transcripts into `memory/sessions/*.md`
- `off` disables that export hook entirely

`memory.user_profile_write_mode` is a fourth axis for the managed `USER.md`
surface:

- `explicit-and-auto` allows settings saves and silent timezone/location capture
- `explicit-only` allows settings saves, but disables silent timezone/location capture
- `off` stops Moltis from writing `USER.md`; user profile data stays in
  `moltis.toml [user]`

## Managed Files and Attachments

[Managed Files](managed-files.md) are not memory. Moltis does not automatically
ingest, index, embed, summarize, search, or inject files from
`<data_dir>/files`. The runtime prompt advertises the Files path and effective
sandbox access so an agent can deliberately list or read a requested file, but
the file's contents enter context only after a permitted tool reads them.

Managed Files are also separate from session attachments. Attachments belong
to a chat message and session media; uploading one does not copy it into the
managed tree. A Settings > Files upload is shared across all agent sessions
whose tool and mount policy permits access, including channel-originated
sessions.

Portable data exports exclude Managed Files by default and include them only
with an explicit opt-in. Session media is a separate export choice.

`memory.citations` and `memory.search_merge_strategy` are typed retrieval
knobs:

- `citations`: `auto`, `on`, or `off`
- `search_merge_strategy`: `rrf` or `linear`

Two easy-to-miss interaction rules:

- builtin embedding knobs such as `memory.provider`, `memory.base_url`,
  `memory.model`, and `memory.api_key` do nothing while
  `memory.backend = "qmd"`
- `memory.session_export` affects searchable transcript files under
  `memory/sessions/*.md`, not prompt memory injection

The chat UI exposes the active prompt-memory mode in the toolbar and full
context view. Frozen sessions can also refresh their snapshot manually without
restarting the session.

## Sandboxes

Sandboxes have three separate persistence surfaces:

- Workspace mount, this is how commands can read or write Moltis memory files
  when `workspace_mount` is not `none`
- Sandbox home, this is `/home/sandbox` and is controlled by
  `tools.exec.sandbox.home_persistence`
- Managed Files, mounted independently at `/home/sandbox/files` in supported
  local containers and controlled by `managed_files_mount`

Those are not the same thing. In particular, the Files mount remains separate
even though its container path is below `/home/sandbox`.

If a file exists only in `/home/sandbox`, `memory_search` will not index it.
If a file exists in the mounted Moltis workspace, it is part of Moltis's
normal memory surface, regardless of whether the command that wrote it ran in a
sandbox or not.

A file in `/home/sandbox/files` is likewise not indexed unless it is explicitly
copied or saved into a configured memory surface.

With the default `workspace_mount = "ro"`, sandboxed commands may still read
mounted files such as `MEMORY.md`, but they cannot modify them directly.
Durable long-term memory mutations should still happen through Moltis memory
tools such as `memory_save`, `memory_forget`, and `memory_delete`, not via
shell redirection inside the sandbox.

## Between Sandboxes

Sandbox-to-sandbox sharing depends on `home_persistence`:

- `off`: nothing in sandbox home persists
- `session`: sandbox home persists only for that session
- `shared`: sandbox home is reused across sessions/containers that share the
  configured home

This affects `/home/sandbox`, not `MEMORY.md` semantics. Long-term memory is
still governed by the mounted Moltis workspace and agent-scoped memory files.
