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

### `sessions_search`

Search prior session history for relevant snippets. By default the current
session is excluded when `_session_key` is available in tool context.

```json
{
  "query": "checkpoint rollback",
  "limit": 5,
  "exclude_current": true
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
tools.allow = ["sessions_list", "sessions_history", "sessions_search", "sessions_send", "task_list", "spawn_agent"]
sessions.can_send = true

[agents.presets.observer]
tools.allow = ["sessions_list", "sessions_history", "sessions_search"]
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

Use `spawn_agent` when work is short-lived and synchronous. For longer delegated
work, call `spawn_agent` with `nonblocking: true`; it returns a `task_id` while
the sub-agent continues in the background. Use `spawn_status` to check progress,
`spawn_result` to fetch the final output, `spawn_list` to recover task IDs after
context loss, and `cancel_spawn` to stop work that is no longer needed.

Use `active_tools` and `tool_choice` to prevent model drift on small/cheap
LLMs. These controls apply **per agent run** (not per iteration within a run)
and are available on agent presets, `spawn_agent`, and `cron` `agentTurn`
payloads.

- `active_tools` filters the tool schemas visible to the agent.
- `tool_choice` controls provider-level tool selection:
  - `auto` — model decides (default).
  - `any` — model must call some tool but chooses which one.
  - `none` — no tools sent; forces text-only output.
  - `tool` + `name` — model must call the named tool.

Supported on Anthropic, OpenAI (Responses and Chat Completions), and
OpenAI-compatible providers.

**Classify-then-generate pattern** — use two `spawn_agent` calls, each with
its own tool controls:

```json
// Turn 1: forced classifier
{
  "task": "Classify whether the reply should be inline, file, or PR.",
  "active_tools": ["classify_destination"],
  "tool_choice": { "type": "tool", "name": "classify_destination" },
  "nonblocking": true
}
// Turn 2: scoped generation (parent reads classifier result, spawns again)
{
  "task": "Generate the report and send it as a document.",
  "active_tools": ["write_file", "send_document"],
  "tool_choice": { "type": "auto" }
}
```

Example preset defaults:

```toml
[agents.presets.destination-router.tool_controls]
active_tools = ["classify_destination"]

[agents.presets.destination-router.tool_controls.tool_choice]
type = "tool"
name = "classify_destination"
```

Example scheduled agent turn:

```json
{
  "kind": "agentTurn",
  "message": "Generate the report and send it as a document.",
  "active_tools": ["write_file", "send_document"],
  "tool_choice": { "type": "any" }
}
```

Use session tools when you need:

- long-lived specialist sessions
- handoffs with durable history
- asynchronous team-style orchestration

Common coordinator flow:

1. `sessions_list` to discover workers
2. `sessions_search` to find prior related work
3. `sessions_history` to inspect progress
4. `sessions_send` to dispatch next tasks
5. `task_list` to track cross-session work items
