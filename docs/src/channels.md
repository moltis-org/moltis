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

See the web UI's **Channels** tab for guided setup with each platform.
