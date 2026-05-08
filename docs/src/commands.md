# Slash Commands

Slash commands are available in the **web UI chat input**, on all **messaging
channels** (Telegram, Discord, Slack, Matrix, etc.), and where noted, via the
**CLI**.

Type `/` in the chat input to see the autocomplete popup.

## Session Management

| Command | Description |
|---------|-------------|
| `/new` | Start a new session |
| `/clear` | Clear session history |
| `/compact` | Summarize conversation to save tokens |
| `/context` | Show session context and project info |
| `/sessions` | List and switch sessions (channels only) |
| `/attach` | Attach an existing session to this channel (channels only) |
| `/fork [label]` | Fork the current session into a new branch |

### /fork

Creates an independent copy of the current conversation. The new session
inherits the parent's model, project, mode, and agent. Messages up to the
current point are copied.

```
/fork experiment-a
```

Available in web UI, all channels, and via the `sessions.fork` RPC. See
[Session Branching](session-branching.md) for details.

## Control

| Command | Description |
|---------|-------------|
| `/agent [N]` | Switch session agent |
| `/mode [N\|name\|none]` | Switch session mode |
| `/model [N]` | Switch provider/model |
| `/sandbox [on\|off\|image N]` | Toggle sandbox and choose image |
| `/sh [on\|off]` | Enter command mode (passthrough to shell) |
| `/stop` | Abort the current running agent |
| `/peek` | Show current thinking/tool status |
| `/update [version]` | Update moltis (owner-only) |

## Quick Actions

| Command | Description |
|---------|-------------|
| `/btw <question>` | Quick side question (no tools, not persisted) |
| `/fast [on\|off\|status]` | Toggle fast/priority mode |
| `/insights [days]` | Show usage analytics (tokens, providers) |
| `/steer <text>` | Inject guidance into the current agent run |
| `/queue <message>` | Queue a message for the next agent turn |
| `/rollback [N\|diff N]` | List or restore file checkpoints |

### /btw

Ask a quick side question without tools and without persisting the exchange to
session history. Uses the session's current model and recent context (last 20
messages) as read-only background.

```
/btw what's the default port for PostgreSQL?
```

The response appears inline and is discarded after display.

### /fast

Toggle fast/priority mode for the current session. When enabled, uses
provider-specific priority processing where supported (Anthropic prompt caching
priority, OpenAI priority processing).

```
/fast          # toggle
/fast on       # enable
/fast off      # disable
/fast status   # check current state
```

Session-scoped — does not persist across gateway restarts.

### /insights

Show usage analytics from the metrics store. Displays LLM completions, token
counts (input/output), errors, tool executions, and per-provider breakdowns.

```
/insights       # last 30 days (default)
/insights 7     # last 7 days
/insights 90    # last 90 days
```

In the web UI, `/insights` renders a formatted markdown table inline. The same
data is available as a dashboard in **Monitoring > Insights** tab, and via the
REST API at `GET /api/metrics/insights?days=N`.

### /steer

Inject guidance into an active agent run without interrupting it. The text is
seen by the LLM on its next iteration (after the current tool call completes).

```
/steer use the staging API, not production
/steer focus on security issues only
```

Only works while an agent run is active. If no run is active, returns an error.

### /queue

Queue a message for the next agent turn without interrupting the current one.
When the active run finishes, the queued message is automatically submitted.

```
/queue now write tests for what you just built
```

If no run is active, the message is sent immediately.

### /rollback

List and restore file checkpoints created by the automatic checkpointing
system. Before every `Write`, `Edit`, or `MultiEdit` tool call, the original
file is snapshotted.

```
/rollback           # list recent turns with file changes
/rollback 1         # restore all files from turn 1
/rollback diff 1    # preview which files were changed in turn 1
```

Checkpoints are grouped by **turn** (one user message = one turn). Restoring a
turn reverts all files that were modified during that turn to their pre-turn
state.

See [Checkpoints](checkpoints.md) for details on the automatic checkpointing
system.

## Approval Management

| Command | Description |
|---------|-------------|
| `/approvals` | List pending exec approvals |
| `/approve [N]` | Approve a pending exec request |
| `/deny [N]` | Deny a pending exec request |

## Help

| Command | Description |
|---------|-------------|
| `/help` | Show available commands (handled locally by each channel) |
