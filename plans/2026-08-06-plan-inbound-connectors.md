# Inbound Connectors Plan

## Summary

Build connectors as a separate ingestion subsystem alongside Channels, not as
another channel type.

- Channels answer: "How can someone talk to the assistant?"
- Connectors answer: "What external data should Moltis synchronize locally?"

The two systems can share provider clients, authentication, webhook
infrastructure, secret handling, and UI patterns. They should not share the
conversational lifecycle because channels are tied to sessions, agent runs,
replies, typing indicators, and sender access control.

The first end-to-end implementation targets CalDAV, covering web-managed
authentication, calendar discovery, and event-resource synchronization. SQLite
is the canonical store, with JSON and Markdown generated as optional
projections. iCalendar URL feeds can be added later as a simpler read-only
driver.

## Domain Model

Separate four concepts:

| Concept | Example |
| --- | --- |
| Connection | My CalDAV account |
| Dataset | All calendars or a selected set of calendars |
| Sync | One execution that updates the dataset |
| Projection | SQLite, JSON, or Markdown representation |

A connection can own multiple datasets. For example, one CalDAV connection
could provide:

- All calendars and events.
- Events from selected calendars within a configured time horizon.

Credentials, schedules, selectors, and output formats therefore remain
independent rather than becoming one large connector configuration.

## High-Level Architecture

```text
CalDAV / Slack / Google
          |
    ConnectorDriver
          |
     Sync engine
          |
  Canonical connectors.db
          |
   +------+------+
   |             |
 JSON export   Markdown export
   |             |
 static UI     memory/agent use
```

Introduce:

- `moltis-connectors`: accounts, datasets, sync engine, persistence,
  scheduling, and projections.
- `moltis-connector-caldav`: CalDAV discovery and synchronization driver.
- A gateway `ConnectorManager`: registry, startup, cancellation, health, and
  RPC integration.
- Settings -> Integrations -> Connectors.
- RPC namespace `connectors.*`.

The first web-managed slice uses authenticated RPC. A REST namespace and live
progress events can be added when non-UI consumers require them.

Use compiled-in drivers and a typed `ConnectorKind` initially. Dynamic
third-party plugins add substantial complexity without helping prove the first
implementations.

## Persistence

Use `<data_dir>/connectors.db` as the canonical store. Connector data can become
much larger than regular Moltis configuration, so it should not grow
`moltis.db` indefinitely.

Core tables:

```text
connector_accounts
connector_datasets
connector_items
connector_sync_partitions
connector_sync_runs
connector_sync_errors
```

`connector_items` uses a common envelope:

```text
dataset_id
remote_id
kind
remote_version
occurred_at
updated_at
deleted_at
content_hash
body_json
```

Provider-specific records remain typed inside their provider crate and are
serialized only at the storage boundary. Common fields remain queryable
without understanding every provider payload.

Sync state must support partitions. A Calendar dataset may have one cursor per
calendar, while a Slack dataset may have one cursor per channel.

## Sync Contract

Each driver produces idempotent pages:

```rust
struct SyncPage {
    mutations: Vec<ItemMutation>,
    next_checkpoint: ConnectorCheckpoint,
    complete: bool,
}
```

The engine commits item mutations and the corresponding checkpoint in the same
SQLite transaction. A crash may replay a page safely, but it must never advance
a cursor before storing its data.

The engine supports:

- Upserts by stable remote ID.
- Tombstones for deleted records.
- Provider-specific cursors and sync tokens.
- Cursor expiration and full resynchronization.
- Retry classification and exponential backoff.
- Cancellation and clean shutdown.
- Per-account concurrency limits.
- Manual and scheduled runs.
- Periodic reconciliation when a provider cannot guarantee exact incremental
  filtering.

Reuse `CronSchedule` and schedule calculation from `moltis-cron`, but not
`CronService`. Its payload and execution model are currently tied to agent
turns and system events. The connector manager should own connector execution.

## Projections

SQLite remains the source of truth. JSON and Markdown are rebuildable
projections, not alternative sync stores.

Suggested output layout:

```text
<data_dir>/connectors/exports/<dataset-slug>/
  manifest.json
  items.jsonl
  markdown/
    <stable-item-id>.md
```

Dataset projection options:

- JSONL for static dashboards and scripts.
- Markdown for human reading or explicit memory indexing.
- SQLite-only for large datasets.
- Attachment policy: none, metadata, selected, or all.

Projection replacement uses temporary files and atomic rename. Projections
must never contain credentials or internal cursor state.

Moltis must not serve user-authored HTML from its authenticated application
origin. Arbitrary dashboard JavaScript on that origin could call privileged
APIs. Static dashboards can live beside their JSON projections and be served
by a separate local static server.

## CalDAV Vertical Slice

The first version provides a web-managed CalDAV connection and calendar
datasets.

- Authenticate with an HTTPS server URL, username, and app password.
- Discover calendars after saving or testing a connection.
- Include all calendars or an explicit selection.
- Compare resource ETags and fetch only new or changed iCalendar bodies.
- Reconcile missing resources only after a complete successful inventory.
- Preserve raw VCALENDAR resources and recurrence definitions without
  expanding occurrences.
- Represent calendar metadata separately from event resources.
- Encrypt passwords through vault-backed connector storage.

## Settings UX

Add Connectors next to Channels under the Integrations group in
`crates/web/ui/src/pages/SettingsPage.tsx`. Use the existing page-style settings
lifecycle.

The page has three tabs:

- Connections: connect, reconnect, test, edit, and remove.
- Datasets: natural-language sync instruction, compiled-plan review, run now,
  and preview.
- Activity: current progress, run history, item counts, and errors.

Each dataset card shows:

- Connection and resource type.
- Last successful sync.
- Next scheduled sync.
- Active and deleted record counts.
- Output paths.
- Run now and open output actions.
- Actionable error details.

Follow the Channels and Webhooks patterns for redacted credentials, vault
behavior, account cards, setup guidance, live WebSocket events, and E2E tests.

## Natural-Language Use

Dataset creation is agent-planned. The user supplies one instruction covering
source scope, filters, schedule, and outputs. A one-shot configured model call
compiles it into a typed connector plan. The model receives no credentials or
event bodies, and deterministic validation remains authoritative before save.

Expose one narrow trusted-only read tool:

```text
connectors
  list_datasets
  search_items
  get_item
```

The tool reads the local SQLite mirror only and cannot expose accounts, trigger
sync, or mutate remote data. Search results are bounded previews and full item
reads have a separate byte limit.

Ingested email and message content is untrusted. It must not automatically
enter system prompts or memory indexing because external content can contain
prompt injection. Markdown indexing must be an explicit dataset option, and
tool query results must be bounded and identified as untrusted content.

## Channel Reuse

Reuse:

- Vault-backed secret-field encryption from the channel store.
- Account lifecycle and redacted update patterns.
- Webhook signature validation and replay protection.
- `FairQueue` for bounded per-account processing.
- Settings cards, modals, status events, and setup guidance.
- Provider API clients after extracting them behind lower-level traits.

Do not reuse:

- `ChannelPlugin`.
- `ChannelEventSink`.
- `ChannelReplyTarget`.
- Session binding.
- Sender allowlists and OTP.
- Typing, reply streaming, TTS, and reaction handling.

For Slack and Discord, extract shared API and authentication clients later.
Their connector implementations need history and search APIs, while Channels
primarily use event streams and assistant replies.

## API Shape

Initial RPC methods:

```text
connectors.available
connectors.accounts.list
connectors.accounts.add
connectors.accounts.update
connectors.accounts.remove
connectors.accounts.test
connectors.datasets.list
connectors.datasets.compile
connectors.datasets.add
connectors.datasets.update
connectors.datasets.remove
connectors.datasets.sync
connectors.runs.list
connectors.items.query
```

Use `connectors.*` WebSocket events for account state, sync progress, sync
completion, and failures. Public OAuth callbacks must have narrowly scoped
authentication exemptions, state validation, replay protection, and rate
limits. Secrets and dataset contents must never enter gon data.

## Security and Privacy

- Store passwords, access tokens, refresh tokens, client secrets, and webhook
  secrets as `secrecy::Secret<String>`.
- Encrypt persisted connector credentials through the vault with AAD scoped by
  connector type, account ID, and field name.
- Redact credentials in API objects, logs, status events, and debug output.
- Reject new secret writes while an initialized vault is sealed.
- Create connector databases and projection directories with restrictive local
  permissions.
- Canonicalize projection paths and keep them beneath the connector export root
  by default.
- Apply explicit retention and attachment policies because synchronized email
  and calendars are sensitive even when credentials are encrypted.
- Require authenticated access to connector APIs.
- Never render or execute synchronized HTML email bodies directly.
- Treat all synchronized content as untrusted input to the agent.

## Testing

Add high-coverage tests for:

- A fake paginated driver with updates, deletions, retries, and cursor expiry.
- Atomic item and checkpoint commits.
- Replay after interruption.
- Full reconciliation and tombstoning only after a complete scan.
- Per-partition cursors.
- Scheduler startup, cancellation, and overlapping-run prevention.
- Vault encryption, redaction, sealed-vault behavior, and secret-preserving
  updates.
- CalDAV discovery, ETag inventory, and multiget behavior using protocol
  fixtures and fake clients.
- Atomic JSONL and Markdown projection replacement.
- RPC authorization and response redaction.
- Settings navigation and complete connector workflows in Playwright.
- Prompt-injection boundaries and bounded agent-tool results.

## Delivery Sequence

1. Build the connector core, SQLite store, transactional sync engine, fake
   driver, scheduler, and run history.
2. Add vault-backed CalDAV credentials and calendar discovery.
3. Implement ETag-aware CalDAV event-resource synchronization.
4. Add JSONL and Markdown projections.
5. Add authenticated `connectors.*` RPC.
6. Add the Connectors settings page and E2E coverage.
7. Add REST access and live progress events when external consumers need them.
8. Add agent-compiled dataset instructions, source-state filtering, local FTS,
   and the trusted read-only connector tool.
9. Add Slack, Discord, and other drivers after the synchronization
   contract is proven.

## Key Decision

SQLite is canonical, while Markdown and JSON are projections. This gives the
sync engine one transactional source of truth and still supports dashboards,
scripts, human-readable files, and optional agent indexing without pretending
that one output representation fits every source.
