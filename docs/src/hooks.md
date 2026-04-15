# Hooks

Hooks let you observe, modify, or block actions at key points in the agent lifecycle. Use them for auditing, policy enforcement, prompt injection filtering, notifications, and custom integrations.

## How Hooks Work

```
┌──────────────────────────────────────────────────────────────┐
│                        Agent Loop                            │
│                                                              │
│  User Message ─→ BeforeLLMCall ─→ LLM Provider              │
│                       │                 │                    │
│                  modify/block      AfterLLMCall              │
│                                         │                    │
│                                    modify/block              │
│                                         │                    │
│                                         ▼                    │
│                                  BeforeToolCall              │
│                                         │                    │
│                                    modify/block              │
│                                         │                    │
│                                    Tool Execution            │
│                                         │                    │
│                                    AfterToolCall             │
│                                         │                    │
│                                         ▼                    │
│                              (loop continues or)             │
│                             Response → MessageSent           │
└──────────────────────────────────────────────────────────────┘
```

## Event Types

### Modifying Events (Sequential)

These events run hooks sequentially. Hooks can modify the payload or block the action.

| Event | Description | Can Modify | Can Block |
|-------|-------------|------------|-----------|
| `BeforeAgentStart` | Before agent loop starts | yes | yes |
| `BeforeLLMCall` | Before prompt is sent to the LLM provider | yes | yes |
| `AfterLLMCall` | After LLM response, before tool execution | yes | yes |
| `BeforeToolCall` | Before a tool executes | yes | yes |
| `BeforeCompaction` | Before context compaction | yes | yes |
| `MessageReceived` | When an inbound channel/UI message arrives | yes | yes |
| `MessageSending` | Before sending a response | yes | yes |
| `ToolResultPersist` | When a tool result is persisted | yes | yes |

For `MessageReceived`, `Block(reason)` aborts the turn — the user message is
not persisted, no run starts, and the reason is delivered back to the sender
via the originating channel (or broadcast as a `chat` rejection event for web
clients). `ModifyPayload` must return an object of shape `{"content": "..."}`;
the `content` string replaces the inbound text before it reaches the model or
the session store.

### Read-Only Events (Parallel)

These events run hooks in parallel for performance. They cannot modify or block.

| Event | Description |
|-------|-------------|
| `AfterToolCall` | After a tool completes |
| `AfterCompaction` | After context is compacted |
| `AgentEnd` | When agent loop completes |
| `MessageSent` | After response is delivered |
| `SessionStart` | When a new session begins |
| `SessionEnd` | When a session ends |
| `GatewayStart` | When Moltis starts |
| `GatewayStop` | When Moltis shuts down |
| `Command` | When a slash command is used |

## Prompt Injection Filtering

The `BeforeLLMCall` and `AfterLLMCall` hooks provide filtering points for prompt injection defense.

### BeforeLLMCall

Fires before each LLM API call. The payload includes the full message array, provider name, model ID, and iteration count. Use it to:

- Scan prompts for injection patterns before they reach the LLM
- Redact PII or sensitive data from the conversation
- Add safety prefixes to system prompts
- Block requests that match known attack patterns

**Payload fields:**

| Field | Type | Description |
|-------|------|-------------|
| `session_key` | string | Session identifier |
| `provider` | string | Provider name (e.g. "openai", "anthropic") |
| `model` | string | Model ID (e.g. "gpt-5.2-codex", "qwen2.5-coder-7b-q4_k_m") |
| `messages` | array | Serialized message array (OpenAI format) |
| `tool_count` | number | Number of tool schemas sent to the LLM |
| `iteration` | number | 1-based loop iteration |

### AfterLLMCall

Fires after the LLM response is received but before tool calls execute. For streaming responses, this fires after the full response is accumulated (text has already been streamed to the UI) but blocking still prevents tool execution.

**Payload fields:**

| Field | Type | Description |
|-------|------|-------------|
| `session_key` | string | Session identifier |
| `provider` | string | Provider name |
| `model` | string | Model ID |
| `text` | string/null | LLM response text |
| `tool_calls` | array | Tool calls requested by the LLM |
| `input_tokens` | number | Tokens consumed by the prompt |
| `output_tokens` | number | Tokens in the response |
| `iteration` | number | 1-based loop iteration |

## Channel Provenance

`BeforeToolCall`, `AfterToolCall`, `SessionStart`, and `MessageReceived` currently include channel provenance. The fields are optional so hooks keep working for sessions that do not originate from a channel integration.

`MessageReceived` keeps its legacy `channel` string field and adds the richer object as `channel_binding`. `BeforeToolCall`, `AfterToolCall`, and `SessionStart` expose the same richer object as `channel`. `ToolResultPersist` has a schema field reserved for the same shape, but that event is not currently dispatched.

| Field | Type | Description |
|-------|------|-------------|
| `surface` | string/null | Runtime surface, for example `telegram`, `discord`, `web`, `cron`, `heartbeat` |
| `session_kind` | string/null | High-level source kind, usually `channel`, `web`, or `cron` |
| `channel_type` | string/null | Channel plugin type when channel-bound |
| `account_id` | string/null | Channel account identifier |
| `chat_id` | string/null | Channel chat, room, or peer identifier |
| `chat_type` | string/null | Best-effort chat classification, currently most useful for Telegram |
| `sender_id` | string/null | Reserved for future sender provenance, currently omitted |

Example `BeforeToolCall` payload excerpt:

```json
{
  "event": "BeforeToolCall",
  "session_key": "telegram:bot-main:-100123",
  "tool_name": "exec",
  "arguments": {
    "command": "pwd"
  },
  "channel": {
    "surface": "telegram",
    "session_kind": "channel",
    "channel_type": "telegram",
    "account_id": "bot-main",
    "chat_id": "-100123",
    "chat_type": "channel_or_supergroup"
  }
}
```

### Example: Block Suspicious Tool Calls

```bash
#!/bin/bash
# filter-injection.sh — subscribe to AfterLLMCall
payload=$(cat)
event=$(echo "$payload" | jq -r '.event')

if [ "$event" = "AfterLLMCall" ]; then
    # Check if tool calls contain suspicious patterns
    tool_names=$(echo "$payload" | jq -r '.tool_calls[].name')

    for name in $tool_names; do
        # Block unexpected tool calls that might come from injection
        case "$name" in
            exec|bash|shell)
                text=$(echo "$payload" | jq -r '.text // ""')
                if echo "$text" | grep -qi "ignore previous\|disregard\|new instructions"; then
                    echo "Blocked suspicious tool call after potential injection" >&2
                    exit 1
                fi
                ;;
        esac
    done
fi

exit 0
```

### Example: External Proxy Filter

```bash
#!/bin/bash
# proxy-filter.sh — subscribe to BeforeLLMCall
payload=$(cat)

# Send to an external moderation API
result=$(echo "$payload" | curl -s -X POST \
  -H "Content-Type: application/json" \
  -d @- \
  "$MODERATION_API_URL/check")

# Block if the API flags it
if echo "$result" | jq -e '.flagged' > /dev/null 2>&1; then
    reason=$(echo "$result" | jq -r '.reason // "content policy violation"')
    echo "$reason" >&2
    exit 1
fi

exit 0
```

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
  "session_key": "abc123",
  "tool_name": "exec",
  "arguments": {
    "command": "ls -la"
  },
  "channel": {
    "surface": "telegram",
    "session_kind": "channel",
    "channel_type": "telegram",
    "account_id": "bot-main",
    "chat_id": "-100123",
    "chat_type": "channel_or_supergroup"
  }
}
```

For modifying events, stdin is the full tagged `HookPayload`. If your hook returns
`{"action":"modify","data":...}`, the `data` value replaces the event-specific
mutable portion of the payload. For `BeforeToolCall`, that means the replacement
value becomes the new `arguments` object.

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
tool=$(echo "$payload" | jq -r '.tool_name')

if [ "$tool" = "exec" ]; then
    # Add safety flag to shell commands executed by the exec tool
    modified_args=$(echo "$payload" | jq '.arguments.command = "set -e; " + .arguments.command | .arguments')
    echo "{\"action\":\"modify\",\"data\":$modified_args}"
fi

exit 0
```

### Example: Block Dangerous Commands

```bash
#!/bin/bash
payload=$(cat)
command=$(echo "$payload" | jq -r '.arguments.command // ""')

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
[hooks]
[[hooks.hooks]]
name = "audit-log"
command = "./hooks/audit.sh"
events = ["BeforeToolCall", "AfterToolCall"]
timeout = 5

[[hooks.hooks]]
name = "llm-filter"
command = "./hooks/filter-injection.sh"
events = ["BeforeLLMCall", "AfterLLMCall"]
timeout = 10

[[hooks.hooks]]
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

- **Threshold**: 3 consecutive failures
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

## Workspace Context Files

Moltis supports several workspace markdown files in `data_dir`.

### BOOT.md

`BOOT.md` is loaded per session and injected into the system prompt as startup context.

Best use is for short, explicit startup tasks (health checks, reminders,
"send one startup message", etc.). If the file is missing or empty, nothing is injected.

Agent-specific overrides are supported: place `BOOT.md` in `agents/<id>/BOOT.md`.

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

### Recommended: Destructive Command Guard (dcg)

[dcg](https://github.com/Dicklesworthstone/destructive_command_guard) is an
external tool that scans shell commands against 49+ destructive pattern
categories, including heredoc/inline-script scanning, database, cloud, and
infrastructure patterns.

**Install:**

Pin to a released tag and verify the script's SHA-256 before executing it —
never pipe an unpinned `curl | bash` from `main`. Check the project's
[releases page](https://github.com/Dicklesworthstone/destructive_command_guard/releases)
for the latest tag and expected checksum.

```bash
DCG_VERSION="v0.4.0"
DCG_SHA256="2cd1287c30cc7bfca3ec6e45a3a474e9bb8f8586dfe83d78db0d6c3a25f3b55c"
curl -fsSL "https://raw.githubusercontent.com/Dicklesworthstone/destructive_command_guard/${DCG_VERSION}/install.sh" -o /tmp/dcg-install.sh
echo "${DCG_SHA256}  /tmp/dcg-install.sh" | shasum -a 256 -c - && bash /tmp/dcg-install.sh
rm /tmp/dcg-install.sh
```

Alternatively, review the script first and only then execute it:

```bash
curl -fsSL "https://raw.githubusercontent.com/Dicklesworthstone/destructive_command_guard/v0.4.0/install.sh" -o /tmp/dcg-install.sh
less /tmp/dcg-install.sh   # review before running
bash /tmp/dcg-install.sh && rm /tmp/dcg-install.sh
```

**Hook setup:**

Copy the bundled hook example to your hooks directory:

```bash
cp -r examples/hooks/dcg-guard ~/.moltis/hooks/dcg-guard
chmod +x ~/.moltis/hooks/dcg-guard/handler.sh
```

The hook subscribes to `BeforeToolCall`, extracts exec commands, pipes them
through dcg, and blocks any command that dcg flags as destructive. See
`examples/hooks/dcg-guard/HOOK.md` for details.

> **Note:** dcg complements but does not replace the built-in dangerous command
> blocklist, sandbox isolation, or the approval system. Use it as an additional
> defense layer with broader pattern coverage.

### Slack Notification on Session End

```bash
#!/bin/bash
# slack-notify.sh
payload=$(cat)
session_key=$(echo "$payload" | jq -r '.session_key')

curl -X POST "$SLACK_WEBHOOK_URL" \
  -H 'Content-Type: application/json' \
  -d "{\"text\":\"Session $session_key ended\"}"

exit 0
```

### Redact Secrets from Tool Arguments

```bash
#!/bin/bash
# redact-secrets.sh
payload=$(cat)

# Redact secrets from exec-tool command arguments before execution
command=$(echo "$payload" | jq -r '.arguments.command // ""')
redacted=$(printf '%s' "$command" | sed -E '
  s/sk-[a-zA-Z0-9]{32,}/[REDACTED]/g
  s/ghp_[a-zA-Z0-9]{36}/[REDACTED]/g
  s/password=[^&[:space:]]+/password=[REDACTED]/g
')

modified_args=$(echo "$payload" | jq --arg command "$redacted" '.arguments.command = $command | .arguments')
echo "{\"action\":\"modify\",\"data\":$modified_args}"
exit 0
```

### Block File Writes Outside Project

```bash
#!/bin/bash
# sandbox-writes.sh
payload=$(cat)
tool=$(echo "$payload" | jq -r '.tool_name')

if [ "$tool" = "write_file" ]; then
    path=$(echo "$payload" | jq -r '.arguments.path')

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
6. **Layer defenses** — Use `BeforeLLMCall` for input filtering and `AfterLLMCall` for output filtering
