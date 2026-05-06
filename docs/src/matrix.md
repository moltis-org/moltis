# Matrix

Moltis can connect to Matrix as a bot account using a homeserver URL plus one
of three authentication methods: OIDC, password, or access token. The
integration runs as an outbound sync loop, so it does not require a public
webhook URL, port forwarding, or TLS termination on your side.

```admonish warning
Matrix encrypted chats require password auth.

Access-token auth can connect for plain Matrix traffic, but it reuses an
existing Matrix session without that device's local private encryption keys.
That means Moltis cannot reliably decrypt encrypted rooms when you connect with
an access token copied out of Element.

Use password auth so Moltis logs in as its own Matrix device and persists its
own E2EE keys locally. After that:

- If Element starts verification, Moltis accepts it automatically
- Moltis posts emoji confirmation instructions in the Matrix chat
- Reply `verify yes`, `verify no`, `verify show`, or `verify cancel` in Matrix
- Older encrypted history may remain unreadable until keys are shared

For dedicated bot accounts, the web UI now defaults to **Let Moltis own this
Matrix account**. In that mode Moltis bootstraps cross-signing and recovery for
the bot account so its own device can be verified automatically. If you want to
open the same bot account in Element yourself, switch the channel to
user-managed mode instead.
```

## Feature Set

Matrix is no longer a minimal transport. The current integration supports the
full day-to-day bot flow, including encrypted chats when you connect with
password auth.

| Area | Status | Notes |
|------|--------|-------|
| Web UI setup and editing | Supported | Add, edit, remove, and retry Matrix channels from **Settings -> Channels** |
| Direct messages and rooms | Supported | DM policy, room policy, allowlists, mention gating, and auto-join |
| End-to-end encrypted chats | Supported with password auth | Moltis creates and persists its own Matrix device and crypto state |
| Device verification | Supported | Moltis accepts Element verification and you confirm with `verify yes`, `verify no`, `verify show`, or `verify cancel` |
| Cross-signing / recovery ownership | Supported | Password auth defaults to `moltis_owned`; older accounts may require one browser approval before takeover |
| Streaming replies | Supported | Edit-in-place streaming for text responses |
| Thread-aware replies and context | Supported | Replies stay in threads and context fetch follows the thread root |
| Voice and audio messages | Supported | Matrix audio is downloaded and sent through the normal transcription pipeline |
| Interactive actions | Supported | Short action lists are sent as native Matrix polls |
| Reactions | Supported | Ack reactions and normal reaction flows work |
| Location | Supported | Inbound location shares update user location and outbound location sends are supported |
| OTP sender approval | Supported | Unknown DM senders can self-approve through the shared OTP flow |
| Model routing overrides | Supported | Per-room and per-user model/provider overrides |

The main remaining Matrix-specific limitations are:

- access-token auth is still plain-traffic-only, encrypted chats need password auth
- existing Matrix accounts with old crypto state may require one browser approval before Moltis can take over ownership
- Matrix interactive actions are poll-based, not arbitrary button/select UIs
- older encrypted history may remain unreadable until the missing room keys are shared with the Moltis device
- arbitrary remote-media fetch and reupload for outbound URLs is still limited

## How It Works

```
┌──────────────────────────────────────────────────────┐
│                 Matrix homeserver                     │
│          (/sync, send, relations, room APIs)         │
└──────────────────┬───────────────────────────────────┘
                   │  outbound HTTPS requests
                   ▼
┌──────────────────────────────────────────────────────┐
│                moltis-matrix crate                    │
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

## Authentication Modes

Moltis supports three ways to authenticate with a Matrix homeserver:

| Mode | When to use | Encrypted chats? |
|------|------------|------------------|
| **OIDC** (recommended) | Modern homeservers using Matrix Authentication Service (e.g. matrix.org since April 2025) | Yes |
| **Password** | Older homeservers without OIDC support | Yes |
| **Access token** | Quick testing or plain-traffic-only bots | No |

### OIDC Authentication

OIDC is the recommended authentication mode for homeservers that support it.
Moltis uses the matrix-sdk built-in OAuth 2.0 flow (MSC3861):

1. In the web UI, select **OIDC (recommended)** as the authentication mode
2. Enter the homeserver URL and click **Authenticate with OIDC**
3. A browser window opens for you to log in at the homeserver's identity provider
4. After successful authentication, the browser redirects back to Moltis
5. Moltis automatically registers a device, exchanges tokens, and starts syncing

OIDC sessions are refreshed automatically. Moltis persists the session tokens
to disk so the bot reconnects after restarts without re-authentication.

To configure OIDC via `moltis.toml`:

```toml
[channels.matrix.my-bot]
homeserver = "https://matrix.example.com"
auth_mode = "oidc"
```

Note: OIDC initial setup requires the web UI (browser-based flow). After the
first login, the saved session is reused for restarts.

## Prerequisites

Before configuring Moltis, you need a Matrix bot account:

1. Create or choose a Matrix account for the bot on your homeserver
2. For OIDC: just the homeserver URL (credentials are entered in the browser)
3. For password: keep the account password available
4. For access token: obtain a token from Element (plain traffic only)
5. Optionally pick a stable `device_id` for session restore

```admonish warning
Matrix credentials are secrets. Treat access tokens and passwords like
passwords, never commit them to version control. Moltis stores them with
`secrecy::Secret` and redacts them from logs and API responses.
```

## Getting an Access Token

If you want to use access-token auth, Element can show the token for the
currently logged-in account:

1. Sign into the dedicated Matrix bot account in Element
2. Open `Settings`
3. Open `Help & About`
4. Expand `Advanced`
5. Click `Access Token`

Use that token as `access_token` in the Matrix channel config.

```admonish warning
Access-token auth does **not** support encrypted Matrix chats.

Why: the token authenticates an already-existing Matrix device/session, but it
does not transfer that device's local private E2EE keys into Moltis. For
encrypted rooms, use `user_id` + `password` so Moltis creates and persists its
own Matrix device.
```

If you want encrypted Matrix chats, use password login instead. In that case,
set `user_id` and `password`.

## Why Password Auth Is Required For Encryption

For encrypted Matrix chats, Moltis must behave like its own Matrix device with
its own persistent crypto state.

Password auth works for that flow because Moltis logs in as a fresh Matrix
device, generates its own E2EE identity and one-time keys, stores that device
state locally, and can then complete normal Element verification.

Access-token auth is different. It authenticates an already-existing Matrix
session, often one created by Element, but Moltis does not receive that
existing device's private encryption keys just by knowing the token. That is
why access-token auth works for plain Matrix traffic but is not a reliable way
to support encrypted rooms.

Two related Matrix recovery tools often cause confusion:

- The Element recovery key helps a Matrix client unlock server-backed secret
  storage and backups.
- Exported room keys let another Matrix client import some historical Megolm
  room keys from a file.

Moltis does not currently implement recovery-key entry or room-key import, and
neither feature would make access-token auth equivalent to a proper dedicated
Moltis device anyway. The supported path for encrypted Matrix chats is still:

1. Add the Matrix account with `user_id` + `password`
2. Let Moltis create its own Matrix device
3. Complete Element verification with that Moltis device
4. Send new encrypted messages after verification

## Ownership Modes

When you add a password-based Matrix account, Moltis offers two ownership modes:

### `moltis_owned`

This is the default for dedicated bot accounts. Moltis tries to become the
owner of the account's Matrix crypto state so the bot can verify its own device
properly.

In this mode Moltis will:

- create or restore its own Matrix device
- bootstrap or recover secret storage and cross-signing when possible
- self-sign the Moltis device after takeover succeeds
- show encryption ownership status directly in **Settings -> Channels**

For fresh bot accounts, this is usually automatic.

For older accounts with pre-existing Matrix crypto state, Matrix may require one
browser approval before Moltis is allowed to reset and take over cross-signing.
When that happens, the Channels page shows:

- an **Open approval page for @user:server** button
- a retry button after you finish the reset in the browser

### `user_managed`

Use this when you want to manage the same bot account yourself in Element or
another Matrix client.

In this mode Moltis still connects and can chat, but it does not try to take
ownership of cross-signing or recovery. The Channels page shows the homeserver,
user ID, device ID, and device name you need if you want to log into that bot
account in your own Matrix app.

## Configuration

Matrix can be configured either:

- manually in `moltis.toml`
- through the web UI in **Settings -> Channels**

Web UI channel accounts are stored in the internal `channels` table in `data_dir()/moltis.db`. They are not written back into `moltis.toml`. If you need a Matrix setting that does not have a dedicated field yet, use the advanced JSON config editor in the channel form.

```admonish warning
Web-managed Matrix credentials are not currently wrapped by the Moltis vault.

Today, channel configs are stored as JSON in the internal `channels` table in
`data_dir()/moltis.db`. Matrix secrets are still handled as secrets in code,
redacted from logs, and redacted from API responses, but they are not yet
encrypted-at-rest by the vault layer.
```

Manual file configuration uses a `[channels.matrix.<account-id>]` section in `moltis.toml`.
If you want encrypted Matrix chats, use password auth:

```toml
[channels.matrix.my-bot]
homeserver = "https://matrix.example.com"
user_id = "@bot:example.com"
password = "correct horse battery staple"
ownership_mode = "moltis_owned"
device_display_name = "Moltis Matrix Bot"
```

If you only need plain, unencrypted Matrix traffic, access-token auth still
works:

```toml
[channels.matrix.my-bot]
homeserver = "https://matrix.example.com"
access_token = "syt_..."
user_id = "@bot:example.com"
```

To show Matrix in the channel picker, include `"matrix"` in `channels.offered`:

```toml
[channels]
offered = ["telegram", "discord", "slack", "matrix"]
```

After editing `channels.offered`, reload the web UI so it fetches the latest
picker list from `moltis.toml`.

### Configuration Fields

| Field | Required | Default | Description |
|-------|----------|---------|-------------|
| `homeserver` | **yes** | — | Base URL of the Matrix homeserver |
| `access_token` | no | — | Access token for the bot account, for plain and unencrypted Matrix traffic only |
| `password` | no | — | Password for the bot account, required for encrypted Matrix chats |
| `user_id` | no | — | Bot user ID, for example `@bot:example.com`, auto-detected via `whoami` when omitted |
| `device_id` | no | — | Optional device ID used for session restore |
| `device_display_name` | no | — | Optional device display name used for password-based logins |
| `ownership_mode` | no | `"user_managed"` | Who manages cross-signing and recovery: `"moltis_owned"` or `"user_managed"` |
| `dm_policy` | no | `"allowlist"` | Who can DM the bot: `"open"`, `"allowlist"`, or `"disabled"` |
| `room_policy` | no | `"allowlist"` | Which rooms can talk to the bot: `"open"`, `"allowlist"`, or `"disabled"` |
| `mention_mode` | no | `"mention"` | When the bot responds in rooms: `"always"`, `"mention"`, or `"none"` |
| `room_allowlist` | no | `[]` | Matrix room IDs or aliases allowed to interact with the bot |
| `user_allowlist` | no | `[]` | Matrix user IDs allowed to DM the bot |
| `auto_join` | no | `"always"` | Invite handling: `"always"`, `"allowlist"`, or `"off"` |
| `model` | no | — | Override the default model for this account |
| `model_provider` | no | — | Provider for the overridden model |
| `stream_mode` | no | `"edit_in_place"` | How streaming replies are sent: `"edit_in_place"` or `"off"` |
| `edit_throttle_ms` | no | `500` | Minimum milliseconds between edit-in-place streaming updates |
| `stream_min_initial_chars` | no | `30` | Minimum buffered characters before the first streamed send |
| `channel_overrides` | no | `{}` | Per-room model/provider overrides |
| `user_overrides` | no | `{}` | Per-user model/provider overrides |
| `reply_to_message` | no | `true` | Send threaded/rich replies when possible |
| `ack_reaction` | no | `"👀"` | Emoji reaction added while processing, omit to disable |
| `otp_self_approval` | no | `true` | Enable OTP self-approval for non-allowlisted DM users |
| `otp_cooldown_secs` | no | `300` | Cooldown in seconds after 3 failed OTP attempts |

### Web UI Notes

When you add Matrix through the web UI:

- the homeserver field defaults to `https://matrix.org`
- Moltis auto-generates the internal `account_id`
- the saved account lives in `data_dir()/moltis.db`, not in `moltis.toml`
- the web UI defaults to password auth because encrypted Matrix chats require it
- password-based channels default to **Let Moltis own this Matrix account**
- Matrix channels can be added, edited, removed, and retried entirely from **Settings -> Channels**
- access-token auth is for plain Matrix traffic only, because Moltis cannot import the existing device's private E2EE keys from an access token
- if you switch a channel to user-managed mode, the Channels page shows the homeserver, user ID, device ID, and device name you need to open that bot account in Element
- if an older Matrix account needs one external approval before takeover, the channel card shows an approval link plus a retry action so you do not need to rebuild the channel config manually
- if Element starts device verification, Moltis accepts it and posts emoji confirmation instructions in the room
- send `verify yes`, `verify no`, `verify show`, or `verify cancel` as normal messages in that same Matrix chat to finish or inspect the verification flow
- older encrypted history may still be unreadable if this Moltis device joined after those keys were created

If you want to inspect web-added channels directly, query the SQLite database:

```bash
sqlite3 ~/.moltis/moltis.db 'select channel_type, account_id, config from channels;'
```

If you use `MOLTIS_DATA_DIR` or `--data-dir`, check that directory instead of `~/.moltis`.

### Full Example

```toml
[channels]
offered = ["matrix"]

[channels.matrix.my-bot]
homeserver = "https://matrix.example.com"
user_id = "@bot:example.com"
password = "correct horse battery staple"
ownership_mode = "moltis_owned"
device_id = "MOLTISBOT"
device_display_name = "Moltis Matrix Bot"
dm_policy = "allowlist"
room_policy = "allowlist"
mention_mode = "mention"
room_allowlist = ["!ops:example.com", "#support:example.com"]
user_allowlist = ["@alice:example.com", "@bob:example.com"]
auto_join = "allowlist"
model = "gpt-4.1"
model_provider = "openai"
stream_mode = "edit_in_place"
edit_throttle_ms = 500
stream_min_initial_chars = 30
reply_to_message = true
ack_reaction = "👀"
otp_self_approval = true
otp_cooldown_secs = 300

[channels.matrix.my-bot.channel_overrides."!ops:example.com"]
model = "claude-sonnet-4-20250514"
model_provider = "anthropic"

[channels.matrix.my-bot.user_overrides."@alice:example.com"]
model = "o3"
model_provider = "openai"
```

## Access Control

Matrix uses the same gating model as the other channel integrations.

### DM Policy

| Value | Behavior |
|-------|----------|
| `"allowlist"` | Only users in `user_allowlist` can DM the bot (default) |
| `"open"` | Anyone can DM the bot |
| `"disabled"` | DMs are silently ignored |

### Room Policy

| Value | Behavior |
|-------|----------|
| `"allowlist"` | Only rooms in `room_allowlist` are allowed (default) |
| `"open"` | Any joined room can interact with the bot |
| `"disabled"` | Room messages are silently ignored |

### Mention Mode

| Value | Behavior |
|-------|----------|
| `"mention"` | Bot only responds when explicitly mentioned in a room (default) |
| `"always"` | Bot responds to every message in allowed rooms |
| `"none"` | Bot never responds in rooms |

When `mention_mode = "mention"`, Moltis checks Matrix intentional mentions
(`m.mentions`) and also falls back to a literal MXID mention in the plain body.

## Invite Handling

| Value | Behavior |
|-------|----------|
| `"always"` | Auto-join every invite (default) |
| `"allowlist"` | Auto-join only when the inviter is in `user_allowlist` or the room is already in `room_allowlist` |
| `"off"` | Never auto-join invites |

## Threads and Replies

Matrix replies now preserve thread context when the referenced event belongs to
an existing thread. When `reply_to_message = true`, Moltis sends a rich reply
and keeps the reply inside the thread when appropriate.

For thread context injection, Moltis resolves the inbound event to the thread
root and fetches prior `m.thread` relations so the LLM sees the room thread
history instead of just the last message.

## Voice and Location Messages

Matrix audio messages are downloaded through the homeserver media API and
transcribed with the same voice pipeline used by the other voice-enabled
channels. If voice transcription is not configured, Moltis replies with setup
guidance instead of silently dropping the message.

Inbound Matrix location shares now update the stored user location and also
resolve pending tool-triggered location requests. If there is no pending
location request, the coordinates are forwarded to the chat session so the LLM
can acknowledge them naturally.

## Interactive Actions

When Moltis needs to ask the user to choose from a short list of actions, the
Matrix integration sends a native poll instead of a plain text fallback. The
selected poll answer is fed back into the same interaction callback path used by
the other channel integrations.

Matrix poll answers are single-choice and capped by the protocol at 20 options.
If a generated interactive message exceeds that limit, Moltis falls back to a
plain numbered text list.

## OTP Self-Approval

When `dm_policy = "allowlist"` and `otp_self_approval = true`, unknown DM users
can self-approve:

1. User sends a DM to the bot
2. Moltis generates a 6-digit OTP challenge
3. The code appears in the web UI under **Channels > Senders**
4. The bot owner shares the code out-of-band
5. User replies with the code in Matrix
6. On success, the sender is approved

After 3 failed attempts, the sender is locked out for `otp_cooldown_secs`
seconds.

## Troubleshooting

### Bot does not connect

- Verify `homeserver` is correct and reachable
- Verify the access token or password is valid
- Set `user_id` explicitly if startup auto-detection is unreliable
- Look at logs: `RUST_LOG=moltis_matrix=debug moltis`

### Element shows the room as encrypted

- That is fine, encrypted rooms are supported
- Make sure the Matrix account was added with password auth, not access-token auth
- If the Moltis device is new, start a fresh Element verification with the bot
- Moltis will accept the request and post emoji instructions in the chat
- Send `verify yes` as a normal chat message if the emojis match, `verify no` if they do not
- If older encrypted history still does not decrypt, resend the message after verification

### Access-token auth connects but encrypted messages do not decrypt

- That is expected with the current implementation
- Access-token auth is for plain Matrix traffic only
- Remove the Matrix account from Moltis
- Re-add it with `user_id` + `password`
- Verify the new Moltis device in Element
- Resend a brand new encrypted message after verification

### Element says it is waiting for Moltis to accept verification

- Moltis should accept Matrix verification requests automatically
- Watch the chat for the emoji confirmation prompt
- If the prompt scrolled away, send `verify show` as a normal message in that same Matrix chat
- If an older stale verification request was replayed from sync history, start a fresh verification in Element and then use the `verify ...` commands
- If nothing happens, check the Matrix logs for verification events and try starting verification again

### Channels page says `Ownership approval required`

- This means Moltis connected, but Matrix wants one explicit browser approval before cross-signing can be reset for this older account
- Use the **Open approval page for @user:server** button in the Matrix channel card
- Make sure the browser page is signed into that exact Matrix account, not your personal one
- After approving the reset, use the retry button in the same channel card so Moltis can finish ownership bootstrap
- Until that finishes, the bot may still chat, but the device can remain `unverified`

### Bot can chat, but the Channels page still says `Device not yet verified by owner`

- Matrix encryption and Matrix cross-signing are related, but not identical
- A Matrix device can already send and receive messages before it is verified by the account owner
- If you are using `moltis_owned`, let Moltis finish ownership bootstrap or complete the approval flow above
- If you are using `user_managed`, verify the Moltis device from your own Matrix client instead

### Bot does not respond in rooms

- Check `room_policy`
- Check `room_allowlist`
- Check `mention_mode`, especially if it is `"mention"` or `"none"`
- Make sure the bot has joined the room, or enable `auto_join`

### Bot does not respond in DMs

- Check `dm_policy`
- Check `user_allowlist`
- If OTP is enabled, look in **Channels > Senders** for a pending challenge
