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
[channels.telegram.my-bot]
allowlist = ["owner-id"]        # who may DM the bot
operators = ["owner-id"]        # who may use privileged access in proven DMs
```

```admonish warning title="Upgrading: privileged commands are off until you set `operators`"
Earlier versions treated the DM `allowlist` as the privileged list, so anyone
allowed to DM the bot could run `/sh`, `/approve`, and `/update`. That fallback
is gone: **an empty `operators` list means nobody**, including you.

After upgrading, add your own exact platform sender ID to `operators` for each
account, or those commands will refuse everyone. If you do not know your ID,
run `/sh` — the refusal message tells you what yours is on that channel.
```

```admonish warning title="Upgrading: shared chats start from an empty history once"
An untrusted turn only sees history from runs that were themselves untrusted, and
messages written before this change carry no such marker. The first turn after
upgrading in a group chat, guild, or channel-bound session therefore starts with
no prior context. This happens once per session; everything written from then on
accumulates normally.
```

A command requires an operator DM when the worst case reaches beyond the
current chat. Everything scoped to the conversation you are already in stays
open to any sender who clears the access gate:

| | Commands |
|---|---|
| **Anyone who may chat** | `/help`, `/new`, `/clear`, `/compact`, `/title`, `/fork`, `/stop`, `/model`, `/mode`, `/fast` |
| **Operators in proven direct chats only** | `/sh`, `/update`, `/approve`, `/deny`, `/approvals`, `/sandbox`, `/attach`, `/sessions`, `/context`, `/insights`, `/peek`, `/btw`, `/rollback`, `/agent`, `/steer`, `/queue` |

The public set can only disrupt the room's own conversation — something any
member can already do by talking. The operator set runs host commands (`/sh`,
`/update`), acts on the owner's behalf (`/approve` and `/deny` resolve the
*owner's* pending exec requests, which is code execution by proxy), weakens
isolation (`/sandbox` can turn the sandbox off), or reads state outside the
current chat (`/attach`, `/sessions`, `/context`, `/insights`, `/peek`, `/btw`).

New commands default to operator direct-chat only, so adding one is safe until
it is deliberately reviewed.

By default, guest, shared-room, and unknown-topology channel turns receive no
tools. The gateway applies both the registry's public audience ceiling and a
deny-all name policy, so account, group, and per-sender tool policies cannot
restore even a tool reviewed for other untrusted origins unless the channel
account explicitly lifts that ceiling as described below.

Every normal turn in a shared room is untrusted, including turns sent by an
operator, because the shared history contains messages from other people.
`/sh`, shell command mode, and every privileged command are denied there. In an
operator's conversation that the adapter can prove is direct, normal agent
turns may use the full configured tool set. Unknown chat kinds fail closed as
shared.

```admonish note title="Conservative DM detection"
Discord, Microsoft Teams, and Matrix chat IDs do not encode whether the
conversation is direct, and that topology is not yet carried into the gateway.
Those integrations currently fail closed as shared even for actual DMs, so use
the authenticated web UI or a supported proven-direct channel for privileged
work. Normal tool-free chat still works.

Phone calls are also treated as shared, for a different reason: the only
identifier a call carries is the caller number, and caller ID is trivially
spoofable. There is nothing there to authenticate an operator against, so
telephony never grants privileged access no matter what `operators` contains.

Chat kinds are matched as an **allowlist of shapes known to be one-to-one**
(Telegram positive chat IDs, Slack `D…` conversations, WhatsApp
`@s.whatsapp.net` and `@lid` JIDs, Signal non-`group:` identifiers, Nostr DMs).
An ID whose form is not recognised is shared.
```

Untrusted turns also omit owner-private prompt context: user profile, project
context, long-term memory, skills, and automatic memory extraction.

### Channel-bound sessions

A session is *bound* to a chat when the chat created it, or when an operator ran
`/attach` to move an existing session into that chat. Binding is what makes the
chat's replies land in the right place — and it also means the session's history
contains messages the bot did not author.

Every request into a bound session that does not come from the channel gateway
is therefore treated as untrusted, whoever made it: web UI turns, cron jobs,
webhooks, and the `sessions_send` tool all run with no tools and no private
context. The downgrade is logged (`session is bound to a channel; running this
turn without tools or private context`), because otherwise an affected cron job
simply answers as though it had no tools.

```admonish tip title="Getting a session back"
If you attached a working session to a chat and now want it back, open it in the
web UI and press **Release channel** in the session header (or call
`sessions.patch` with `channelBinding: null`). Messages from that chat return to
the chat's own session, and the released session regains tools, memory, and
project context.

A chat's *own* session cannot be released — its history is the room's history,
and the next inbound message would re-create the binding. Attach a different
session to that chat instead.
```

### Granting operator

Two places, and both state what it means:

- **When approving a sender.** Approving asks for a role: *Guest* (may chat) or
  *Operator* (may run commands on this machine). While an account has no
  operators at all, Operator is pre-selected — on a fresh install the person
  holding the OTP code read it off this very UI, so they are almost certainly
  the owner. Once one operator exists, Guest is the default.
- **Settings → Channels → Edit → Operators**, to add or remove IDs directly.

Approving as operator records the sender's exact platform ID, not their
username, because operator matching is exact.

```admonish tip title="Locked out of your own bot?"
Send `/sh`. The refusal tells you your sender ID on that channel, which is what
`operators` needs.
```

```admonish warning title="This matters most in group and guild chats"
In a Discord guild or a group chat, every member who passes the group policy
clears the access gate. Without an `operators` list, anything gated only on
"can this person talk to the bot" is open to the whole room.
```

Resolution is **fail-closed**:

| `operators` | `allowlist` | Who is an operator |
|-------------|-------------|--------------------|
| non-empty | anything | only exact sender IDs in `operators` |
| empty | anything | **nobody** — privileged commands are disabled |

A sender with no identifier (for example an unattributed button callback) is
never an operator.

`operators` contains exact, case-sensitive platform sender IDs. Globs,
usernames, and partial IDs do not grant privilege. For WhatsApp use the full
JID; for Matrix use the full Matrix user ID.

```admonish note title="Interaction with OTP self-approval"
OTP self-approval appends the approved sender to `allowlist` only. It never
promotes the sender to operator; privileged access must be configured
separately.
```

Untrusted tool restrictions stack with the per-channel tool policy
(`channels.<type>.<account>.tools.groups.<chat_type>`). By default those
policies can further restrict an operator DM, but cannot enable tools for a
guest, shared room, or unknown chat: the untrusted ceiling denies everything
before they are consulted. An account that raises its ceiling with
`untrusted_audience` and `untrusted_tools` hands the decision back to those
policies for its own untrusted turns. Slack and WhatsApp support these settings.
The `/sh` shortcut stays restricted to operator direct chats either way.

Grant eligibility for privileged access by adding the sender to `operators`;
the sender must still use a proven direct chat.

### OTP Self-Approval

Channels that support OTP (Telegram, Microsoft Teams, Slack, Discord, Matrix, WhatsApp) allow non-allowlisted
users to self-approve by entering a 6-digit code. The code appears in the web UI
under **Channels > Senders**. See each channel's page for details.

| Field | Default | Description |
|-------|---------|-------------|
| `otp_self_approval` | `true` | Enable OTP challenges for non-allowlisted DM users |
| `otp_cooldown_secs` | `300` | Lockout duration after 3 failed attempts |
