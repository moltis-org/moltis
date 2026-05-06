# Tool Policy

Tool policies control which tools are available during a session. Policies
use a layered system where each layer can restrict or widen access, and
**deny always wins** — once a tool is denied at any layer, no later layer
can re-allow it.

## Layers

Six layers are evaluated in order. Later layers can replace the allow list,
but deny entries accumulate across all of them.

| # | Layer | Config path | Applies to |
|---|-------|-------------|------------|
| 1 | Global | `[tools.policy]` | All sessions |
| 2 | Per-provider | `[providers.<name>.policy]` | Requests routed through that provider |
| 3 | Per-agent preset | `[agents.presets.<id>.tools]` | Sub-agents spawned with that preset |
| 4 | Per-channel group | `[channels.<type>.<account>.tools.groups.<chat_type>]` | Channel sessions matching that chat type |
| 5 | Per-sender | `...groups.<chat_type>.by_sender.<sender_id>` | Messages from that sender in that group |
| 6 | Sandbox | `[tools.exec.sandbox.tools_policy]` | Commands running inside a sandbox container |

**Web UI sessions** see layers 1-3 (no channel context), plus layer 6 if sandboxed.
**Channel sessions** can see all 6 layers.

## Merge Semantics

Each layer produces an `allow` list and a `deny` list. When merging a
higher-priority layer on top of a lower one:

- **Deny accumulates.** Every deny entry from every layer is collected. If
  any layer denies a tool, it stays denied.
- **Allow replaces.** A non-empty allow list from a later layer replaces the
  previous allow list entirely. An empty allow list is a no-op (keeps the
  previous allow list).
- **Empty allow = permit all.** When the effective allow list is empty,
  everything not denied is allowed.

## Glob Patterns

Both `allow` and `deny` entries support glob-style patterns:

- `"*"` — matches every tool name
- `"browser*"` — matches any tool whose name starts with `browser`
- `"exec"` — matches only the exact tool name `exec`

## Profiles

The global layer and per-sender overrides support a `profile` field that
expands to a predefined allow list before the explicit allow/deny entries
are applied.

| Profile | Allow list |
|---------|-----------|
| `"minimal"` | `exec` |
| `"coding"` | `exec`, `browser`, `memory` |
| `"full"` | `*` (everything) |

When a profile is set, its allow list is applied first, then the explicit
`allow`/`deny` from the same layer are merged on top (deny accumulates,
non-empty allow replaces).

## Layer 1 — Global

The base policy for all sessions. Set in `moltis.toml` under `[tools.policy]`:

```toml
[tools.policy]
allow = []           # empty = permit all tools not denied
deny = ["browser"]   # deny browser in every session
# profile = "full"   # optional named profile
```

## Layer 2 — Per-Provider

Each provider entry can carry its own policy. When a request is routed
through that provider, the policy is merged on top of the global layer.

```toml
[providers.openai]
# ... api_key, models, etc.
policy.deny = ["exec"]
```

This denies `exec` whenever OpenAI is the active provider, regardless of
what the global layer allows. Other providers are unaffected.

## Layer 3 — Per-Agent Preset

Agent presets (used by `spawn_agent`) can restrict their sub-agent's tools.

```toml
[agents.presets.researcher]
model = "anthropic/claude-haiku-3-5-20241022"
tools.allow = ["read_file", "glob", "grep", "web_search", "web_fetch"]
tools.deny  = ["exec", "write_file"]
```

When the `researcher` preset is active, only the five listed tools are
allowed, and `exec`/`write_file` are explicitly denied. See
[Agent Presets](agent-presets.md) for the full preset reference.

> **Note:** Preset tool policies apply only to sub-agents spawned via
> `spawn_agent`. They do not affect the main agent session. Use the global
> `[tools.policy]` for the main session.

## Layer 4 — Per-Channel Group

Channel accounts can restrict tools by chat type (`private`, `group`,
`channel`, etc.). This is useful for hardening group chats where the bot
is exposed to untrusted users.

```toml
[channels.telegram.my-bot.tools.groups.group]
deny = ["exec", "browser"]
```

In this example, `exec` and `browser` are denied in Telegram group chats
handled by the `my-bot` account. Private chats and web UI sessions are
unaffected.

## Layer 5 — Per-Sender

Within a channel group, individual senders can receive overrides. This lets
you trust specific users in an otherwise restricted group.

```toml
[channels.telegram.my-bot.tools.groups.group]
deny = ["exec", "browser"]

[channels.telegram.my-bot.tools.groups.group.by_sender."123456"]
allow = ["*"]
```

Sender `123456` gets `allow = ["*"]`, which replaces the previous allow
list. However, because **deny always accumulates**, the `exec` and
`browser` denials from the group layer still apply. The sender override
is useful for widening the allow list (e.g., granting access to tools that
were not in the previous allow set) or for applying a different profile.

If you need a trusted sender to have `exec` access in a group, avoid
denying `exec` at the group layer. Instead, use a restrictive allow list
at the group level and widen it per-sender:

```toml
[channels.telegram.my-bot.tools.groups.group]
allow = ["web_search", "web_fetch", "memory_search"]

[channels.telegram.my-bot.tools.groups.group.by_sender."123456"]
allow = ["*"]
```

Here, untrusted group members can only use the three listed tools. Sender
`123456` gets full access because the group layer did not deny anything —
it only narrowed the allow list.

## Layer 6 — Sandbox

When a session runs inside a sandbox container, this layer applies on top of
all other layers. It lets you restrict tools for sandboxed execution without
affecting non-sandboxed sessions.

```toml
[tools.exec.sandbox.tools_policy]
allow = ["exec"]         # only exec inside sandbox
deny = ["browser"]       # never allow browser in sandbox
```

This layer is skipped entirely when the session is not sandboxed.

## Examples

### Deny exec for a specific provider

```toml
[providers.openai]
policy.deny = ["exec"]
```

When using OpenAI, the agent cannot run shell commands. All other
providers retain their normal tool access.

### Restrict group chats on Telegram

```toml
[channels.telegram.my-bot.tools.groups.group]
deny = ["exec", "browser*"]
```

Group chats cannot use `exec` or any tool starting with `browser`.
Private chats are unaffected.

### Trust a sender in a restricted group

```toml
[channels.telegram.my-bot.tools.groups.group]
allow = ["web_search", "web_fetch"]

[channels.telegram.my-bot.tools.groups.group.by_sender."123456"]
allow = ["*"]
```

Normal group members can only search and fetch. Sender `123456` can use
every tool (nothing was denied at the group layer, so nothing accumulates).

### Agent preset with limited tools

```toml
[agents.presets.researcher]
tools.allow = ["read_file", "glob", "grep"]
tools.deny  = ["exec"]
```

The `researcher` sub-agent can only read files and search. Even if a
higher layer allows `exec`, it is denied here and the denial carries
through.

### Use a profile for the global policy

```toml
[tools.policy]
profile = "coding"
deny = ["web_fetch"]
```

The `coding` profile expands to `allow = ["exec", "browser", "memory"]`.
Then `web_fetch` is denied. The effective policy allows `exec`, `browser`,
and `memory`, and denies `web_fetch`. All other tools are not in the allow
list and are therefore blocked.

### Widen a sender via profile

```toml
[channels.telegram.my-bot.tools.groups.group]
allow = ["web_search"]

[channels.telegram.my-bot.tools.groups.group.by_sender."123456"]
profile = "full"
```

Sender `123456` gets `allow = ["*"]` from the `full` profile, replacing
the group's narrow allow list. Since the group layer only set `allow` (no
`deny`), nothing is denied and the sender has full tool access.

## Debugging

Enable `debug` logging to see which layers are applied at runtime:

```
policy: applied global profile 'coding'
policy: applied global layer
policy: applied provider layer    provider=openai
policy: applied agent preset layer agent_id=researcher
policy: applied group layer       channel=telegram account_id=my-bot group_id=group
policy: applied sender layer      channel=telegram group_id=group sender_id=123456
policy: applied sandbox layer
```

Each line indicates a layer was non-empty and merged into the effective
policy. Missing lines mean that layer had no configuration or the runtime
context did not match (e.g., no channel context for a web UI session).
