# Channels

Moltis connects to messaging platforms through **channels**. Each channel type
has a distinct inbound mode, determining how it receives messages, and a set of
capabilities that control what features are available.

## Supported Channels

| Channel | Inbound Mode | Public URL Required | Key Capabilities |
|---------|-------------|--------------------|--------------------|
| Telegram | Polling | No | Streaming, voice ingest, reactions, OTP, location |
| Discord | Gateway (WebSocket) | No | Streaming, interactive messages, threads, voice ingest, reactions |
| Matrix | Gateway (sync loop) | No | Streaming, voice ingest, interactive polls, threads, reactions, OTP, location, encrypted chats, device verification, ownership bootstrap |
| Microsoft Teams | Webhook | Yes | Streaming, interactive messages, threads, reactions |
| WhatsApp | Gateway (WebSocket) | No | Streaming, voice ingest, OTP, pairing, location |
| Slack | Socket Mode | No | Streaming, interactive messages, threads, reactions |
| Nostr | Gateway (relay subscription) | No | OTP, encrypted DMs (NIP-04) |
| Signal | Gateway (signal-cli SSE) | No | OTP, DMs, groups, outbound text |

## Inbound Modes

### Polling

The bot periodically fetches new messages from the platform API. No public URL
or open port is needed. Used by Telegram.

### Gateway / WebSocket

The bot opens a persistent outbound WebSocket connection to the platform and
receives events in real time, or uses a persistent sync loop over outbound HTTP.
No public URL needed. Used by Discord, Matrix, and WhatsApp.

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

Channels can be configured in two places:

- In `moltis.toml` under `[channels]`, for file-managed setups
- In the web UI under **Settings -> Channels**, which stores channel accounts in the internal `channels` table inside `data_dir()/moltis.db`

The web UI does not write channel settings back into `moltis.toml`. It includes an advanced JSON config editor so channel-specific settings remain reachable even when a dedicated form field has not been added yet.

The channel picker itself is controlled by `[channels].offered` in
`moltis.toml`. If you edit that list by hand, reload the page so the web UI
re-reads the current picker options.

Channel configs stored through the web UI currently live as JSON records in the
internal `channels` table in `data_dir()/moltis.db`. They are not currently
wrapped by the Moltis vault, so treat local access to that database as access
to the configured channel credentials.

Some channel integrations also have platform-specific limits. For Matrix,
encrypted chats require password auth. Access-token auth is only suitable for
plain Matrix traffic because Moltis cannot import an existing device's private
E2EE keys from an access token alone. See [Matrix](./matrix.md) for the full
setup, ownership, verification, and troubleshooting flow.

`moltis.toml` and the web UI are both loaded at startup. If the same `(channel_type, account_id)` exists in both, the `moltis.toml` entry wins.

Manual file configuration looks like this:

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

[channels.matrix.my_matrix_bot]
homeserver = "https://matrix.example.com"
access_token = "syt_..."
user_id = "@bot:example.com"

[channels.whatsapp.my_wa]
dm_policy = "open"

[channels.signal.my_signal]
account = "+15551234567"
http_url = "http://127.0.0.1:8080"
```

For detailed configuration, see the per-channel pages:
[Telegram](telegram.md), [Microsoft Teams](teams.md), [Discord](discord.md),
[Slack](slack.md), [Matrix](matrix.md), [WhatsApp](whatsapp.md),
[Nostr](nostr.md), [Signal](signal.md).

You can also use the web UI's **Channels** tab for guided setup with each platform. Web-added channels do not get written back into `moltis.toml`.

For Matrix specifically, the web UI now supports the full normal setup flow:

- password auth is the default because it unlocks encrypted chats
- dedicated bot accounts default to `moltis_owned` so Moltis can bootstrap cross-signing and recovery
- older Matrix accounts that need one external approval expose that approval flow in the channel card instead of failing silently

## Proactive Outbound Messaging

Agents are not limited to replying in the current chat. Moltis supports three
main outbound patterns:

- **`send_message` tool** for direct proactive messages to any configured channel account/chat
- **`update_channel_settings` tool** for safe in-chat edits to channel access rules, allowlists, and model routing
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

`account_id` is the configured channel account name, either from `moltis.toml` or from a channel account stored through the web UI, and `to` is the destination chat, peer, or room identifier for that platform.

Example `update_channel_settings` tool call:

```json
{
  "account_id": "my-telegram-bot",
  "settings": {
    "dm_policy": "allowlist",
    "allowlist_add": ["alice"],
    "model": "openai/gpt-5"
  }
}
```

`update_channel_settings` intentionally supports a narrow patch surface. It is
for non-secret channel settings only, not raw `moltis.toml` editing, token
rotation, or arbitrary config mutation.

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
WhatsApp, MS Teams), `guild_allowlist` (Discord), `channel_allowlist` (Slack),
`room_allowlist` (Matrix).

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

### Operators (Privileged Senders)

Passing the access gate lets someone *talk* to the bot. It does not let them
run commands on your machine. That is a separate list:

```toml
[channels.discord.my-bot]
allowlist = ["owner-id"]        # who may DM the bot
operators = ["owner-id"]        # who may run /sh and host-reaching tools
```

Operators may use `/sh`, shell command mode, `/approve`, `/deny`, and `/update`.
Everyone else can chat normally but is refused shell access and runs the agent
**without** the privileged tools:

- **Host reach** — `exec`, `process`, `write_file`, `browser`,
  `sandbox_packages`, `nodes_*`, and the external agent bridges (`codex`,
  `opencode`, `anthropic`)
- **Your private state** — `memory_*`, `sessions_*`, `codebase_*`, `caldav`,
  `teams_*`, `get_user_location`, checkpoints and session branching
- **Escalation** — `update_channel_settings`, which edits allowlists and this
  operator list
- **Persistence and side effects** — `cron`, `webhook`, `spawn_*`,
  `send_message`, `voice_call`, `speak`, `notify`, `home_assistant`, skill
  authoring, and every MCP tool (`mcp_*`)

Guests keep `web_search`, `web_fetch`, `calc`, `generate_image`, `send_image`,
`send_document`, and the other informational tools.

Edit the list in the web UI under **Settings → Channels → Edit → Operators**.

```admonish warning title="This matters most in group and guild chats"
In a Discord guild or a group chat, every member who passes the group policy
clears the access gate. Without an `operators` list, anything gated only on
"can this person talk to the bot" is open to the whole room.
```

Resolution is **fail-closed**, in this order:

| `operators` | `allowlist` | Who is an operator |
|-------------|-------------|--------------------|
| non-empty | anything | only senders matching `operators` |
| empty | non-empty | senders matching `allowlist` (compatibility fallback) |
| empty | empty | **nobody** — shell access is disabled entirely |

A sender with no identifier (for example an unattributed button callback) is
never an operator.

`operators` uses the same matching rules as allowlists (case-insensitive, glob
wildcards, user-part matching for suffixed IDs like WhatsApp JIDs) with one
deliberate difference: **an empty `operators` list matches nobody**, whereas an
empty `allowlist` means "open".

```admonish note title="Interaction with OTP self-approval"
OTP self-approval appends the approved sender to `allowlist`. On an account with
no explicit `operators`, the fallback therefore promotes OTP-approved senders to
operators. Set `operators` explicitly on any bot where people other than you can
get onto the allowlist.
```

Guest tool restrictions stack with the per-channel tool policy
(`channels.<type>.<account>.tools.groups.<chat_type>`), so you can tighten them
further — but note that policy denials always win, so it cannot be used to hand
a guest back a denied tool. Grant privileges by adding the sender to
`operators`.

### OTP Self-Approval

Channels that support OTP (Telegram, Microsoft Teams, Slack, Discord, Matrix, WhatsApp) allow non-allowlisted
users to self-approve by entering a 6-digit code. The code appears in the web UI
under **Channels > Senders**. See each channel's page for details.

| Field | Default | Description |
|-------|---------|-------------|
| `otp_self_approval` | `true` | Enable OTP challenges for non-allowlisted DM users |
| `otp_cooldown_secs` | `300` | Lockout duration after 3 failed attempts |
