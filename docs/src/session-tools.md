# Session Tools

Session tools enable persistent, asynchronous coordination between agent sessions.

## Available Tools

### `sessions_list`

List sessions visible to the current policy.

Input:

```json
{
  "filter": "optional text",
  "limit": 20
}
```

### `sessions_history`

Read message history from a target session.

Input:

```json
{
  "key": "agent:research:main",
  "limit": 20,
  "offset": 0
}
```

### `sessions_send`

Send a message to another session, optionally waiting for reply.

```json
{
  "key": "agent:coder:main",
  "message": "Please implement JWT middleware",
  "wait_for_reply": true,
  "context": "coordinator"
}
```

## Session Access Policy

Configure policy in a preset to control what sessions a sub-agent can access:

```toml
[agents.presets.coordinator]
tools.allow = ["sessions_list", "sessions_history", "sessions_send", "task_list", "spawn_agent"]
sessions.can_send = true

[agents.presets.observer]
tools.allow = ["sessions_list", "sessions_history"]
sessions.key_prefix = "agent:research:"
sessions.can_send = false
```

Policy fields:

- `key_prefix`: restrict visibility by session-key prefix
- `allowed_keys`: extra explicit session keys
- `can_send`: controls `sessions_send` (default: `true`)
- `cross_agent`: allow access to sessions owned by other agents (default: `false`)

When no policy is configured, all sessions are visible and sendable.

## Coordination Patterns

Use `spawn_agent` when work is short-lived and synchronous.

Use session tools when you need:

- long-lived specialist sessions
- handoffs with durable history
- asynchronous team-style orchestration

Common coordinator flow:

1. `sessions_list` to discover workers
2. `sessions_history` to inspect progress
3. `sessions_send` to dispatch next tasks
4. `task_list` to track cross-session work items
