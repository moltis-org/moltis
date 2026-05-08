# Signal

Moltis can receive and send Signal messages through an external
[`signal-cli`](https://github.com/AsamK/signal-cli) daemon. The Moltis process
talks to the daemon over local HTTP JSON-RPC for outbound messages and Server
Sent Events for inbound messages.

## How It Works

```
Signal Network
      │
      ▼
signal-cli daemon  ── HTTP JSON-RPC + SSE ──  moltis-signal
      │                                      │
      └──────── linked Signal account ──────┴── Moltis Gateway
```

Moltis does not embed libsignal directly. The Signal account, device link, and
Signal protocol state stay inside signal-cli, which keeps the Moltis integration
smaller and avoids coupling releases to Signal's native protocol internals.

## Prerequisites

Install and configure signal-cli first:

```bash
signal-cli -u +15551234567 register
signal-cli -u +15551234567 verify 123456
signal-cli --account +15551234567 daemon --http 127.0.0.1:8080
```

You can also use signal-cli's linked-device flow instead of registering a new
number. Keep the HTTP daemon reachable only from trusted local services.

## Configuration

Add a `[channels.signal.<account-id>]` section to `moltis.toml`:

```toml
[channels.signal.personal]
account = "+15551234567"
http_url = "http://127.0.0.1:8080"
dm_policy = "allowlist"
allowlist = ["+15557654321", "550e8400-e29b-41d4-a716-446655440000"]
group_policy = "disabled"
mention_mode = "mention"
otp_self_approval = true
otp_cooldown_secs = 300
text_chunk_limit = 4000
```

Make sure `"signal"` is included in `channels.offered` if you customize that
list. It is included by default.

## Fields

| Field | Required | Default | Description |
|-------|----------|---------|-------------|
| `account` | no | account ID | Signal account loaded in signal-cli, usually an E.164 phone number |
| `account_uuid` | no | not set | Optional Signal account UUID for allowlist matching |
| `http_url` | no | `http://127.0.0.1:8080` | signal-cli daemon HTTP URL |
| `dm_policy` | no | `allowlist` | `open`, `allowlist`, or `disabled` |
| `allowlist` | no | `[]` | Allowed sender phone numbers, UUIDs, or normalized identifiers |
| `group_policy` | no | `disabled` | `open`, `allowlist`, or `disabled` |
| `group_allowlist` | no | `[]` | Allowed Signal group IDs |
| `mention_mode` | no | `mention` | `mention`, `always`, or `none` |
| `ignore_stories` | no | `true` | Ignore story events from signal-cli |
| `otp_self_approval` | no | `true` | Let unknown DM senders self-approve with a PIN challenge |
| `otp_cooldown_secs` | no | `300` | Cooldown after 3 failed OTP attempts |
| `text_chunk_limit` | no | `4000` | Maximum UTF-8 bytes per outbound text chunk |

## Current Limits

Signal support currently handles text DMs and group text messages. Outbound
media uses the text fallback, and inbound attachments are surfaced as an
attachment placeholder until attachment ingestion is added.
