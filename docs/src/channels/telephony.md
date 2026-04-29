# Telephony (Phone Calls)

Moltis can make and receive phone calls, enabling voice-based AI conversations over the public telephone network (PSTN).

## Supported Providers

| Provider | Status | Features |
|----------|--------|----------|
| **Twilio** | Supported | Outbound calls, inbound calls, TTS, speech recognition, DTMF |

## Quick Start

### 1. Get a Twilio Account

1. Sign up at [twilio.com](https://www.twilio.com/console)
2. Get your **Account SID** and **Auth Token** from the dashboard
3. Buy or provision a phone number with Voice capability

### 2. Configure in moltis.toml

```toml
[channels.telephony.default]
provider = "twilio"
account_sid = "$TWILIO_ACCOUNT_SID"
auth_token = "$TWILIO_AUTH_TOKEN"
from_number = "+15551234567"        # Your Twilio phone number (E.164)
```

Or configure via the web UI: **Settings > Channels > Connect Phone Calls**.

### 3. Start the Gateway

```bash
moltis gateway
```

The telephony channel starts automatically with the gateway.

## Configuration Reference

```toml
[channels.telephony.<account-name>]
provider = "twilio"                    # Provider backend (currently only "twilio")
account_sid = "AC..."                  # Twilio Account SID
auth_token = "..."                     # Twilio Auth Token
from_number = "+15551234567"           # Outbound caller ID (E.164)
to_number = "+15559876543"             # Default destination (optional)

# Webhook settings
webhook_url = "https://your-domain.com"  # Public URL for Twilio callbacks
webhook_port = 3334                      # Webhook listener port

# Call settings
max_duration_secs = 3600               # Max call duration (default: 1 hour)
notify_hangup_delay_secs = 3           # Delay before hangup in notify mode

# Access control
inbound_policy = "disabled"            # disabled | allowlist | open
allowlist = ["+15559876543"]           # Allowed inbound callers (E.164)

# Voice settings
voice_id = "Polly.Joanna"             # TTS voice ID
tts_provider = "elevenlabs"           # Override TTS provider

# Agent routing
model = "claude-sonnet-4-20250514"                     # LLM model for conversations
model_provider = "anthropic"           # Model provider
agent_id = "main"                      # Agent ID for call handling
```

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
moltis voice-call status <call-id>
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
- **Credential storage**: Account SID and Auth Token are stored as secrets (never logged or exposed in API responses)
- **Max duration**: Calls are automatically terminated after the configured max duration

## Audio Pipeline

```
User Speech -> Twilio STT -> Text -> Agent (LLM) -> Text -> TTS -> mu-law 8kHz -> Caller
```

The telephony audio pipeline converts between PSTN-standard mu-law encoding (8 kHz, ITU-T G.711) and the PCM audio used by TTS providers.
