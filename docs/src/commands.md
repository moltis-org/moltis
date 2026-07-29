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
| `/title` | Generate a title from session history |
| `/context` | Show session context and project info — **operator DMs only on channels** |
| `/sessions` | List and switch sessions (channels only) — **operator DMs only** |
| `/attach` | Attach an existing session to this channel (channels only) — **operator DMs only** |
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
| `/agent [N]` | Switch session agent — **operator DMs only on channels** |
| `/mode [N\|name\|none]` | Switch session mode |
| `/model [N]` | Switch provider/model |
| `/sandbox [on\|off\|image N]` | Toggle sandbox and choose image — **operator DMs only on channels** |
| `/sh [on\|off]` | Enter command mode (passthrough to shell) — **operator DMs only on channels** |
| `/stop` | Abort the current running agent |
| `/peek` | Show current thinking/tool status — **operator DMs only on channels** |
| `/update [version]` | Update moltis — **operator DMs only on channels** |

```admonish warning title="Some commands are restricted on channels"
Commands scoped to the current conversation — `/help`, `/new`, `/clear`,
`/compact`, `/title`, `/fork`, `/stop`, `/model`, `/mode`, `/fast` — are open to
any sender who clears the channel's access gate.

Commands that reach the host, act on the owner's behalf, or read state outside
the current chat require the sender to be an **operator in a proven direct
chat**. An empty `operators` list means nobody, so these are disabled until you
configure it. Non-operators can still chat normally.

Shared-room, unknown-topology, and guest turns receive no tools or owner-private
context. `/sh` and other privileged commands are also denied there. See
[Channels → Operators](channels.md#operators-privileged-senders).
```

## Quick Actions

| Command | Description |
|---------|-------------|
| `/btw <question>` | Quick side question (no tools, not persisted) — **operator DMs only on channels** |
| `/fast [on\|off\|status]` | Toggle fast/priority mode |
| `/insights [days]` | Show usage analytics (tokens, providers) — **operator DMs only on channels** |
| `/steer <text>` | Inject guidance into the current agent run — **operator DMs only on channels** |
| `/queue <message>` | Queue a message for the next agent turn — **operator DMs only on channels** |
| `/rollback [N\|diff N]` | List or restore file checkpoints — **operator DMs only on channels** |

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
| `/approvals` | List pending exec approvals — **operator DMs only on channels** |
| `/approve [N]` | Approve a pending exec request — **operator DMs only on channels** |
| `/deny [N]` | Deny a pending exec request — **operator DMs only on channels** |

## Help

| Command | Description |
|---------|-------------|
| `/help` | Show available commands, marking which need an operator |
