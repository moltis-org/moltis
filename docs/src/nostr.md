# Nostr

Moltis can receive and send encrypted direct messages over
[Nostr](https://nostr.com), the decentralized social protocol. The integration
uses [NIP-04](https://github.com/nostr-protocol/nips/blob/master/04.md)
encrypted DMs (kind:4) and connects to relays via `nostr-sdk` — no public URL
or server infrastructure is required.

## How It Works

```
┌──────────────────────────────────────────────────────┐
│              Nostr Relay Network                      │
│   (relay.damus.io, nos.lol, relay.nostr.band, ...)   │
└──────────────────┬───────────────────────────────────┘
                   │  WebSocket subscription (kind:4)
                   ▼
┌──────────────────────────────────────────────────────┐
│                moltis-nostr crate                     │
│  ┌────────────┐  ┌────────────┐  ┌────────────────┐  │
│  │    Bus     │  │  Outbound  │  │     Plugin     │  │
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

The bot connects **outward** to Nostr relays via WebSocket. No port forwarding,
public domain, or TLS certificate is needed. Messages are end-to-end encrypted
between the sender and the bot using NIP-04.

## Prerequisites

Before configuring Moltis, you need a Nostr secret key:

1. Generate a new key pair using any Nostr client (e.g.
   [Damus](https://damus.io), [Amethyst](https://github.com/vitorpamplona/amethyst),
   or a key generation tool)
2. Copy the secret key — either the `nsec1...` bech32 format or the 64-character
   hex format
3. Note the corresponding public key (`npub1...`) to share with users who want to
   message the bot

```admonish warning
The secret key is highly sensitive — it controls the bot's Nostr identity.
Never commit it to version control. Moltis stores it with `secrecy::Secret` and
redacts it from logs, but your `moltis.toml` file is plain text on disk.
Consider using [Vault](vault.md) for encryption at rest.
```

## Configuration

Add a `[channels.nostr.<account-id>]` section to your `moltis.toml`:

```toml
[channels.nostr.my-bot]
secret_key = "nsec1..."
relays = ["wss://relay.damus.io", "wss://relay.nostr.band", "wss://nos.lol"]
dm_policy = "allowlist"
allowed_pubkeys = ["npub1abc...", "npub1def..."]
```

Make sure `"nostr"` is included in `channels.offered` (it is by default):

```toml
[channels]
offered = ["telegram", "discord", "slack", "matrix", "nostr"]
```

### Configuration Fields

| Field | Required | Default | Description |
|-------|----------|---------|-------------|
| `secret_key` | **yes** | — | Nostr secret key (`nsec1...` bech32 or 64-char hex) |
| `relays` | no | `["wss://relay.damus.io", "wss://relay.nostr.band", "wss://nos.lol"]` | Relay WebSocket URLs to connect to |
| `dm_policy` | no | `"allowlist"` | Who can DM the bot: `"open"`, `"allowlist"`, or `"disabled"` |
| `allowed_pubkeys` | no | `[]` | Public keys allowed to DM (`npub1...` or hex, when `dm_policy = "allowlist"`) |
| `enabled` | no | `true` | Whether this account is active |
| `model` | no | — | Override the default model for this channel |
| `model_provider` | no | — | Provider for the overridden model |
| `otp_self_approval` | no | `true` | Allow non-allowlisted senders to self-approve via OTP code |
| `otp_cooldown_secs` | no | `300` | Cooldown after 3 failed OTP attempts |
| `profile.name` | no | — | NIP-01 profile display name |
| `profile.display_name` | no | — | NIP-01 longer display name |
| `profile.about` | no | — | NIP-01 bio / about text |
| `profile.picture` | no | — | NIP-01 avatar URL (HTTPS) |
| `profile.nip05` | no | — | NIP-05 identifier (e.g. `bot@example.com`) |

### Full Example

```toml
[channels.nostr.my-bot]
secret_key = "nsec1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqspcgef"
relays = [
  "wss://relay.damus.io",
  "wss://relay.nostr.band",
  "wss://nos.lol",
]
dm_policy = "allowlist"
allowed_pubkeys = [
  "npub1abc123...",
]
model = "anthropic/claude-sonnet-4-20250514"
model_provider = "anthropic"
otp_self_approval = true
otp_cooldown_secs = 300

[channels.nostr.my-bot.profile]
name = "Moltis Bot"
about = "AI assistant on Nostr"
nip05 = "bot@example.com"
```

## Access Control

### DM Policy

- **`allowlist`** (default) — Only public keys in `allowed_pubkeys` can message
  the bot. Unknown senders receive an OTP challenge if `otp_self_approval` is
  enabled, or are silently ignored.
- **`open`** — Anyone can DM the bot.
- **`disabled`** — All inbound DMs are ignored.

### OTP Self-Approval

When `otp_self_approval` is enabled and a non-allowlisted sender messages the
bot, the sender appears in the Senders tab of the web UI where they can be
approved or denied. This works the same as OTP for Telegram and Matrix.

## NIP-04 Encryption

All messages between the bot and users are encrypted using
[NIP-04](https://github.com/nostr-protocol/nips/blob/master/04.md) (kind:4
events). The bot can also decrypt inbound NIP-04 messages from any client that
supports this standard.

NIP-44 and NIP-17 (gift-wrapped DMs) are planned for future releases.

## Relay Health

The bot maintains persistent WebSocket connections to all configured relays.
The health probe reports the number of connected relays (e.g. "2/3 relays
connected"). If all relays disconnect, the bot will automatically attempt to
reconnect via `nostr-sdk`'s built-in reconnection logic.
