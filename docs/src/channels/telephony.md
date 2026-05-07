# Telephony (Phone Calls)

Moltis can make and receive phone calls, enabling voice-based AI conversations over the public telephone network (PSTN).

## Supported Providers

| Provider | Status | Features |
|----------|--------|----------|
| **Twilio** | Supported | Outbound calls, inbound calls, TTS, speech recognition, DTMF |
| **Telnyx** | Supported | Outbound calls, inbound calls |
| **Plivo** | Supported | Outbound calls, inbound calls |

## Quick Start

### 1. Get a Twilio Account

1. Sign up at [twilio.com](https://www.twilio.com/console)
2. Get your **Account SID** and **Auth Token** from the dashboard
3. Buy or provision a phone number with Voice capability

### 2. Configure Phone Settings

Open the web UI and go to **Settings > Phone**. Choose a provider, save its credentials, set the
caller phone number, and configure a public HTTPS webhook URL.

For TOML-managed settings, keep phone configuration under `[phone]`. Credentials are best stored
through **Settings > Phone** so they live in the credential store instead of plain TOML:

```toml
[phone]
enabled = true
provider = "twilio"
inbound_policy = "allowlist"
allowlist = ["+15559876543"]

[phone.twilio]
from_number = "+15551234567"        # Your Twilio phone number (E.164)
webhook_url = "https://your-domain.com"
```

### 3. Start the Gateway

```bash
moltis gateway
```

The phone integration starts automatically with the gateway when `[phone]` is enabled and the active
provider has complete credentials.

## Configuration Reference

```toml
[phone]
enabled = true                         # Enable phone calls globally
provider = "twilio"                    # twilio | telnyx | plivo
inbound_policy = "disabled"            # disabled | allowlist | open
allowlist = ["+15559876543"]           # Allowed inbound callers (E.164)
max_duration_secs = 3600               # Max call duration (default: 1 hour)

[phone.twilio]
from_number = "+15551234567"            # Outbound caller ID (E.164)
webhook_url = "https://your-domain.com" # Public URL for provider callbacks

[phone.telnyx]
from_number = "+15551234567"
webhook_url = "https://your-domain.com"

[phone.plivo]
from_number = "+15551234567"
webhook_url = "https://your-domain.com"
```

The gateway still runs telephony through its internal channel plugin, but phone setup is deliberately
separate from normal channel accounts in the UI.

## Call Modes

### Conversation Mode (default)
Full multi-turn interaction. The agent listens for speech, processes it through the LLM, and responds with TTS. The call continues until the user or agent hangs up, or the max duration is reached.

### Notify Mode
One-way message delivery. The agent speaks a message and hangs up after a short delay. Useful for alerts, reminders, and notifications.

## Agent Tool

Agents can make calls using the built-in `voice_call` tool:

```json
{
  "action": "initiate_call",
  "to": "+15559876543",
  "message": "Hello, this is a reminder about your appointment.",
  "mode": "notify"
}
```

Available actions:
- `initiate_call` - Start an outbound call
- `end_call` - Hang up an active call
- `get_status` - Check call state and transcript
- `send_dtmf` - Send touch-tone digits

## CLI Commands

```bash
moltis voice-call call --to +15559876543 --message "Hello"
moltis voice-call status [call-id]
moltis voice-call end <call-id>
moltis voice-call setup
```

## RPC Methods

| Method | Scope | Description |
|--------|-------|-------------|
| `voicecall.status` | read | List telephony accounts and active calls |
| `voicecall.initiate` | write | Start an outbound call |
| `voicecall.end` | write | Hang up a call |

## Webhook Endpoints

When configured with a public `webhook_url`, the gateway exposes:

| Endpoint | Purpose |
|----------|---------|
| `POST /api/channels/telephony/{account}/status` | Call status callbacks |
| `POST /api/channels/telephony/{account}/answer` | TwiML for answered calls |
| `POST /api/channels/telephony/{account}/gather` | Speech/DTMF result handler |

Configure these in your Twilio phone number settings, or they are set automatically when initiating outbound calls.

## Security

- **Webhook verification**: Twilio webhooks are verified using HMAC-SHA1 signature validation
- **Inbound access control**: Phone numbers can be restricted via allowlist
- **Credential storage**: Provider credentials are stored in the credential store when configured from Settings > Phone
- **Max duration**: Calls are automatically terminated after the configured max duration

## Audio Pipeline

```
User Speech -> Twilio STT -> Text -> Agent (LLM) -> Text -> TTS -> mu-law 8kHz -> Caller
```

The telephony audio pipeline converts between PSTN-standard mu-law encoding (8 kHz, ITU-T G.711) and the PCM audio used by TTS providers.
