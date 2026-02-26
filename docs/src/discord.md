# Discord

Moltis can connect to Discord as a bot, letting you chat with your agent from
any Discord server or DM. The integration uses Discord's
[Gateway API](https://discord.com/developers/docs/events/gateway) via a
persistent WebSocket connection — no public URL or webhook endpoint is required.

## How It Works

```
┌──────────────────────────────────────────────────────┐
│                   Discord Gateway                     │
│              (wss://gateway.discord.gg)               │
└──────────────────┬───────────────────────────────────┘
                   │  persistent WebSocket
                   ▼
┌──────────────────────────────────────────────────────┐
│               moltis-discord crate                    │
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

The bot connects **outward** to Discord's servers. Unlike Microsoft Teams
(which requires an inbound webhook), Discord needs no port forwarding, no
public domain, and no TLS certificate. This makes it especially easy to run
on a home machine or behind a NAT.

## Prerequisites

Before configuring Moltis, create a Discord bot:

1. Go to the [Discord Developer Portal](https://discord.com/developers/applications)
2. Click **New Application** and give it a name
3. Navigate to **Bot** in the left sidebar
4. Click **Reset Token** and copy the bot token
5. Under **Privileged Gateway Intents**, enable **Message Content Intent**
6. Navigate to **OAuth2 → URL Generator**
   - Scopes: `bot`
   - Bot Permissions: `Send Messages`, `Attach Files`, `Read Message History`, `Add Reactions`
7. Copy the generated URL and open it to invite the bot to your server

```admonish warning
The bot token is a secret — treat it like a password. Never commit it to
version control. Moltis stores it with `secrecy::Secret` and redacts it from
logs, but your `moltis.toml` file is plain text on disk. Consider using
[Vault](vault.md) for encryption at rest.
```

## Configuration

Add a `[channels.discord.<account-id>]` section to your `moltis.toml`:

```toml
[channels.discord.my-bot]
token = "MTIzNDU2Nzg5.example.bot-token"
```

Make sure `"discord"` is included in `channels.offered` so the Web UI shows
the Discord option:

```toml
[channels]
offered = ["telegram", "discord"]
```

### Configuration Fields

| Field | Required | Default | Description |
|-------|----------|---------|-------------|
| `token` | **yes** | — | Discord bot token from the Developer Portal |
| `dm_policy` | no | `"allowlist"` | Who can DM the bot: `"open"`, `"allowlist"`, or `"disabled"` |
| `group_policy` | no | `"open"` | Who can talk to the bot in guild channels: `"open"`, `"allowlist"`, or `"disabled"` |
| `mention_mode` | no | `"mention"` | When the bot responds in guilds: `"always"`, `"mention"` (only when @mentioned), or `"none"` |
| `allowlist` | no | `[]` | Discord usernames allowed to DM the bot (when `dm_policy = "allowlist"`) |
| `guild_allowlist` | no | `[]` | Guild (server) IDs allowed to interact with the bot |
| `model` | no | — | Override the default model for this channel |
| `model_provider` | no | — | Provider for the overridden model |
| `reply_to_message` | no | `false` | Send bot responses as Discord replies to the user's message |
| `ack_reaction` | no | — | Emoji reaction added while processing (e.g. `"👀"`); omit to disable |
| `activity` | no | — | Bot activity status text (e.g. `"with AI"`) |
| `activity_type` | no | `"custom"` | Activity type: `"playing"`, `"listening"`, `"watching"`, `"competing"`, or `"custom"` |
| `status` | no | `"online"` | Bot online status: `"online"`, `"idle"`, `"dnd"`, or `"invisible"` |
| `otp_self_approval` | no | `true` | Enable OTP self-approval for non-allowlisted DM users |
| `otp_cooldown_secs` | no | `300` | Cooldown in seconds after 3 failed OTP attempts |

### Full Example

```toml
[channels]
offered = ["telegram", "discord"]

[channels.discord.my-bot]
token = "MTIzNDU2Nzg5.example.bot-token"
dm_policy = "allowlist"
group_policy = "open"
mention_mode = "mention"
allowlist = ["alice", "bob"]
guild_allowlist = ["123456789012345678"]
reply_to_message = true
ack_reaction = "👀"
model = "gpt-4o"
model_provider = "openai"
activity = "with AI"
activity_type = "custom"
status = "online"
otp_self_approval = true
```

## Access Control

Discord uses the same gating system as Telegram and Microsoft Teams:

### DM Policy

Controls who can send direct messages to the bot.

| Value | Behavior |
|-------|----------|
| `"allowlist"` | Only users listed in `allowlist` can DM (default) |
| `"open"` | Anyone who can reach the bot can DM it |
| `"disabled"` | DMs are silently ignored |

### Group Policy

Controls who can interact with the bot in guild (server) channels.

| Value | Behavior |
|-------|----------|
| `"open"` | Bot responds in any guild channel (default) |
| `"allowlist"` | Only guilds listed in `guild_allowlist` are allowed |
| `"disabled"` | Guild messages are silently ignored |

### Mention Mode

Controls when the bot responds in guild channels (does not apply to DMs).

| Value | Behavior |
|-------|----------|
| `"mention"` | Bot only responds when @mentioned (default) |
| `"always"` | Bot responds to every message in allowed channels |
| `"none"` | Bot never responds in guilds (useful for DM-only bots) |

### Guild Allowlist

If `guild_allowlist` is non-empty, messages from guilds **not** in the list are
silently dropped — regardless of `group_policy`. This provides a server-level
filter on top of the channel-level policy.

### OTP Self-Approval

When `dm_policy = "allowlist"` and `otp_self_approval = true` (the default),
unknown users who DM the bot receive a verification challenge. The flow:

1. User sends a DM to the bot
2. Bot responds with a challenge prompt (the 6-digit code is **not** shown to the user)
3. The code appears in the Moltis web UI under **Channels → Senders**
4. The bot owner shares the code with the user out-of-band
5. User replies with the 6-digit code
6. On success, the user is automatically added to the allowlist

After 3 failed attempts, the user is locked out for `otp_cooldown_secs` seconds
(default: 300). Codes expire after 5 minutes.

```admonish tip
This is the same OTP mechanism used by the Telegram integration. It provides a
simple access control flow without requiring manual allowlist management.
```

## Bot Presence

Configure the bot's Discord presence (the "Playing..." / "Listening to..." status)
using the `activity`, `activity_type`, and `status` fields:

```toml
[channels.discord.my-bot]
token = "..."
activity = "with AI"
activity_type = "custom"  # or "playing", "listening", "watching", "competing"
status = "online"         # or "idle", "dnd", "invisible"
```

The presence is set when the bot connects to the Discord gateway. If no activity
or status is configured, the bot uses Discord's default (online, no activity).

## Slash Commands

The bot automatically registers native Discord slash commands when it connects:

| Command | Description |
|---------|-------------|
| `/new` | Start a new chat session |
| `/clear` | Clear the current session history |
| `/compact` | Summarize the current session |
| `/context` | Show session info (model, tokens, plugins) |
| `/model` | List or switch the AI model |
| `/sessions` | List or switch chat sessions |
| `/agent` | List or switch agents |
| `/help` | Show available commands |

Slash commands appear in Discord's command palette (type `/` in any channel where
the bot is present). Responses are ephemeral — only visible to the user who
invoked the command.

```admonish note
Text-based `/` commands (e.g. typing `/model` as a regular message) continue to
work alongside native slash commands. The native commands provide autocomplete and
a better Discord-native experience.
```

## Web UI Setup

You can also configure Discord through the web interface:

1. Open **Settings → Channels**
2. Click **Connect Discord**
3. Enter an account ID (any alias) and your bot token
4. Adjust DM policy, mention mode, and allowlist as needed
5. Click **Connect**

The same form is available during onboarding when Discord is in `channels.offered`.

## Talking to Your Bot

Once the bot is connected there are several ways to interact with it.

### In a Server

To use the bot in a Discord server you need to invite it first:

1. Go to the [Discord Developer Portal](https://discord.com/developers/applications)
2. Select your application → **OAuth2 → URL Generator**
3. Scopes: check **bot**
4. Bot Permissions: check **Send Messages**, **Read Message History**, and **Add Reactions**
5. Copy the generated URL and open it in your browser
6. Select the server you want to add the bot to and confirm

```admonish tip
The Moltis web UI generates this invite link automatically when you paste your
bot token. Look for the "Invite bot to a server" card in the Connect Discord
dialog.
```

Once the bot is in your server, **@mention** it in any channel to get a
response (assuming `mention_mode = "mention"`, the default). If you set
`mention_mode = "always"` the bot responds to every message in allowed channels.

### Via Direct Message

You can DM the bot directly from Discord — no shared server required:

1. Open Discord and go to **Direct Messages**
2. Click the **New Message** icon (or **Find or start a conversation**)
3. Search for the bot's username and select it
4. Send a message

```admonish note
If `dm_policy` is set to `"allowlist"` (the default), make sure your Discord
username is listed in the `allowlist` array — otherwise the bot will ignore your
DMs. Set `dm_policy = "open"` to allow anyone to DM the bot.
```

### Without a Shared Server

DMs work even if you and the bot don't share a server. Discord bots are
reachable by username from any account. This makes DMs the simplest way to
start chatting — just connect the bot in Moltis and message it directly.

## Message Handling

### Inbound Messages

When a message arrives from Discord:

1. Bot's own messages are ignored
2. Guild allowlist is checked (if configured)
3. DM/group policy is evaluated
4. Mention mode is checked (guild messages only)
5. Bot mention prefix (`@BotName`) is stripped from the message text
6. The message is logged and dispatched to the chat engine
7. Commands (messages starting with `/`) are dispatched to the command handler

### Outbound Messages

Discord enforces a **2,000-character limit** per message. Moltis automatically
splits long responses into multiple messages, preferring to break at newline
boundaries and avoiding splits inside fenced code blocks.

Streaming uses **edit-in-place** — an initial message is sent after 30 characters
and then updated every 500ms as tokens arrive. If the final text exceeds 2,000
characters, the first message is edited to the limit and overflow is sent as
follow-up messages.

### Reply-to-Message

Set `reply_to_message = true` to have the bot send responses as Discord replies
(threaded to the user's original message). The first chunk of a multi-chunk or
streamed response carries the reply reference; follow-up chunks are sent as
regular messages.

### Ack Reactions

Set `ack_reaction = "👀"` (or any Unicode emoji) to have the bot react to the
user's message when processing starts. The reaction is removed once the response
is complete. This provides a visual indicator that the bot has seen the message
and is working on a reply.

## Crate Structure

```
crates/discord/
├── Cargo.toml
└── src/
    ├── lib.rs         # Public exports
    ├── commands.rs    # Native Discord slash command registration
    ├── config.rs      # DiscordAccountConfig (token, policies, presence, OTP)
    ├── error.rs       # Error enum (Config, Gateway, Send, Channel)
    ├── handler.rs     # serenity EventHandler (inbound + OTP + interactions)
    ├── outbound.rs    # ChannelOutbound + ChannelStreamOutbound impls
    ├── plugin.rs      # ChannelPlugin + ChannelStatus impls
    └── state.rs       # AccountState + AccountStateMap (includes OtpState)
```

The crate implements the same trait set as `moltis-telegram` and `moltis-msteams`:

| Trait | Purpose |
|-------|---------|
| `ChannelPlugin` | Start/stop accounts, lifecycle management |
| `ChannelOutbound` | Send text, media, typing indicators |
| `ChannelStreamOutbound` | Handle streaming responses |
| `ChannelStatus` | Health probes (connected / disconnected) |

## Troubleshooting

### Bot doesn't respond

- Verify **Message Content Intent** is enabled in the Developer Portal
- Check that the bot token is correct (reset it if unsure)
- Ensure the bot has been invited to the server with the right permissions
- Check `dm_policy` / `group_policy` — if set to `"allowlist"`, make sure
  your username or guild ID is listed
- Look at logs: `RUST_LOG=moltis_discord=debug moltis`

### "Gateway connection failed"

- Check your network connection — the bot connects outward to
  `wss://gateway.discord.gg`
- Firewalls or proxies that block outbound WebSocket connections will prevent
  the bot from connecting
- The token may have been revoked — regenerate it in the Developer Portal

### Bot responds in DMs but not in guilds

- Check `mention_mode` — if set to `"mention"`, you must @mention the bot
- Check `group_policy` — if `"disabled"`, guild messages are ignored
- Check `guild_allowlist` — if non-empty, the guild must be listed
