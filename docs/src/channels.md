# Channels

Moltis connects to messaging platforms through **channels**. Each channel type
has a distinct inbound mode, determining how it receives messages, and a set of
capabilities that control what features are available.

## Supported Channels

| Channel | Inbound Mode | Public URL Required | Key Capabilities |
|---------|-------------|--------------------|--------------------|
| Telegram | Polling | No | Streaming, voice ingest, reactions, OTP, location |
| Discord | Gateway (WebSocket) | No | Streaming, interactive messages, threads, reactions |
| Microsoft Teams | Webhook | Yes | Streaming, interactive messages, threads |
| WhatsApp | Gateway (WebSocket) | No | Streaming, voice ingest, OTP, pairing, location |
| Slack | Socket Mode | No | Streaming, interactive messages, threads, reactions |

## Inbound Modes

### Polling

The bot periodically fetches new messages from the platform API. No public URL
or open port is needed. Used by Telegram.

### Gateway / WebSocket

The bot opens a persistent outbound WebSocket connection to the platform and
receives events in real time. No public URL needed. Used by Discord and
WhatsApp.

### Socket Mode

Similar to a gateway connection, but uses the platform's Socket Mode protocol.
No public URL needed. Used by Slack.

### Webhook

The platform sends HTTP POST requests to a publicly reachable endpoint on your
server. You must configure the messaging endpoint URL in the platform's
settings. Used by Microsoft Teams.

### None (Send-Only)

For channels that only send outbound messages and do not receive inbound
traffic. No channels currently use this mode, but it is available for future
integrations (e.g. email, SMS).

## Capabilities Reference

| Capability | Description |
|-----------|-------------|
| `supports_outbound` | Can send messages to users |
| `supports_streaming` | Can stream partial responses (typing/editing) |
| `supports_interactive` | Can send interactive components (buttons, menus) |
| `supports_threads` | Can reply in threads |
| `supports_voice_ingest` | Can receive and transcribe voice messages |
| `supports_pairing` | Requires device pairing (QR code) |
| `supports_otp` | Supports OTP-based sender approval |
| `supports_reactions` | Can add/remove emoji reactions |
| `supports_location` | Can receive and process location data |

## Setup

Each channel is configured in `moltis.toml` under `[channels]`:

```toml
[channels.telegram.my_bot]
token = "123456:ABC-DEF..."
dm_policy = "allowlist"
allowlist = ["alice", "bob"]

[channels.msteams.my_teams_bot]
app_id = "..."
app_password = "..."

[channels.discord.my_discord_bot]
token = "..."

[channels.slack.my_slack_bot]
bot_token = "xoxb-..."
app_token = "xapp-..."

[channels.whatsapp.my_wa]
dm_policy = "open"
```

For detailed configuration, see the per-channel pages:
[Telegram](telegram.md), [Discord](discord.md), [Slack](slack.md),
[WhatsApp](whatsapp.md).

You can also use the web UI's **Channels** tab for guided setup with each platform.

## Proactive Outbound Messaging

Agents are not limited to replying in the current chat. Moltis supports three
main outbound patterns:

- **`send_message` tool** for direct proactive messages to any configured channel account/chat
- **Cron job delivery** for background jobs that should post their final output to a channel
- **Heartbeat delivery** for periodic heartbeat acknowledgements sent to a chosen chat

Example `send_message` tool call:

```json
{
  "account_id": "my-telegram-bot",
  "to": "123456789",
  "text": "Deployment finished successfully."
}
```

`account_id` is the configured channel account name from `moltis.toml`, and
`to` is the destination chat, peer, or room identifier for that platform.

## Access Control

All channels share the same access control model with three settings:

### DM Policy

Controls who can send direct messages to the bot.

| Value | Behavior |
|-------|----------|
| `"allowlist"` | Only users listed in `allowlist` can DM (**default for all channels except WhatsApp**) |
| `"open"` | Anyone can DM the bot |
| `"disabled"` | DMs are silently ignored |

```admonish warning title="Empty allowlist blocks everyone"
When `dm_policy = "allowlist"` with an empty `allowlist`, **all DMs are blocked**.
This is a security feature — removing all entries from an allowlist never silently
switches to open access. Add user IDs/usernames to `allowlist` or set
`dm_policy = "open"`.
```

### Group Policy

Controls who can interact with the bot in group chats / channels / guilds.

| Value | Behavior |
|-------|----------|
| `"open"` | Bot responds in all groups (default) |
| `"allowlist"` | Only groups on the allowlist are allowed |
| `"disabled"` | Group messages are silently ignored |

The group allowlist field name varies by channel: `group_allowlist` (Telegram,
WhatsApp, MS Teams), `guild_allowlist` (Discord), `channel_allowlist` (Slack).

### Mention Mode

Controls when the bot responds in groups (does not apply to DMs).

| Value | Behavior |
|-------|----------|
| `"mention"` | Bot only responds when @mentioned (default) |
| `"always"` | Bot responds to every message |
| `"none"` | Bot never responds in groups (DM-only) |

### Allowlist Matching

All allowlist fields across all channels share the same matching behavior:

- **Values are strings** — even for numeric IDs, use `"123456789"` not `123456789`
- **Case-insensitive** — `"Alice"` matches `"alice"`
- **Glob wildcards** — `"admin_*"`, `"*@example.com"`, `"user_*_vip"`
- **Multiple identifiers** — both the user's numeric ID and username are checked (where applicable)

### OTP Self-Approval

Channels that support OTP (Telegram, Discord, WhatsApp) allow non-allowlisted
users to self-approve by entering a 6-digit code. The code appears in the web UI
under **Channels > Senders**. See each channel's page for details.

| Field | Default | Description |
|-------|---------|-------------|
| `otp_self_approval` | `true` | Enable OTP challenges for non-allowlisted DM users |
| `otp_cooldown_secs` | `300` | Lockout duration after 3 failed attempts |
