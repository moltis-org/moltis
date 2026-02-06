# Session Branching

Session branching (forking) lets you create an independent copy of a
conversation at any point. The new session diverges without affecting the
original — useful for exploring alternative approaches, running "what if"
scenarios, or preserving a checkpoint before a risky prompt.

## Forking from the UI

There are two ways to fork a session in the web UI:

- **Chat header** — click the **Fork** button in the header bar (next to
  Delete). This is visible for every session except cron sessions.
- **Sidebar** — hover over a session in the sidebar and click the fork icon
  that appears in the action buttons.

Both create a new session that copies all messages from the current one and
immediately switch you to it.

Forked sessions appear **indented** under their parent in the sidebar, with a
branch icon to distinguish them from top-level sessions. The metadata line
shows `fork@N` where N is the message index at which the fork occurred.

## Agent Tool

The agent can also fork programmatically using the `branch_session` tool:

```json
{
  "at_message": 5,
  "label": "explore-alternative"
}
```

- **`at_message`** — the message index to fork at (messages 0..N are copied).
  If omitted, all messages are copied.
- **`label`** — optional human-readable label for the new session.

The tool returns the new session key.

## RPC Method

The `sessions.fork` RPC method is the underlying mechanism:

```json
{ "key": "main", "at_message": 5, "label": "my-fork" }
```

On success the response payload contains `{ "sessionKey": "session:<uuid>" }`.

## What Gets Inherited

When forking, the new session inherits:

| Inherited | Not inherited |
|-----------|---------------|
| Messages (up to fork point) | Worktree branch |
| Model selection | Sandbox settings |
| Project assignment | Channel binding |
| MCP disabled flag | |

## Parent-Child Relationships

Fork relationships are stored directly on the `sessions` table:

- **`parent_session_key`** — the key of the session this was forked from.
- **`fork_point`** — the message index where the fork occurred.

These fields drive the tree rendering in the sidebar. Sessions with a parent
appear indented under it; deeply nested forks indent further.

```admonish warning title="Deleting a parent"
Deleting a parent session does **not** cascade to its children. Child sessions
become top-level sessions — they keep their messages and history but lose
their visual nesting in the sidebar.
```

## Navigation After Delete

When you delete a forked session, the UI navigates back to its parent session.
If the deleted session had no parent (or the parent no longer exists), it falls
back to the next sibling or `main`.

```admonish info title="Independence"
A forked session is fully independent after creation. Changes to the parent
do not propagate to the fork, and vice versa.
```
