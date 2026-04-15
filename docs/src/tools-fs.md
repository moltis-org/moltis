# Filesystem Tools

Moltis ships six native filesystem tools that agents use for structured,
typed file I/O: `Read`, `Write`, `Edit`, `MultiEdit`, `Glob`, and `Grep`.
Their schemas match Claude Code exactly so LLMs trained on those tools
work without adaptation. See [GitHub #657](https://github.com/moltis-org/moltis/issues/657)
for background.

Prefer these over shelling out via the `exec` tool running `cat` / `sed` /
`rg` — the native tools give the model line-numbered reads, uniqueness-
enforced edits, typed error payloads, and structured audit logs.

## Tools

### `Read`

Read a file with line-numbered output.

```json
{
  "file_path": "/absolute/path/to/file.rs",
  "offset": 1,
  "limit": 2000
}
```

- `file_path` — absolute path (required). Relative paths are rejected.
- `offset` — 1-indexed line to start at. Default `1`.
- `limit` — max lines to return. Default `2000`.

Returns one of the following typed payloads (the `kind` field is the
discriminator the LLM branches on):

- `kind: "text"` — happy path. Includes `content` (cat-n style line
  numbers), `total_lines`, `rendered_lines`, `start_line`, `truncated`.
- `kind: "binary"` — file detected as binary. Includes `bytes`. With
  `binary_policy = "base64"` the payload also carries `base64`.
- `kind: "not_found"` — file does not exist.
- `kind: "permission_denied"` — read not permitted by filesystem ACLs.
- `kind: "too_large"` — file exceeds `max_read_bytes`.
- `kind: "not_regular_file"` — path is a directory, fifo, socket, etc.
- `kind: "path_denied"` — blocked by `[tools.fs]` allow/deny rules.

CRLF files are rendered with `\r` stripped so the line-number column
aligns correctly; the file on disk is not modified.

### `Write`

Atomically write a file. Parent directories must already exist. Refuses
to follow symlinks.

```json
{
  "file_path": "/absolute/path/to/file.rs",
  "content": "fn main() {}\n"
}
```

Returns `{ file_path, bytes_written, checkpoint_id }`. `checkpoint_id`
is `null` unless `[tools.fs].checkpoint_before_mutation` is enabled.

### `Edit`

Exact-match string replacement. Refuses to edit if `old_string` is not
unique in the file unless `replace_all=true` — the uniqueness requirement
is the main correctness win over shell `sed`.

```json
{
  "file_path": "/absolute/path/to/file.rs",
  "old_string": "fn foo()",
  "new_string": "fn bar()",
  "replace_all": false
}
```

Returns `{ file_path, replacements, replace_all, recovered_via_crlf, checkpoint_id }`.
`recovered_via_crlf` is `true` when the tool fell back to CRLF matching
for an LF-only `old_string` against a CRLF file.

### `MultiEdit`

Apply multiple sequential edits to a single file atomically. Each edit
sees the output of the previous. Either all succeed or the file is left
untouched.

```json
{
  "file_path": "/absolute/path/to/file.rs",
  "edits": [
    { "old_string": "alpha", "new_string": "ALPHA" },
    { "old_string": "beta",  "new_string": "BETA" }
  ]
}
```

### `Glob`

Find files matching a glob pattern, sorted by modification time (newest
first). Respects `.gitignore` by default.

```json
{
  "pattern": "**/*.rs",
  "path": "/absolute/path/to/project"
}
```

`path` is required unless `[tools.fs].workspace_root` is configured. A
relative `path` is rejected.

### `Grep`

Regex content search. Walks with the same ignore rules as `Glob`.

```json
{
  "pattern": "fn\\s+main",
  "path": "/absolute/path/to/project",
  "output_mode": "content",
  "glob": "**/*.rs",
  "-n": true,
  "-C": 2
}
```

Parameters: `pattern` (regex, required), `path`, `glob`, `type`
(`rust` / `py` / `ts` / etc.), `output_mode` (`files_with_matches` /
`content` / `count`), `-i` (case-insensitive), `-n` (line numbers),
`-A` / `-B` / `-C` (context lines), `multiline`, `head_limit`, `offset`.

## Configuration

All fs tools are configured under a single `[tools.fs]` section. Every
field is optional and conservative by default — the tools work out of
the box with no configuration.

```toml
[tools.fs]
# Default search root for Glob/Grep when `path` is omitted. Absolute.
# workspace_root = "/home/user/projects/my-app"

# Absolute path globs the fs tools may touch. Empty = no allowlist.
# Evaluated after canonicalization so symlinks can't escape.
allow_paths = []

# Deny globs. Deny wins over allow.
deny_paths = []

# Per-session Read history + loop detection. Prerequisite for
# must_read_before_write.
track_reads = false

# Refuse Write/Edit/MultiEdit on files the session hasn't Read.
# Requires track_reads = true.
must_read_before_write = false

# When true, Write/Edit/MultiEdit pause for explicit operator approval
# before mutating a file.
require_approval = true

# Read size cap. Files larger than this return a typed "too_large" payload.
max_read_bytes = 10485760  # 10 MB

# Binary file handling:
#   "reject" — typed marker without content (default)
#   "base64" — include base64-encoded bytes in the payload
binary_policy = "reject"

# Whether Glob/Grep respect .gitignore / .ignore / .git/info/exclude.
respect_gitignore = true

# When true, Write/Edit/MultiEdit snapshot the target file via the
# existing CheckpointManager before mutating, so the pre-edit state can
# be restored via the `checkpoint_restore` tool. Off by default because
# checkpoints grow with agent activity.
checkpoint_before_mutation = false
```

`require_approval` reuses the existing approval queue and WebSocket
prompting path. If nobody approves the request, the mutation times out
instead of landing silently.

## Policy Integration

The `[tools.policy]` allow/deny list gates access by tool name, not by
file path. You can make an agent read-only without touching fs-level
policy:

```toml
[tools.policy]
deny = ["Write", "Edit", "MultiEdit"]
```

The agent retains `Read`, `Glob`, and `Grep` — no need to deny the
shell tool wholesale and lose every other capability.

File-path allow/deny lives in `[tools.fs]`. Layer both for fine-grained
control. Example: a code-reviewer agent that can read the project tree
but can't touch anything outside it:

```toml
[tools.policy]
allow = ["exec", "browser", "memory", "Read", "Glob", "Grep"]

[tools.fs]
workspace_root = "/home/user/project"
allow_paths = ["/home/user/project/**"]
deny_paths  = ["/home/user/project/.env*", "/home/user/project/secrets/**"]
respect_gitignore = true
require_approval = true
```

## Structured Audit

Every tool invocation is a structured event through moltis's existing
tracing layer and the `moltis_tool_executions_total` /
`moltis_tool_execution_errors_total` metrics. Writes appear in traces as
structured key/value pairs — `tool=Write file_path=... bytes=... outcome=ok`
— dramatically easier to review than an opaque shell command string, and
the second big win (alongside model-quality improvements) that motivated
#657.

## Related

- [Checkpoints](checkpoints.md) — pairs with `checkpoint_before_mutation`
  for opt-in pre-edit snapshots.
- [Hooks](hooks.md) — `BeforeToolCall` and `ToolResultPersist` receive
  structured payloads for each fs tool call, so policy hooks can inspect
  typed parameters instead of parsing shell strings.
- [Sandbox](sandbox.md) — fs tools route through the sandbox when the
  session is sandboxed. If no real sandbox backend is available, Moltis
  warns at startup and the tools operate on the gateway host.
