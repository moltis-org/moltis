# Slack

Moltis can connect to Slack as a bot, letting you chat with your agent from any
Slack workspace. The integration supports both **Socket Mode** (default, no
public URL needed) and **Events API** (webhook-based).

## How It Works

```
┌──────────────────────────────────────────────────────┐
│                    Slack API                          │
│            (Socket Mode / Events API)                │
└──────────────────┬───────────────────────────────────┘
                   │  WebSocket (Socket Mode)
                   │  or HTTP POST (Events API)
                   ▼
┌──────────────────────────────────────────────────────┐
│                moltis-slack crate                     │
│  ┌────────────┐  ┌────────────┐  ┌────────────────┐  │
│  │  Handler   │  │  Outbound  │  │     Plugin     │  │
│  │ (inbound)  │  │ (replies)  │  │  (lifecycle)   │  │
│  └────────────┘  └────────────┘  └────────────────┘  │
└──────────────────┬───────────────────────────────────┘
                   │
                   ▼
┌──────────────────────────────────────────────────────┐
│                 Moltis Gateway                        │
│         (chat dispatch, tools, memory)                │
└──────────────────────────────────────────────────────┘
```

With **Socket Mode** (the default), the bot opens an outbound WebSocket
connection to Slack — no public URL, port forwarding, or TLS certificate is
needed. With **Events API** mode, Slack sends HTTP POST requests to your server,
requiring a publicly reachable endpoint.

## Prerequisites

Before configuring Moltis, create a Slack app:

1. Go to [api.slack.com/apps](https://api.slack.com/apps) and click **Create New App**
2. Choose **From scratch**, name the app, and select your workspace
3. Navigate to **OAuth & Permissions** and add these Bot Token Scopes:
   - `app_mentions:read` — read @mentions
   - `chat:write` — send messages
   - `im:history` — read DM history
   - `im:read` — view DM metadata
   - `channels:history` — read channel messages (for `mention_mode = "always"`)
4. Click **Install to Workspace** and copy the **Bot User OAuth Token** (`xoxb-...`)
5. For Socket Mode (recommended):
   - Go to **Socket Mode** and enable it
   - Generate an **App-Level Token** (`xapp-...`) with the `connections:write` scope
6. For Events API mode:
   - Go to **Event Subscriptions** and enable it
   - Set the Request URL to your Moltis instance endpoint
   - Copy the **Signing Secret** from **Basic Information**
7. Under **Event Subscriptions > Subscribe to bot events**, add:
   - `app_mention` — when someone @mentions the bot
   - `message.im` — direct messages to the bot

```admonish warning
The bot token and app token are secrets — treat them like passwords. Never
commit them to version control. Moltis stores them with `secrecy::Secret` and
redacts them from logs.
```

## Configuration

Add a `[channels.slack.<account-id>]` section to your `moltis.toml`:

```toml
[channels.slack.my-bot]
bot_token = "xoxb-your-bot-token"
app_token = "xapp-your-app-token"
```

Make sure `"slack"` is included in `channels.offered`:

```toml
[channels]
offered = ["slack"]
```

### Configuration Fields

| Field | Required | Default | Description |
|-------|----------|---------|-------------|
| `bot_token` | **yes** | — | Bot user OAuth token (`xoxb-...`) |
| `app_token` | **yes**\* | — | App-level token for Socket Mode (`xapp-...`). \*Required for `socket_mode`. |
| `connection_mode` | no | `"socket_mode"` | Connection method: `"socket_mode"` or `"events_api"` |
| `signing_secret` | no\* | — | Signing secret for Events API request verification. \*Required for `events_api`. |
| `dm_policy` | no | `"allowlist"` | Who can DM the bot: `"open"`, `"allowlist"`, or `"disabled"` |
| `group_policy` | no | `"open"` | Who can talk to the bot in channels: `"open"`, `"allowlist"`, or `"disabled"` |
| `mention_mode` | no | `"mention"` | When the bot responds in channels: `"always"`, `"mention"`, or `"none"` |
| `allowlist` | no | `[]` | Slack user IDs allowed to DM the bot (when `dm_policy = "allowlist"`) |
| `channel_allowlist` | no | `[]` | Slack channel IDs allowed to interact with the bot |
| `model` | no | — | Override the default model for this channel |
| `model_provider` | no | — | Provider for the overridden model |
| `stream_mode` | no | `"edit_in_place"` | Streaming mode: `"edit_in_place"`, `"native"`, or `"off"` |
| `edit_throttle_ms` | no | `500` | Minimum milliseconds between streaming edit updates |
| `thread_replies` | no | `true` | Reply in threads |

```admonish important title="Allowlist values are strings"
All allowlist entries must be **strings**. Use Slack user IDs like
`["U0123456789"]`. Matching is case-insensitive and supports glob wildcards.
```

### Full Example

```toml
[channels]
offered = ["slack"]

[channels.slack.my-bot]
bot_token = "xoxb-..."
app_token = "xapp-..."
connection_mode = "socket_mode"
dm_policy = "allowlist"
group_policy = "open"
mention_mode = "mention"
allowlist = ["U0123456789", "U9876543210"]
channel_allowlist = ["C0123456789"]
model = "claude-sonnet-4-20250514"
model_provider = "anthropic"
stream_mode = "edit_in_place"
edit_throttle_ms = 500
thread_replies = true
```

### Events API Mode

If you prefer webhook-based delivery instead of Socket Mode:

```toml
[channels.slack.my-bot]
bot_token = "xoxb-..."
connection_mode = "events_api"
signing_secret = "abc123..."
```

This requires your Moltis instance to be reachable from the internet (or use
[Tailscale Funnel](configuration.md#tailscale-integration)).

## Access Control

Slack uses the same gating system as Telegram, Discord, and other channels.

### DM Policy

| Value | Behavior |
|-------|----------|
| `"allowlist"` | Only users listed in `allowlist` can DM (default) |
| `"open"` | Anyone in the workspace can DM the bot |
| `"disabled"` | DMs are silently ignored |

### Group Policy

| Value | Behavior |
|-------|----------|
| `"open"` | Bot responds in any channel it's invited to (default) |
| `"allowlist"` | Only channels listed in `channel_allowlist` are allowed |
| `"disabled"` | Channel messages are silently ignored |

### Mention Mode

| Value | Behavior |
|-------|----------|
| `"mention"` | Bot only responds when @mentioned (default) |
| `"always"` | Bot responds to every message in allowed channels |
| `"none"` | Bot never responds in channels (useful for DM-only bots) |

### Allowlist Matching

Allowlist entries support:

- **Exact match** (case-insensitive): `"U0123456789"`
- **Glob wildcards**: `"U012*"`, `"*admin*"`

## Streaming

Slack supports three streaming modes:

| Mode | Behavior |
|------|----------|
| `"edit_in_place"` | Sends a placeholder message and edits it as tokens arrive (default) |
| `"native"` | Uses Slack's streaming API (`chat.startStream`/`chat.appendStream`/`chat.stopStream`) |
| `"off"` | No streaming — sends the full response as a single message |

The `edit_in_place` mode throttles updates to `edit_throttle_ms` milliseconds
(default: 500) to avoid Slack API rate limits.

## Thread Replies

By default (`thread_replies = true`), the bot replies in a thread attached to
the user's message. Set `thread_replies = false` to have the bot reply directly
in the channel.

## Troubleshooting

### Bot doesn't respond

- Verify the bot and app tokens are correct
- Check that Socket Mode is enabled in the Slack app settings
- Check `dm_policy` — if set to `"allowlist"`, make sure your Slack user ID
  is in `allowlist`
- Ensure the bot has been invited to channels you want it to respond in
- Look at logs: `RUST_LOG=moltis_slack=debug moltis`

### Bot doesn't respond in channels

- Check `mention_mode` — if `"mention"`, you must @mention the bot
- Check `group_policy` — if `"disabled"`, channel messages are ignored
- Check `channel_allowlist` — if non-empty, the channel must be listed
- Ensure the bot is a member of the channel (invite it with `/invite @botname`)
