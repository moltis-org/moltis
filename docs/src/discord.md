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
   - Bot Permissions: `Send Messages`, `Attach Files`, `Read Message History`
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
model = "gpt-4o"
model_provider = "openai"
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
4. Bot Permissions: check **Send Messages** and **Read Message History**
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
boundaries when possible.

Streaming is currently **accumulate-then-send** (same as Microsoft Teams) —
the full response is collected, then sent as one or more messages. Edit-in-place
streaming is planned for a future release.

## Crate Structure

```
crates/discord/
├── Cargo.toml
└── src/
    ├── lib.rs         # Public exports
    ├── config.rs      # DiscordAccountConfig (token, policies, allowlists)
    ├── error.rs       # Error enum (Config, Gateway, Send, Channel)
    ├── handler.rs     # serenity EventHandler (inbound message processing)
    ├── outbound.rs    # ChannelOutbound + ChannelStreamOutbound impls
    ├── plugin.rs      # ChannelPlugin + ChannelStatus impls
    └── state.rs       # AccountState + AccountStateMap
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
