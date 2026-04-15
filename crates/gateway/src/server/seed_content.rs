pub(crate) const EXAMPLE_HOOK_MD: &str = r#"+++
name = "example"
description = "Skeleton hook — edit this to build your own"
emoji = "🪝"
events = ["BeforeToolCall"]
# command = "./handler.sh"
# timeout = 10
# priority = 0

# [requires]
# os = ["darwin", "linux"]
# bins = ["jq", "curl"]
# env = ["SLACK_WEBHOOK_URL"]
+++

# Example Hook

This is a skeleton hook to help you get started. It subscribes to
`BeforeToolCall` but has no `command`, so it won't execute anything.

## Quick start

1. Uncomment the `command` line above and point it at your script
2. Create `handler.sh` (or any executable) in this directory
3. Click **Reload** in the Hooks UI (or restart moltis)

## How hooks work

Your script receives the event payload as **JSON on stdin** and communicates
its decision via **exit code** and **stdout**:

| Exit code | Stdout | Action |
|-----------|--------|--------|
| 0 | *(empty)* | **Continue** — let the action proceed |
| 0 | `{"action":"modify","data":{...}}` | **Modify** — alter the payload |
| 1 | *(stderr used as reason)* | **Block** — prevent the action |

## Example handler (bash)

```bash
#!/usr/bin/env bash
# handler.sh — log every tool call to a file
payload=$(cat)
tool=$(echo "$payload" | jq -r '.tool_name // "unknown"')
echo "$(date -Iseconds) tool=$tool" >> /tmp/moltis-hook.log
# Exit 0 with no stdout = Continue
```

## Available events

**Can modify or block (sequential dispatch):**
- `BeforeAgentStart` — before a new agent run begins
- `BeforeLLMCall` — before a prompt is sent to the LLM provider
- `AfterLLMCall` — after an LLM response arrives, before any tool execution
- `BeforeToolCall` — before executing a tool (inspect/modify arguments)
- `BeforeCompaction` — before compacting chat history
- `MessageReceived` — when an inbound channel/UI message arrives;
  `Block(reason)` rejects it, `ModifyPayload({"content": "..."})` rewrites
  the text before the turn begins
- `MessageSending` — before sending a message to the LLM
- `ToolResultPersist` — before persisting a tool result

**Read-only (parallel dispatch, Block/Modify ignored):**
- `AgentEnd` — after an agent run completes
- `AfterToolCall` — after a tool finishes (observe result)
- `AfterCompaction` — after compaction completes
- `MessageSent` — after a message is sent
- `SessionStart` / `SessionEnd` — session lifecycle
- `GatewayStart` / `GatewayStop` — server lifecycle

## Frontmatter reference

```toml
name = "my-hook"           # unique identifier
description = "What it does"
emoji = "🔧"               # optional, shown in UI
events = ["BeforeToolCall"] # which events to subscribe to
command = "./handler.sh"    # script to run (relative to this dir)
timeout = 10                # seconds before kill (default: 10)
priority = 0                # higher runs first (default: 0)

[requires]
os = ["darwin", "linux"]    # skip on other OSes
bins = ["jq"]               # required binaries in PATH
env = ["MY_API_KEY"]        # required environment variables
```
"#;

pub(crate) const DCG_GUARD_HOOK_MD: &str = r#"+++
name = "dcg-guard"
description = "Blocks destructive commands using Destructive Command Guard (dcg)"
emoji = "🛡️"
events = ["BeforeToolCall"]
command = "./handler.sh"
timeout = 5
+++

# Destructive Command Guard (dcg)

Uses the external [dcg](https://github.com/Dicklesworthstone/destructive_command_guard)
tool to scan shell commands before execution. dcg ships 49+ pattern categories
covering filesystem, git, database, cloud, and infrastructure commands.

This hook is **seeded by default** into `~/.moltis/hooks/dcg-guard/` on first
run. When `dcg` is not installed the hook fails open (all commands pass
through) and writes a loud warning to stderr on every invocation — check the
gateway log if the guard appears inert.

## Install dcg

See the upstream [installation section](https://github.com/Dicklesworthstone/destructive_command_guard#installation).
The two supported commands from that README are:

```bash
uv tool install destructive-command-guard
# or
pipx install destructive-command-guard
```

> **Important:** this hook runs inside the **Moltis service environment**,
> not your interactive shell. `dcg` must be resolvable on the service's
> `PATH`. The handler already prepends `$HOME/.local/bin`, `/usr/local/bin`
> and `/opt/homebrew/bin`, which covers the default install locations of
> `uv tool`, `pipx` and Homebrew. If you install `dcg` elsewhere, make sure
> that directory is on the gateway process `PATH` (e.g. via the systemd
> unit's `Environment=PATH=...`).

Once installed, restart Moltis. The startup log will print either
`dcg-guard: dcg <version> detected, guard active` or
`dcg-guard: 'dcg' not found on PATH; destructive command guard is INACTIVE`.
"#;

pub(crate) const DCG_GUARD_HANDLER_SH: &str = r#"#!/usr/bin/env bash
# Hook handler: translates Moltis BeforeToolCall payload to dcg format.
# When dcg is not installed the hook is a fail-open no-op (all commands pass
# through) but a loud warning is written to stderr so the gateway log makes
# it obvious that the guard is inert.

set -euo pipefail

# Hooks run in the Moltis gateway process environment, which under systemd
# often strips `$HOME/.local/bin` and friends. Prepend the usual user/local
# bin directories so `dcg` installed via `uv tool install` / `pipx` / brew is
# resolvable regardless of how Moltis was launched.
export PATH="${HOME:-/root}/.local/bin:/usr/local/bin:/opt/homebrew/bin:${PATH:-/usr/bin:/bin}"

# Warn loudly (but do not block) when dcg is not installed.
if ! command -v dcg >/dev/null 2>&1; then
    echo "dcg-guard: 'dcg' binary not found on PATH (PATH=$PATH); command NOT scanned. Install dcg to enable the guard." >&2
    cat >/dev/null
    exit 0
fi

INPUT=$(cat)

# Only inspect exec tool calls.
TOOL_NAME=$(printf '%s' "$INPUT" | grep -o '"tool_name":"[^"]*"' | head -1 | cut -d'"' -f4)
if [ "$TOOL_NAME" != "exec" ]; then
    exit 0
fi

# Extract the command string from the arguments object.
COMMAND=$(printf '%s' "$INPUT" | grep -o '"command":"[^"]*"' | head -1 | cut -d'"' -f4)
if [ -z "$COMMAND" ]; then
    exit 0
fi

# Build the payload dcg expects and pipe it in.
DCG_INPUT=$(printf '{"tool_name":"Bash","tool_input":{"command":"%s"}}' "$COMMAND")
DCG_RESULT=$(printf '%s' "$DCG_INPUT" | dcg 2>&1) || {
    echo "$DCG_RESULT" >&2
    exit 1
}

exit 0
"#;

pub(crate) const EXAMPLE_SKILL_MD: &str = r#"---
name: template-skill
description: Starter skill template (safe to copy and edit)
---

# Template Skill

Use this as a starting point for your own skills.

## How to use

1. Copy this folder to a new skill name (or edit in place)
2. Update `name` and `description` in frontmatter
3. Replace this body with clear, specific instructions

## Tips

- Keep instructions explicit and task-focused
- Avoid broad permissions unless required
- Document required tools and expected inputs
"#;

pub(crate) const TMUX_SKILL_MD: &str = r#"---
name: tmux
description: Run and interact with terminal applications (htop, vim, etc.) using tmux sessions in the sandbox
allowed-tools:
  - process
---

# tmux — Interactive Terminal Sessions

Use the `process` tool to run and interact with interactive or long-running
programs inside the sandbox. Every command runs in a named **tmux session**,
giving you full control over TUI apps, REPLs, and background processes.

## When to use this skill

- **TUI / ncurses apps**: htop, vim, nano, less, top, iftop
- **Interactive REPLs**: python3, node, irb, psql, sqlite3
- **Long-running commands**: tail -f, watch, servers, builds
- **Programs that need keyboard input**: anything that waits for keypresses

For simple one-shot commands (ls, cat, echo), use `exec` instead.

## Workflow

1. **Start** a session with a command
2. **Poll** to see the current terminal output
3. **Send keys** or **paste text** to interact
4. **Poll** again to see the result
5. **Kill** when done

Always poll after sending keys — the terminal updates asynchronously.

## Actions

### start — Launch a program

```json
{"action": "start", "command": "htop", "session_name": "my-htop"}
```

- `session_name` is optional (auto-generated if omitted)
- The command runs in a 200x50 terminal

### poll — Read terminal output

```json
{"action": "poll", "session_name": "my-htop"}
```

Returns the visible pane content (what a user would see on screen).

### send_keys — Send keystrokes

```json
{"action": "send_keys", "session_name": "my-htop", "keys": "q"}
```

Common key names:
- `Enter`, `Escape`, `Tab`, `Space`
- `Up`, `Down`, `Left`, `Right`
- `C-c` (Ctrl+C), `C-d` (Ctrl+D), `C-z` (Ctrl+Z)
- `C-l` (clear screen), `C-a` / `C-e` (line start/end)
- Single characters: `q`, `y`, `n`, `/`

### paste — Insert text

```json
{"action": "paste", "session_name": "repl", "text": "print('hello world')\n"}
```

Use paste for multi-character input (code, file content). For single
keystrokes, prefer `send_keys`.

### kill — End a session

```json
{"action": "kill", "session_name": "my-htop"}
```

### list — Show active sessions

```json
{"action": "list"}
```

## Tips

- Session names must be `[a-zA-Z0-9_-]` only (no spaces or special chars)
- Always `kill` sessions when done to free resources
- If a program is unresponsive, `send_keys` with `C-c` or `C-\\` first
- Poll output is a snapshot; poll again for updates after sending input
"#;

pub(crate) const DEFAULT_BOOT_MD: &str = r#"<!--
BOOT.md is optional startup context.

How Moltis uses this file:
- Loaded per session and injected into the system prompt.
- Missing/empty/comment-only file = no startup injection.
- Agent-specific overrides: place in agents/<id>/BOOT.md.

Recommended usage:
- Keep it short and explicit.
- Use for startup checks/reminders, not onboarding identity setup.
-->"#;

pub(crate) const DEFAULT_WORKSPACE_AGENTS_MD: &str = r#"<!--
Workspace AGENTS.md contains global instructions for this workspace.

How Moltis uses this file:
- Loaded from data_dir/AGENTS.md when present.
- Injected as workspace context in the system prompt.
- Separate from project AGENTS.md/CLAUDE.md discovery.

Use this for cross-project rules that should apply everywhere in this workspace.
-->"#;

pub(crate) const DEFAULT_TOOLS_MD: &str = r#"<!--
TOOLS.md contains workspace-specific tool notes and constraints.

How Moltis uses this file:
- Loaded from data_dir/TOOLS.md when present.
- Injected as workspace context in the system prompt.

Use this for local setup details (hosts, aliases, device names) and
tool behavior constraints (safe defaults, forbidden actions, etc.).
-->"#;

pub(crate) const DEFAULT_HEARTBEAT_MD: &str = r#"<!--
HEARTBEAT.md is an optional heartbeat prompt source.

Prompt precedence:
1) heartbeat.prompt from config
2) HEARTBEAT.md
3) built-in default prompt

Cost guard:
- If HEARTBEAT.md exists but is empty/comment-only and there is no explicit
  heartbeat.prompt override, Moltis skips heartbeat LLM turns to avoid token use.
-->"#;
