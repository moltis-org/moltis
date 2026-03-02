# WhatsApp Channel

Moltis supports WhatsApp as a messaging channel using the WhatsApp Linked Devices
protocol. Your WhatsApp account connects as a linked device (like WhatsApp Web),
so no separate phone number or WhatsApp Business API is needed — you pair your
existing personal or business WhatsApp by scanning a QR code.

## How It Works

```
┌────────────────┐   QR pair   ┌─────────────────┐   Signal   ┌──────────────┐
│  Your Phone    │ ──────────► │  Moltis Gateway  │ ◄────────► │  WhatsApp    │
│  (WhatsApp)    │             │  (linked device)  │            │  Servers     │
└────────────────┘             └─────────────────┘            └──────────────┘
                                       │
                                       ▼
                               ┌─────────────────┐
                               │  LLM Provider    │
                               │  (Claude, GPT…)  │
                               └─────────────────┘
```

1. Moltis registers as a **linked device** on your WhatsApp account
2. Messages sent to your WhatsApp number arrive at both your phone and Moltis
3. Moltis processes inbound messages through the configured LLM
4. The LLM reply is sent back through your WhatsApp account

```admonish info title="Dedicated vs Personal Number"
**Dedicated number (recommended):** Use a separate phone with its own WhatsApp
account. All messages to that number go to the bot. Clean separation, no
accidental replies to personal contacts.

**Personal number (self-chat):** Use your own WhatsApp account and message
yourself via WhatsApp's "Message Yourself" feature. Moltis automatically
detects self-chat and prevents reply loops. Convenient for personal use.
Note that Moltis (as a linked device) sees all your incoming messages —
whether it *responds* is governed by access control (see below).
```

## Feature Flag

WhatsApp is behind the `whatsapp` cargo feature, enabled by default:

```toml
# crates/cli/Cargo.toml
[features]
default = ["whatsapp", ...]
whatsapp = ["moltis-gateway/whatsapp"]
```

When disabled, all WhatsApp code is compiled out — no QR code library, no
Signal Protocol store, no WhatsApp event handlers.

## Quick Start (Web UI)

The fastest way to connect WhatsApp:

1. Start Moltis: `moltis serve`
2. Open the web UI and navigate to **Settings > Channels**
3. Click **+ Add Channel** > **WhatsApp**
4. Enter an **Account ID** (any name you like, e.g. `my-whatsapp`)
5. Choose a **DM Policy** (Open, Allowlist, or Disabled)
6. Optionally select a default **Model**
7. Click **Start Pairing** — a QR code appears
8. On your phone: **WhatsApp > Settings > Linked Devices > Link a Device**
9. Scan the QR code
10. The modal shows "Connected" with your phone's display name

That's it — messages to your WhatsApp account are now processed by Moltis.

```admonish tip
The QR code refreshes automatically every ~20 seconds. If it expires before
you scan it, a new one appears without any action needed.
```

## Quick Start (Config File)

You can also configure WhatsApp accounts in `moltis.toml`. This is useful for
automated deployments or when you want to pre-configure settings before pairing.

```toml
# ~/.moltis/moltis.toml

[channels.whatsapp."my-whatsapp"]
dm_policy = "open"
model = "anthropic/claude-sonnet-4-20250514"
model_provider = "anthropic"
```

Start Moltis and the account will begin the pairing process. The QR code is
printed to the terminal and also available via the web UI. Once paired, the
config file is updated with:

```toml
[channels.whatsapp."my-whatsapp"]
paired = true
display_name = "John's iPhone"
phone_number = "+15551234567"
dm_policy = "open"
model = "anthropic/claude-sonnet-4-20250514"
model_provider = "anthropic"
```

## Configuration Reference

Each WhatsApp account is a named entry under `[channels.whatsapp]`:

```toml
[channels.whatsapp."<account-id>"]
```

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `paired` | bool | `false` | Whether QR code pairing is complete (auto-set) |
| `display_name` | string | — | Phone name after pairing (auto-populated) |
| `phone_number` | string | — | Phone number after pairing (auto-populated) |
| `store_path` | string | — | Custom path to sled store; defaults to `~/.moltis/whatsapp/<account_id>/` |
| `model` | string | — | Default LLM model ID for this account |
| `model_provider` | string | — | Provider name for the model |
| `dm_policy` | string | `"open"` | DM access policy: `"open"`, `"allowlist"`, or `"disabled"` |
| `group_policy` | string | `"open"` | Group access policy: `"open"`, `"allowlist"`, or `"disabled"` |
| `allowlist` | array | `[]` | Users allowed to DM (usernames or phone numbers) |
| `group_allowlist` | array | `[]` | Group JIDs allowed for bot responses |
| `otp_self_approval` | bool | `true` | Allow non-allowlisted users to self-approve via OTP |
| `otp_cooldown_secs` | int | `300` | Cooldown seconds after 3 failed OTP attempts |

### Full Example

```toml
[channels.whatsapp."personal"]
paired = true
display_name = "John's iPhone"
phone_number = "+15551234567"
model = "anthropic/claude-sonnet-4-20250514"
model_provider = "anthropic"
dm_policy = "allowlist"
allowlist = ["alice", "bob", "+15559876543"]
group_policy = "disabled"
otp_self_approval = true
otp_cooldown_secs = 300

[channels.whatsapp."work-bot"]
paired = true
dm_policy = "open"
group_policy = "allowlist"
group_allowlist = ["120363456789@g.us"]
model = "openai/gpt-4.1"
model_provider = "openai"
```

## Access Control

WhatsApp uses the same access control model as Telegram channels.

### DM Policies

| Policy | Behavior |
|--------|----------|
| `open` | Anyone who messages your WhatsApp can chat with the bot |
| `allowlist` | Only users on the allowlist get responses; others get an OTP challenge |
| `disabled` | All DMs are silently ignored |

### Group Policies

| Policy | Behavior |
|--------|----------|
| `open` | Bot responds in all groups it's part of |
| `allowlist` | Bot only responds in groups on the `group_allowlist` |
| `disabled` | Bot ignores all group messages |

### OTP Self-Approval

When `dm_policy = "allowlist"` and `otp_self_approval = true` (the default),
users not on the allowlist can request access:

1. User sends any message to the bot
2. Bot replies: *"Please reply with the 6-digit code to verify access"*
3. The OTP code appears in the **Senders** tab of the web UI
4. User replies with the code
5. If correct, user is permanently added to the allowlist

After 3 wrong attempts, a cooldown period kicks in (default 5 minutes).
You can also approve or deny users directly from the Senders tab without
waiting for OTP verification.

```admonish tip
Set `otp_self_approval = false` if you want to manually approve every new
user from the web UI instead of letting them self-approve.
```

### Using Your Personal Number Safely

When Moltis is linked to your personal WhatsApp, it sees **every** incoming
message — from friends, family, groups, everyone. The key question is: who
does the bot *respond* to?

**Self-chat always works.** Messages you send to yourself (via "Message
Yourself") bypass access control entirely. You are the account owner, so
you're always authorized regardless of `dm_policy` settings.

**Other people's messages follow `dm_policy`.** If you want Moltis to only
respond to your self-chat and ignore everyone else:

```toml
[channels.whatsapp."personal"]
dm_policy = "disabled"    # Ignore all DMs from other people
group_policy = "disabled" # Ignore all group messages
```

This is the safest configuration for personal use — the bot only responds
when you message yourself.

If you want to selectively allow certain contacts:

```toml
[channels.whatsapp."personal"]
dm_policy = "allowlist"
allowlist = ["alice", "bob"]  # Only these people get bot responses
group_policy = "disabled"
```

```admonish warning title="Default is Open"
The default `dm_policy` is `"open"`, which means **everyone** who messages
your WhatsApp will get a bot response. If you're using your personal number,
change this to `"disabled"` or `"allowlist"` before pairing.
```

## Session Persistence

WhatsApp uses the Signal Protocol for end-to-end encryption. The encryption
keys and session state are stored in a **sled database** at:

```
~/.moltis/whatsapp/<account_id>/
```

This means:

- **No re-pairing after restart** — the linked device session survives process
  restarts, server reboots, and upgrades
- **One store per account** — multiple WhatsApp accounts each get their own
  isolated database
- **Custom path** — set `store_path` in config to use a different location
  (useful for Docker volumes or shared storage)

```admonish warning
Do not delete the sled store directory while Moltis is running. If you need
to re-pair, stop Moltis first, then delete the directory and restart.
```

## Self-Chat

Moltis automatically supports WhatsApp's "Message Yourself" feature. When you
send a message to yourself, the bot processes it as a regular inbound message
and replies in the same chat.

This is useful for:
- **Personal assistant** — chat with your AI without a dedicated phone number
- **Testing** — verify the bot works before sharing with others
- **Quick notes** — send yourself reminders that the AI processes

### Loop Prevention

When the bot replies to your self-chat, WhatsApp delivers that reply back as
an incoming message (since it's your own chat). Moltis uses two mechanisms to
prevent infinite reply loops:

1. **Message ID tracking**: Every message the bot sends is recorded in a
   bounded ring buffer (256 entries). Incoming `is_from_me` messages whose
   ID matches a tracked send are recognized as bot echoes and skipped.

2. **Invisible watermark**: An invisible Unicode sequence (zero-width joiners)
   is appended to every bot-sent text message. If an incoming message contains
   this watermark, it's recognized as a bot echo even if the message ID wasn't
   tracked (e.g. after a restart).

Both checks are automatic — no configuration needed.

## Media Handling

WhatsApp supports rich media messages. Moltis handles each type:

| Message Type | Handling |
|--------------|----------|
| **Text** | Dispatched directly to the LLM |
| **Image** | Downloaded, optimized for LLM consumption (resized if needed), sent as attachment |
| **Voice** | Downloaded and transcribed via STT (if configured); falls back to text guidance |
| **Audio** | Same as voice, but classified separately (non-PTT audio files) |
| **Video** | Thumbnail extracted and sent as image attachment with caption |
| **Document** | Caption and filename/MIME metadata dispatched as text |
| **Location** | Resolves pending location tool requests, or dispatches coordinates to LLM |

```admonish info title="Voice Transcription"
Voice message transcription requires an STT provider to be configured.
See [Voice Services](voice.md) for setup instructions. Without STT,
the bot replies asking the user to send a text message instead.
```

## Managing Channels in the Web UI

### Channels Tab

The Channels page shows all connected accounts across all channel types
(Telegram, WhatsApp). Each card displays:

- **Status badge**: `connected`, `pairing`, or `disconnected`
- **Display name**: Phone name from WhatsApp (after pairing)
- **Sender summary**: List of recent senders with message counts
- **Edit / Remove** buttons

### Adding a WhatsApp Channel

1. Click **+ Add Channel** > **WhatsApp**
2. Fill in the account ID, DM policy, and optional model
3. Click **Start Pairing**
4. Scan the QR code on your phone
5. Wait for the "Connected" confirmation

### Editing a Channel

Click **Edit** on a channel card to modify:
- DM and group policies
- Allowlist entries
- Default model

Changes take effect immediately — no restart needed.

### Senders Tab

Switch to the **Senders** tab to see everyone who has messaged the bot:

- Filter by account using the dropdown
- See message counts, last activity, and access status
- **Approve** or **Deny** users directly
- View pending OTP challenges with the code displayed

## Troubleshooting

### QR Code Not Appearing

- Ensure the `whatsapp` feature is enabled (it is by default)
- Check terminal output for errors — the QR code is also printed to stdout
- Verify the sled store directory is writable: `~/.moltis/whatsapp/`

### "Logged Out" After Restart

- This usually means the sled store was corrupted or deleted
- Check that `~/.moltis/whatsapp/<account_id>/` exists and has data files
- Re-pair by removing the directory and restarting: the pairing flow starts again

### Bot Not Responding to Messages

- Check `dm_policy` — if set to `allowlist`, only listed users get responses
- Check `group_policy` — if set to `disabled`, group messages are ignored
- Look at the **Senders** tab to see if the user is denied or pending OTP
- Check terminal logs for access control decisions

### Self-Chat Not Working

- WhatsApp's "Message Yourself" chat must be used (not a group with only yourself)
- The bot needs to be connected and the account paired
- If you just restarted, the watermark-based detection handles messages that
  arrive before the message ID buffer is rebuilt

## Code Structure

```
crates/whatsapp/
├── src/
│   ├── lib.rs           # Crate entry, WhatsAppPlugin
│   ├── config.rs        # WhatsAppAccountConfig
│   ├── connection.rs    # Bot startup, sled store, event loop
│   ├── handlers.rs      # Event routing, message handling, media
│   ├── outbound.rs      # WhatsAppOutbound (ChannelOutbound impl)
│   ├── state.rs         # AccountState, loop detection, watermark
│   ├── access.rs        # DM/group access control
│   ├── otp.rs           # OTP challenge/verification
│   ├── plugin.rs        # ChannelPlugin trait impl
│   ├── sled_store.rs    # Persistent Signal Protocol store
│   └── memory_store.rs  # In-memory store (tests/fallback)
```
