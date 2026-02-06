# Session State

Moltis provides a per-session key-value store that allows skills, extensions,
and the agent itself to persist context across messages within a session.

## Overview

Session state is scoped to a `(session_key, namespace, key)` triple, backed by
SQLite. Each entry stores a string value and is automatically timestamped.

The agent accesses state through the `session_state` tool, which supports three
operations: `get`, `set`, and `list`.

## Agent Tool

The `session_state` tool is registered as a built-in tool and available in every
session.

### Get a value

```json
{
  "op": "get",
  "namespace": "my-skill",
  "key": "last_query"
}
```

### Set a value

```json
{
  "op": "set",
  "namespace": "my-skill",
  "key": "last_query",
  "value": "SELECT * FROM users"
}
```

### List all keys in a namespace

```json
{
  "op": "list",
  "namespace": "my-skill"
}
```

## Namespacing

Every state entry belongs to a namespace. This prevents collisions between
different skills or extensions using state in the same session. Use your skill
name as the namespace.

## Storage

State is stored in the `session_state` table in the main SQLite database
(`moltis.db`). The migration is in
`crates/sessions/migrations/20260205120000_session_state.sql`.

```admonish tip
State values are strings. To store structured data, serialize to JSON before
writing and parse after reading.
```
