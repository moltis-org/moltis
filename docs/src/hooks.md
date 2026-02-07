# Hooks

Hooks let you observe, modify, or block actions at key points in the agent lifecycle. Use them for auditing, policy enforcement, notifications, and custom integrations.

## How Hooks Work

```
┌─────────────────────────────────────────────────────────┐
│                      Agent Loop                         │
│                                                         │
│  User Message → BeforeToolCall → Tool Execution         │
│                       │                 │               │
│                       ▼                 ▼               │
│                 [Your Hook]      AfterToolCall          │
│                       │                 │               │
│                 modify/block      [Your Hook]           │
│                       │                 │               │
│                       ▼                 ▼               │
│                   Continue → Response → MessageSent     │
└─────────────────────────────────────────────────────────┘
```

## Event Types

### Modifying Events (Sequential)

These events run hooks sequentially. Hooks can modify the payload or block the action.

| Event | Description | Can Modify | Can Block |
|-------|-------------|------------|-----------|
| `BeforeToolCall` | Before a tool executes | ✅ | ✅ |
| `BeforeCompaction` | Before context compaction | ✅ | ✅ |
| `MessageSending` | Before sending a response | ✅ | ✅ |
| `BeforeAgentStart` | Before agent loop starts | ✅ | ✅ |

### Read-Only Events (Parallel)

These events run hooks in parallel for performance. They cannot modify or block.

| Event | Description |
|-------|-------------|
| `AfterToolCall` | After a tool completes |
| `AfterCompaction` | After context is compacted |
| `MessageReceived` | When a user message arrives |
| `MessageSent` | After response is delivered |
| `AgentEnd` | When agent loop completes |
| `SessionStart` | When a new session begins |
| `SessionEnd` | When a session ends |
| `ToolResultPersist` | When tool result is saved |
| `GatewayStart` | When Moltis starts |
| `GatewayStop` | When Moltis shuts down |
| `Command` | When a slash command is used |

## Creating a Hook

### 1. Create the Hook Directory

```bash
mkdir -p ~/.moltis/hooks/my-hook
```

### 2. Create HOOK.md

```markdown
+++
name = "my-hook"
description = "Logs all tool calls to a file"
events = ["BeforeToolCall", "AfterToolCall"]
command = "./handler.sh"
timeout = 5

[requires]
os = ["darwin", "linux"]
bins = ["jq"]
env = ["LOG_FILE"]
+++

# My Hook

This hook logs all tool calls for auditing purposes.
```

### 3. Create the Handler Script

```bash
#!/bin/bash
# handler.sh

# Read event payload from stdin
payload=$(cat)

# Extract event type
event=$(echo "$payload" | jq -r '.event')

# Log to file
echo "$(date -Iseconds) $event: $payload" >> "$LOG_FILE"

# Exit 0 to continue (don't block)
exit 0
```

### 4. Make it Executable

```bash
chmod +x ~/.moltis/hooks/my-hook/handler.sh
```

## Shell Hook Protocol

Hooks communicate via stdin/stdout and exit codes:

### Input

The event payload is passed as JSON on stdin:

```json
{
  "event": "BeforeToolCall",
  "data": {
    "tool": "bash",
    "arguments": {
      "command": "ls -la"
    }
  },
  "session_id": "abc123",
  "timestamp": "2024-01-15T10:30:00Z"
}
```

### Output

| Exit Code | Stdout | Result |
|-----------|--------|--------|
| `0` | (empty) | Continue normally |
| `0` | `{"action":"modify","data":{...}}` | Replace payload data |
| `1` | — | Block (stderr = reason) |

### Example: Modify Tool Arguments

```bash
#!/bin/bash
payload=$(cat)
tool=$(echo "$payload" | jq -r '.data.tool')

if [ "$tool" = "bash" ]; then
    # Add safety flag to all bash commands
    modified=$(echo "$payload" | jq '.data.arguments.command = "set -e; " + .data.arguments.command')
    echo "{\"action\":\"modify\",\"data\":$(echo "$modified" | jq '.data')}"
fi

exit 0
```

### Example: Block Dangerous Commands

```bash
#!/bin/bash
payload=$(cat)
command=$(echo "$payload" | jq -r '.data.arguments.command // ""')

# Block rm -rf /
if echo "$command" | grep -qE 'rm\s+-rf\s+/'; then
    echo "Blocked dangerous rm command" >&2
    exit 1
fi

exit 0
```

## Hook Discovery

Hooks are discovered from `HOOK.md` files in these locations (priority order):

1. **Project-local**: `<workspace>/.moltis/hooks/<name>/HOOK.md`
2. **User-global**: `~/.moltis/hooks/<name>/HOOK.md`

Project-local hooks take precedence over global hooks with the same name.

## Configuration in moltis.toml

You can also define hooks directly in the config file:

```toml
[[hooks]]
name = "audit-log"
command = "./hooks/audit.sh"
events = ["BeforeToolCall", "AfterToolCall"]
timeout = 5
priority = 100  # Higher = runs first

[[hooks]]
name = "notify-slack"
command = "./hooks/slack-notify.sh"
events = ["SessionEnd"]
env = { SLACK_WEBHOOK_URL = "https://hooks.slack.com/..." }
```

## Eligibility Requirements

Hooks can declare requirements that must be met:

```toml
[requires]
os = ["darwin", "linux"]       # Only run on these OSes
bins = ["jq", "curl"]          # Required binaries in PATH
env = ["SLACK_WEBHOOK_URL"]    # Required environment variables
```

If requirements aren't met, the hook is skipped (not an error).

## Circuit Breaker

Hooks that fail repeatedly are automatically disabled:

- **Threshold**: 5 consecutive failures
- **Cooldown**: 60 seconds
- **Recovery**: Auto-re-enabled after cooldown

This prevents a broken hook from blocking all operations.

## CLI Commands

```bash
# List all discovered hooks
moltis hooks list

# List only eligible hooks (requirements met)
moltis hooks list --eligible

# Output as JSON
moltis hooks list --json

# Show details for a specific hook
moltis hooks info my-hook
```

## Bundled Hooks

Moltis includes several built-in hooks:

### boot-md

Reads `BOOT.md` from the workspace on `GatewayStart` and injects it into the agent context.

`BOOT.md` is intended for short, explicit startup tasks (health checks, reminders,
"send one startup message", etc.). If the file is missing or empty, nothing is injected.

## Workspace Context Files

Moltis supports several workspace markdown files in `data_dir`.

### TOOLS.md

`TOOLS.md` is loaded as a workspace context file in the system prompt.

Best use is to combine:

- **Local notes**: environment-specific facts (hosts, device names, channel aliases)
- **Policy constraints**: "prefer read-only tools first", "never run X on startup", etc.

If `TOOLS.md` is empty or missing, it is not injected.

### AGENTS.md (workspace)

Moltis also supports a workspace-level `AGENTS.md` in `data_dir`.

This is separate from project `AGENTS.md`/`CLAUDE.md` discovery. Use workspace
`AGENTS.md` for global instructions that should apply across projects in this workspace.

### session-memory

Saves session context when you use the `/new` command, preserving important information for future sessions.

### command-logger

Logs all `Command` events to a JSONL file for auditing.

## Example Hooks

### Slack Notification on Session End

```bash
#!/bin/bash
# slack-notify.sh
payload=$(cat)
session_id=$(echo "$payload" | jq -r '.session_id')
message_count=$(echo "$payload" | jq -r '.data.message_count')

curl -X POST "$SLACK_WEBHOOK_URL" \
  -H 'Content-Type: application/json' \
  -d "{\"text\":\"Session $session_id ended with $message_count messages\"}"

exit 0
```

### Redact Secrets from Tool Output

```bash
#!/bin/bash
# redact-secrets.sh
payload=$(cat)

# Redact common secret patterns
redacted=$(echo "$payload" | sed -E '
  s/sk-[a-zA-Z0-9]{32,}/[REDACTED]/g
  s/ghp_[a-zA-Z0-9]{36}/[REDACTED]/g
  s/password=[^&\s]+/password=[REDACTED]/g
')

echo "{\"action\":\"modify\",\"data\":$(echo "$redacted" | jq '.data')}"
exit 0
```

### Block File Writes Outside Project

```bash
#!/bin/bash
# sandbox-writes.sh
payload=$(cat)
tool=$(echo "$payload" | jq -r '.data.tool')

if [ "$tool" = "write_file" ]; then
    path=$(echo "$payload" | jq -r '.data.arguments.path')

    # Only allow writes under current project
    if [[ ! "$path" =~ ^/workspace/ ]]; then
        echo "File writes only allowed in /workspace" >&2
        exit 1
    fi
fi

exit 0
```

## Best Practices

1. **Keep hooks fast** — Set appropriate timeouts (default: 5s)
2. **Handle errors gracefully** — Use `exit 0` unless you want to block
3. **Log for debugging** — Write to a log file, not stdout
4. **Test locally first** — Pipe sample JSON through your script
5. **Use jq for JSON** — It's reliable and fast for parsing
