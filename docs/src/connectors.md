# Connectors

Connectors copy bounded snapshots of external data into local, durable datasets
in `<data_dir>/connectors.db`. They share account, dataset, run, projection, and
atomic snapshot machinery, while each provider owns its item schema and may
expose a provider-specific read tool. Trusted agents can search synchronized
data without contacting the remote service. Optional JSONL and Markdown
projections are written under `<data_dir>/connectors/exports/`.

## Email

Moltis can synchronize bounded, read-only email datasets directly from Gmail or
through an existing Himalaya v2 account. Credentials stay with the existing
Google Workspace or Himalaya configuration; they are not copied into the
connector database. Email uses provider-owned schemas and the trusted-only
`gmail_connector` and `himalaya_connector` tools rather than the generic
`connectors` tool.

See [Email connectors](email-connectors.md) for setup, provider limits, Himalaya
v2 commands, storage, and security details.

## Channel message history

Moltis can reuse existing **Slack**, **Discord**, **Matrix**, and **Microsoft
Teams** channel accounts to synchronize message history. No channel tokens,
passwords, or account configuration are copied into the connector database. A
channel connector stores only the channel type and existing account ID, then
uses the same authenticated Rust client and typed history API as normal channel
thread-context fetching.

To create one:

1. Configure and start the Slack, Discord, Matrix, or Teams account in
   **Settings -> Channels**.
2. Open **Settings -> Connectors -> Connections** and select **Add channel
   history connection**.
3. Choose the existing channel account. The connection test verifies that the
   account is active and exposes reusable history.
4. Create a dataset with the provider's conversation/channel ID and thread/root
   message ID. Select a message limit from 1 to 200 and an optional refresh
   schedule.

Each successful synchronization atomically replaces the selected bounded
window. Messages use their native provider ID for deduplication: Slack message
timestamps, Discord message IDs, Matrix event IDs, or Teams Graph message IDs.
Failed or interrupted fetches leave the previous valid snapshot unchanged.

Provider APIs currently bound each fetch:

| Channel | Current reusable history window |
|---------|---------------------------------|
| Slack | One `conversations.replies` response, up to 200 messages |
| Discord | Up to 100 messages from the thread channel |
| Matrix | The root plus one forward thread-relations page |
| Microsoft Teams | One configured Graph chat-history response |

This is deliberately not described as complete workspace or room history.
Provider pagination, edited/deleted-message reconciliation, and conversation
discovery are not yet part of the shared channel API.

Telegram, WhatsApp, Signal, Nostr, and telephony are not offered as remote
history connectors. Their current Moltis integrations expose live events or
sending APIs, not a reusable historical-read API. Moltis does not duplicate
their protocol clients or silently substitute the incomplete local forensic
message log.

## Security and retention

Channel messages are untrusted external content and may contain prompt
injection. The `connectors` agent tool labels results accordingly and remains
trusted-only by default. Connector datasets are operator-wide; only select
conversations whose participants and content are appropriate for every trusted
operator and agent allowed to read the dataset.

Message bodies are stored in plaintext in `connectors.db` and any enabled
projection files. Removing the source channel account prevents future fetches
but does not automatically delete the connector dataset. Remove the dataset or
connector connection when its retained data is no longer needed.

For CalDAV-specific setup, planning, filtering, and credential behavior, see
[CalDAV](caldav.md).
