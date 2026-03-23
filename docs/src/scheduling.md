# Scheduling (Cron Jobs)

Moltis includes a built-in cron system that lets the agent schedule and manage
recurring tasks. Jobs can fire agent turns, send system events, or trigger
other actions on a flexible schedule.

## Heartbeat

The **heartbeat** is a special system cron job (`__heartbeat__`) that runs
periodically (default: every 30 minutes). It gives the agent an opportunity to
check reminders, run deferred tasks, and react to events that occurred while
the user was away.

The heartbeat prompt is assembled from `HEARTBEAT.md` in the workspace data
directory. If the file is empty and no pending events exist, the heartbeat
turn is skipped to save tokens.

Heartbeat replies can also be delivered to a configured channel destination via
`[heartbeat] deliver`, `channel`, and `to` in `moltis.toml`, or from the web UI
under **Settings -> Heartbeat**.

## Event-Driven Heartbeat Wake

Normally the heartbeat fires on its regular schedule. The **wake** system lets
other parts of Moltis trigger an immediate heartbeat when something noteworthy
happens, so the agent can react in near real-time.

### Wake Mode

Every cron job has a `wakeMode` field that controls whether it triggers an
immediate heartbeat after execution:

| Value | Behaviour |
|-------|-----------|
| `"nextHeartbeat"` (default) | No extra wake — events are picked up on the next scheduled heartbeat |
| `"now"` | Immediately reschedules the heartbeat to run as soon as possible |

Set `wakeMode` when creating or updating a job through the `cron` tool:

```json
{
  "action": "add",
  "job": {
    "name": "check-deploy",
    "schedule": { "kind": "every", "every_ms": 300000 },
    "payload": { "kind": "agentTurn", "message": "Check deploy status" },
    "wakeMode": "now"
  }
}
```

Aliases are accepted: `"immediate"`, `"immediately"` map to `"now"`;
`"next"`, `"default"`, `"next_heartbeat"`, `"next-heartbeat"` map to
`"nextHeartbeat"`.

### System Events Queue

Moltis maintains an in-memory bounded queue of **system events** — short text
summaries of things that happened (command completions, cron triggers, etc.).
When the heartbeat fires, any pending events are drained from the queue and
prepended to the heartbeat prompt so the agent sees what occurred.

The queue holds up to 20 events. Consecutive duplicate events are
deduplicated. Events that arrive after the buffer is full are silently dropped
(oldest events are preserved).

### Exec Completion Events

When a background command finishes (via the `exec` tool), Moltis automatically
enqueues a summary event with the command name, exit code, and a short preview
of stdout/stderr. If the heartbeat is idle, it is woken immediately so the
agent can react to the result.

This means the agent learns about completed background tasks without polling
— a long-running build or deployment that finishes while the user is away will
surface in the next heartbeat turn.

## Schedule Types

Jobs support several schedule kinds:

| Kind | Fields | Description |
|------|--------|-------------|
| `every` | `every_ms` | Repeat at a fixed interval (milliseconds) |
| `cron` | `expr`, optional `tz` | Standard cron expression (e.g. `"0 */6 * * *"`) |
| `at` | `at_ms` | Run once at a specific Unix timestamp (ms) |

## Cron Tool

The agent manages jobs through the built-in `cron` tool. Available actions:

- **`add`** — Create a new job
- **`list`** — List all jobs
- **`run`** — Trigger a job immediately
- **`update`** — Patch an existing job (name, schedule, enabled, wakeMode, etc.)
- **`remove`** — Delete a job
- **`runs`** — View recent execution history for a job

### One-Shot Jobs

Set `deleteAfterRun: true` to automatically remove a job after its first
execution. Combined with the `at` schedule, this is useful for deferred
one-time tasks (reminders, follow-ups).

## Channel Delivery

Background agent turns can deliver their final output to a configured channel
account/chat after the run completes.

Use all of the following together:

- `sessionTarget: "isolated"`
- `payload.kind: "agentTurn"`
- `payload.deliver: true`
- `payload.channel: "<account_id>"`
- `payload.to: "<chat_or_peer_id>"`

Example:

```json
{
  "action": "add",
  "job": {
    "name": "daily summary",
    "schedule": { "kind": "cron", "expr": "0 9 * * *", "tz": "Europe/Paris" },
    "sessionTarget": "isolated",
    "payload": {
      "kind": "agentTurn",
      "message": "Summarize yesterday's activity and send it to Telegram.",
      "deliver": true,
      "channel": "my-telegram-bot",
      "to": "123456789"
    }
  }
}
```

Channel delivery is separate from session targeting. The cron job still runs in
an isolated cron session, then Moltis forwards the finished output to the
requested channel destination.

## Session Targeting

Each job specifies where its agent turn runs:

| Target | Description |
|--------|-------------|
| `"main"` | Inject a `systemEvent` into the main session. Use this for reminders that should continue the main conversation. |
| `"isolated"` | Run an `agentTurn` in a throwaway cron session. Use this for background jobs and channel delivery. |
| `named("<name>")` | Internal/persistent cron session used by system jobs such as heartbeat. Not user-configurable through the `cron` tool. |

## Security

See [Cron Job Security](security.md#cron-job-security) for rate limiting,
sandbox isolation, and job notification details.

## Metrics

| Metric | Type | Description |
|--------|------|-------------|
| `moltis_cron_jobs_scheduled` | Gauge | Number of scheduled jobs |
| `moltis_cron_executions_total` | Counter | Job executions |
| `moltis_cron_execution_duration_seconds` | Histogram | Job duration |
| `moltis_cron_errors_total` | Counter | Failed jobs |
| `moltis_cron_stuck_jobs_cleared_total` | Counter | Jobs exceeding 2h timeout |
| `moltis_cron_input_tokens_total` | Counter | Input tokens from cron runs |
| `moltis_cron_output_tokens_total` | Counter | Output tokens from cron runs |
