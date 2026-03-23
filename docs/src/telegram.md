# Telegram

Moltis can connect to Telegram as a bot, letting you chat with your agent from
any Telegram conversation. The integration uses Telegram's
[Bot API](https://core.telegram.org/bots/api) with long polling — no public URL
or webhook endpoint is required.

## How It Works

```
┌──────────────────────────────────────────────────────┐
│                 Telegram Bot API                      │
│            (api.telegram.org/bot...)                  │
└──────────────────┬───────────────────────────────────┘
                   │  long polling (getUpdates)
                   ▼
┌──────────────────────────────────────────────────────┐
│              moltis-telegram crate                    │
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

The bot connects **outward** to Telegram's servers via polling. No port
forwarding, public domain, or TLS certificate is needed. This makes it easy to
run on a home machine or behind a NAT.

## Prerequisites

Before configuring Moltis, create a Telegram bot:

1. Open Telegram and message [@BotFather](https://t.me/BotFather)
2. Send `/newbot` and follow the prompts to choose a name and username
3. Copy the bot token (e.g. `123456:ABC-DEF1234ghIkl-zyx57W2v1u123ew11`)

```admonish warning
The bot token is a secret — treat it like a password. Never commit it to
version control. Moltis stores it with `secrecy::Secret` and redacts it from
logs, but your `moltis.toml` file is plain text on disk. Consider using
[Vault](vault.md) for encryption at rest.
```

## Configuration

Add a `[channels.telegram.<account-id>]` section to your `moltis.toml`:

```toml
[channels.telegram.my-bot]
token = "123456:ABC-DEF1234ghIkl-zyx57W2v1u123ew11"
```

Make sure `"telegram"` is included in `channels.offered` so the Web UI shows
the Telegram option:

```toml
[channels]
offered = ["telegram"]
```

### Configuration Fields

| Field | Required | Default | Description |
|-------|----------|---------|-------------|
| `token` | **yes** | — | Bot token from @BotFather |
| `dm_policy` | no | `"allowlist"` | Who can DM the bot: `"open"`, `"allowlist"`, or `"disabled"` |
| `group_policy` | no | `"open"` | Who can talk to the bot in groups: `"open"`, `"allowlist"`, or `"disabled"` |
| `mention_mode` | no | `"mention"` | When the bot responds in groups: `"always"`, `"mention"` (only when @mentioned), or `"none"` |
| `allowlist` | no | `[]` | User IDs or usernames allowed to DM the bot (when `dm_policy = "allowlist"`) |
| `group_allowlist` | no | `[]` | Group/chat IDs allowed to interact with the bot |
| `model` | no | — | Override the default model for this channel |
| `model_provider` | no | — | Provider for the overridden model |
| `reply_to_message` | no | `false` | Send bot responses as Telegram replies to the user's message |
| `otp_self_approval` | no | `true` | Enable OTP self-approval for non-allowlisted DM users |
| `otp_cooldown_secs` | no | `300` | Cooldown in seconds after 3 failed OTP attempts |
| `stream_mode` | no | `"edit_in_place"` | Streaming mode: `"edit_in_place"` or `"off"` |
| `edit_throttle_ms` | no | `300` | Minimum milliseconds between streaming edit updates |
| `stream_notify_on_complete` | no | `false` | Send a completion notification after streaming finishes |
| `stream_min_initial_chars` | no | `30` | Minimum characters before sending the first streamed message |

```admonish important title="Allowlist values are strings"
All allowlist entries must be **strings**, even for numeric Telegram user IDs.
Write `allowlist = ["123456789"]`, not `allowlist = [123456789]`.
Both numeric user IDs and usernames are supported — the bot checks both when
evaluating access.
```

### Full Example

```toml
[channels]
offered = ["telegram"]

[channels.telegram.my-bot]
token = "123456:ABC-DEF1234ghIkl-zyx57W2v1u123ew11"
dm_policy = "allowlist"
group_policy = "open"
mention_mode = "mention"
allowlist = ["123456789", "alice_username"]
group_allowlist = ["-1001234567890"]
reply_to_message = true
model = "claude-sonnet-4-20250514"
model_provider = "anthropic"
otp_self_approval = true
stream_mode = "edit_in_place"
edit_throttle_ms = 300
```

### Per-User and Per-Channel Model Overrides

You can override the model for specific users or group chats:

```toml
[channels.telegram.my-bot]
token = "..."
model = "claude-sonnet-4-20250514"
model_provider = "anthropic"

[channels.telegram.my-bot.channel_overrides."-1001234567890"]
model = "gpt-4o"
model_provider = "openai"

[channels.telegram.my-bot.user_overrides."123456789"]
model = "claude-opus-4-20250514"
model_provider = "anthropic"
```

User overrides take priority over channel overrides, which take priority over
the account default.

## Access Control

Telegram uses the same gating system as Discord and other channels.

### DM Policy

Controls who can send direct messages to the bot.

| Value | Behavior |
|-------|----------|
| `"allowlist"` | Only users listed in `allowlist` can DM (default) |
| `"open"` | Anyone who can find the bot can DM it |
| `"disabled"` | DMs are silently ignored |

```admonish warning title="Default denies unlisted users"
The default `dm_policy` is `"allowlist"`. With an empty `allowlist`, **all DMs
are blocked**. You must either add entries to `allowlist` or set
`dm_policy = "open"`.
```

### Group Policy

Controls who can interact with the bot in group chats.

| Value | Behavior |
|-------|----------|
| `"open"` | Bot responds in any group (default) |
| `"allowlist"` | Only groups listed in `group_allowlist` are allowed |
| `"disabled"` | Group messages are silently ignored |

### Mention Mode

Controls when the bot responds in groups (does not apply to DMs).

| Value | Behavior |
|-------|----------|
| `"mention"` | Bot only responds when @mentioned (default) |
| `"always"` | Bot responds to every message in allowed groups |
| `"none"` | Bot never responds in groups (useful for DM-only bots) |

### Allowlist Matching

Allowlist entries support:

- **Exact match** (case-insensitive): `"alice"`, `"123456789"`
- **Glob wildcards**: `"admin_*"`, `"*_bot"`, `"user_*_vip"`

Both the user's **numeric Telegram ID** and their **username** are checked
against the allowlist. For example, if a user has ID `123456789` and username
`alice`, either `"123456789"` or `"alice"` in the allowlist grants access.

### OTP Self-Approval

When `dm_policy = "allowlist"` and `otp_self_approval = true` (the default),
unknown users who DM the bot receive a verification challenge:

1. User sends a DM to the bot
2. Bot responds with a challenge prompt (the 6-digit code is **not** shown to the user)
3. The code appears in the Moltis web UI under **Channels > Senders**
4. The bot owner shares the code with the user out-of-band
5. User replies with the 6-digit code
6. On success, the user is automatically added to the allowlist

After 3 failed attempts, the user is locked out for `otp_cooldown_secs` seconds
(default: 300). Codes expire after 5 minutes.

Set `otp_self_approval = false` if you want to manually approve every user from
the web UI.

## Streaming

By default (`stream_mode = "edit_in_place"`), the bot sends an initial message
after `stream_min_initial_chars` characters (default: 30) and then edits it
in place as tokens arrive, throttled to at most one edit every
`edit_throttle_ms` milliseconds (default: 300).

Set `stream_mode = "off"` to disable streaming and send the full response as a
single message.

When `stream_notify_on_complete = true`, the bot sends a short non-silent
message after streaming finishes. This can trigger a Telegram push notification
on the user's device (since silent edits don't always trigger notifications).

## Finding Your Telegram User ID

To find your numeric Telegram user ID (for the `allowlist`):

1. Message [@userinfobot](https://t.me/userinfobot) on Telegram
2. It replies with your user ID, first name, and username
3. Use the numeric ID as a string in your config: `allowlist = ["123456789"]`

Alternatively, use your Telegram username (without the `@`): `allowlist = ["your_username"]`.

## Web UI Setup

You can also configure Telegram through the web interface:

1. Open **Settings > Channels**
2. Click **Connect Telegram**
3. Enter an account ID (any alias) and your bot token
4. Adjust DM policy, mention mode, and allowlist as needed
5. Click **Connect**

The same form is available during onboarding when Telegram is in `channels.offered`.

## Troubleshooting

### Bot doesn't respond

- Verify the bot token is correct (ask @BotFather for `/token` if unsure)
- Check `dm_policy` — if set to `"allowlist"`, make sure your user ID or
  username is listed in `allowlist`
- An empty `allowlist` with `dm_policy = "allowlist"` blocks **all** DMs
- Check `group_policy` — if `"disabled"`, group messages are ignored
- Look at logs: `RUST_LOG=moltis_telegram=debug moltis`

### "allowed_users" doesn't work

The field name is `allowlist`, not `allowed_users`. If you're migrating from
OpenClaw, note that the field was renamed. Values must also be **strings**
(e.g. `["123456789"]`), not integers.

### Bot responds to everyone

- Check that `dm_policy = "allowlist"` is set (it's the default)
- Verify you have entries in `allowlist` — an empty allowlist with
  `dm_policy = "allowlist"` blocks everyone, but `dm_policy = "open"` allows
  everyone

### Bot doesn't respond in groups

- Check `mention_mode` — if set to `"mention"`, you must @mention the bot
- Check `group_policy` — if `"disabled"`, group messages are ignored
- Check `group_allowlist` — if non-empty, the group must be listed
