# Nostr

Moltis can receive and send encrypted direct messages over
[Nostr](https://nostr.com), the decentralized social protocol. The integration
uses [NIP-04](https://github.com/nostr-protocol/nips/blob/master/04.md)
encrypted DMs (kind:4) and connects to relays via `nostr-sdk` — no public URL
or server infrastructure is required. It can also join
[NIP-29](https://github.com/nostr-protocol/nips/blob/master/29.md) group chats,
including Block's [Buzz](https://github.com/block/buzz) channels — see
[Buzz & NIP-29 Groups](#buzz--nip-29-groups).

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
| `groups` | no | `[]` | NIP-29 group ids (`h` tags) to join — see [Buzz & NIP-29 Groups](#buzz--nip-29-groups). Empty = DM-only |
| `group_mention_mode` | no | `"mention"` | When to respond in groups: `"mention"` (p-tagged only), `"always"`, or `"none"` |
| `group_message_kind` | no | `"nip29"` | Dialect for bot-initiated group messages: `"nip29"` (kind:9) or `"buzz_v2"` (kind:40002) |
| `group_ack_reactions` | no | `true` | React 👀 on receipt and ✅/❌ on completion in groups (NIP-25) |
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

## Buzz & NIP-29 Groups

[Buzz](https://github.com/block/buzz) is Block's open-source team-chat and
agent-collaboration platform built on Nostr — a Slack/GitHub rival where AI
agents and humans are first-class members of shared "channels". Its primary API
is [NIP-29](https://github.com/nostr-protocol/nips/blob/master/29.md)
relay-based group chat over a connection authenticated with
[NIP-42](https://github.com/nostr-protocol/nips/blob/master/42.md).

Because Buzz speaks plain Nostr, Moltis participates as an ordinary NIP-29
client — the same code path works against Buzz relays and any other NIP-29
relay. A Buzz channel is a group identified by an `h` tag; messages are
`kind:9` (standard NIP-29) or `kind:40002` (Buzz) chat events — both are read,
see [Message Kinds](#message-kinds-kind9-vs-kind40002). Unlike DMs, group
messages are **not** encrypted — the relay enforces membership and
authorization.

### Configuration

```toml
[channels.nostr.my-bot]
secret_key = "nsec1..."
# Point at the Buzz relay (self-hosted or hosted). NIP-42 auth is automatic.
relays = ["wss://relay.example-buzz.dev"]
# The NIP-29 group ids (h tags) the bot joins. Empty keeps DM-only mode.
groups = ["buzz-general", "buzz-dev"]
group_mention_mode = "mention"  # mention | always | none
group_message_kind = "buzz_v2"  # nip29 (kind:9) | buzz_v2 (kind:40002)
group_ack_reactions = true      # 👀 on receipt, ✅/❌ on completion
```

### Live-Streaming Replies (Buzz only)

Buzz supports editing a published message (`kind:40003`,
`KIND_STREAM_MESSAGE_EDIT`). In a Buzz channel, Moltis publishes its reply as
soon as the first tokens arrive and then edits it in place as the rest stream
in, so the channel sees the answer forming rather than waiting for the whole
turn. Edits are throttled (~1/second) so a long answer does not become hundreds
of signed events in the relay's audit log.

Plain NIP-29 has no edit kind, so on a non-Buzz relay the reply is collected and
posted once, as before. DMs are unchanged (gift wrap has no edit path).

### Acknowledgement Reactions

With `group_ack_reactions` enabled (the default), the bot reacts 👀 to a message
it has picked up, swaps in a phase glyph as the agent works (🌐 browsing, 💻
shell, ✏️ editing, 🏗️ building, 🛫 deploying, 🛠️ any other tool, ⏳ stalled), and
finishes with ✅ or ❌ — the same acknowledgement pattern Moltis uses on Slack.

Reactions are NIP-25 `kind:7` events, so each swap publishes one and retracts
the previous with a NIP-09 deletion (`kind:5`). Relays honour deletions at their
discretion, so a superseded glyph may briefly linger. On a busy turn this is a
handful of extra events per message — set `group_ack_reactions = false` if your
relay charges per event or you would rather keep the channel quiet.

Reactions are only sent in groups: reacting to a gift-wrapped DM would expose
that a conversation happened.

Withdrawing the bot from a group stops new reactions, but it can still take back
one it already placed — otherwise its last 👀 would sit on someone's message
permanently. A retraction that fails to reach the relay is retried a few times,
and if it still does not land the next reaction on that message clears it, since
nothing else revisits the turn once it has finished.

### Message Kinds (kind:9 vs kind:40002)

Buzz defines two chat kinds and posts channel messages as **`kind:40002`**
(`KIND_STREAM_MESSAGE_V2`), while plain NIP-29 relays use **`kind:9`**. Per
Buzz's own architecture notes, a client filtering `kind:9` alone never receives
`kind:40002` messages and vice versa — so speaking only one dialect makes the
bot invisible in the other.

Moltis therefore **always reads both**, and **replies mirror the kind of the
message they answer**, which makes Buzz channels work without any configuration.
`group_message_kind` only decides bot-*initiated* messages in a group nothing
has been received from yet; once a message arrives, the group's observed dialect
is used. Set it to `buzz_v2` on a Buzz relay so even the first proactive message
lands correctly.

### How Responses Are Triggered

`group_mention_mode` controls when the bot replies in a group:

- **`mention`** (default) — respond only when the bot's pubkey is `p`-tagged
  (an `@`-mention). Best for busy channels.
- **`always`** — respond to every message in joined groups. Use for dedicated
  agent channels.
- **`none`** — receive-only. The bot never responds, and never publishes into
  the group at all: outbound sends are refused too, so a turn queued before you
  changed the setting cannot still speak in the channel.

Each group is a separate conversation — the group id is used as the session
key, so different Buzz channels keep independent context. A reply carries the
group's `h` tag, a NIP-10 `e` tag threading it to the message being answered,
and a `p` tag notifying that message's author. It is published with the same
kind as the message it answers.

### Turning Group Chat Off

Removing a group from `groups`, or setting `group_mention_mode = "none"`, stops
the bot publishing into that channel — not just answering in it. A reply target
is stored with the session and outlives the settings that created it, so a turn
queued or resumed from earlier would otherwise still speak in a channel you had
already withdrawn the bot from.

This is enforced at the publish itself, under the same lock the settings save
takes. Once your save returns, nothing further is published: a send already in
flight completes before the save applies, and any send afterwards is refused.
A long streamed reply stops mid-answer rather than finishing, and the partial
text already posted is withdrawn — it can never be completed, so leaving it
would strand half a sentence in the channel as the bot's last word. Taking the
bot's own content back out is not the same as speaking, so it is allowed after
revocation. NIP-09 deletion is only a request, though, so a relay that ignores
it will keep showing the fragment.

### Access Control

`groups` is the whole access model: it is both the set of groups the bot
subscribes to *and* the set it accepts messages from. There is deliberately no
"respond in any group" mode.

```admonish warning
A group's `h` tag is an unauthenticated label. An event's signature proves who
wrote it, not which group it belongs to, and a relay can push any event down
the socket — `nostr-sdk` does not verify that delivered events match the filters
you subscribed with. Moltis therefore re-checks every inbound group message
(`kind:9` and `kind:40002` alike) against `groups` and drops anything else, so
a hostile or buggy relay cannot inject text into the agent by inventing an `h`
tag. Keep `groups` to channels you actually intend the bot to work in.
```

Withdrawing the bot takes effect immediately, including mid-answer. Removing a
group from `groups` — or switching it to `none` — is re-checked before every
publish, so a reply that is still streaming stops where it is instead of running
to completion. The text already posted stays visible: a NIP-09 deletion is only
a request the relay may ignore, so it would promise more than it delivers.

Note that group chat has no sender allowlist: in NIP-29 the relay owns
membership, so anyone the relay admits to a joined channel can address the bot
there. This is the same trust model as a Slack channel, and a reason to prefer
a relay you host.

The exception is commands that control code execution. `/sh` (which turns
subsequent messages into shell commands) and `/sandbox` (which can move
execution off the sandbox onto the host) are refused in groups unless the
sender's key is in `allowed_pubkeys` — the same list that gates DMs, reused
here as the operator list. An empty `allowed_pubkeys` means nobody may run
them, so they fail closed. The refusal is posted in the channel rather than
ignored.

The same list authorizes the gateway's privileged commands (`/approve`,
`/deny`, `/update`) from both DMs and groups. Entries may be written in either
`npub1...` or hex form — they are compared as parsed keys, not as text.

### Setup

1. Generate the bot's key pair (see [Prerequisites](#prerequisites)) and share
   its `npub` with the Buzz workspace admin so they can invite/approve it.
2. Set `relays` to the Buzz relay URL and list the channel ids in `groups`.
3. Restart the account. Relay subscriptions are fixed at connect, so **changing
   `groups` requires an account restart** to take effect.

```admonish note
NIP-42 authentication is enabled for every Nostr account, so the bot
transparently authenticates to relays that require it (Buzz relays challenge
every connection before granting group read/write scopes).
```

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
