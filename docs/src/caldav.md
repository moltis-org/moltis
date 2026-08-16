# CalDAV (Calendars)

Moltis can read and manage remote calendars through the
[CalDAV](https://en.wikipedia.org/wiki/CalDAV) protocol. Once configured, the
agent gains a `caldav` tool that can list calendars, query events, and create,
update, or delete entries on your behalf.

Moltis also provides a read-only **CalDAV connector** that synchronizes remote
calendar data into local datasets. The connector is configured from **Settings
-> Connectors** and is independent from the agent tool described below.
The same connector subsystem can also reuse supported channel accounts for
bounded message-history datasets; see [Connectors](connectors.md).

Both capabilities are compiled in by default. The `connectors` cargo feature
controls inbound synchronization, while `caldav` controls the agent tool. They
can be omitted from custom builds with `--no-default-features`.

Although the live tool and local connector have separate execution paths,
their authentication can be shared. On startup, every valid account under
`[caldav.accounts]` is automatically created or reconciled as a connector
connection and appears in **Settings -> Connectors**. Fastmail and iCloud URLs
use the same provider defaults as the live tool. For a generic provider, the
configured URL must be the actual CalDAV context endpoint: the connector does
not yet follow the live tool's broader service-discovery path.

Imported connections are identified by their config account key, so restarts
do not create duplicates. A single matching connection that was previously
created in Settings is adopted instead. Changes to the configured URL,
username, password, timeout, or global `caldav.enabled` state are reconciled;
unchanged encrypted passwords are preserved byte-for-byte. Connector-only
network permissions remain editable in Settings and are not overwritten by
config reconciliation. Removing or invalidating a config entry disables its
imported connection and clears the copied credential, but never deletes its
local datasets, history, or projections. Restoring a valid entry restores the
credential and re-enables the same connection.

Config-managed connections are labeled in Settings. Their identity,
credentials, timeout, enabled state, and removal are controlled by
`moltis.toml`; only their advanced network permissions can be changed in the
connector UI. This restriction is enforced by the connector API as well as the
UI. Manually created connections require the password to be re-entered whenever
the server URL or username changes, preventing a stored credential from being
silently forwarded to a different authority.

When an initialized vault is sealed, imports and password rotations that would
write a secret are deferred rather than stored as plaintext. Reconciliation is
retried automatically after the vault is unsealed.

## Inbound connector

Use the inbound connector when you want a durable local mirror that agents can
search without waiting for the CalDAV server:

1. Open **Settings -> Connectors -> Connections**.
2. Add a CalDAV connection with its HTTPS server URL, username, and an app
   password, or use an account imported automatically from `[caldav.accounts]`.
3. Open **Datasets** and describe what Moltis should keep locally, for example:

   ```text
   Keep events from the past year where I accepted the invitation.
   Refresh every 30 minutes and generate Markdown projections.
   ```

4. Select **Compile plan**. The configured model translates the instruction
   into a typed plan containing calendar scope, absolute date bounds,
   participation filtering, schedule, and projection settings.
5. Review the compiled summary and save the dataset.

Changing the instruction invalidates the previous plan until it is compiled
again. An advanced JSON override is available for inspecting or precisely
adjusting the generated plan, but free-form model output is never executed
directly.

Plan compilation sends the instruction and discovered calendar display names
to the first configured tool-capable model, falling back to the first configured
model. Private CalDAV hrefs are replaced by opaque identifiers before this
request. Moltis does not send the password, username, server URL, dataset/account
IDs, or event bodies. Compilation requires a configured model and is separate
from scheduled synchronization. Configure only models whose privacy policy is
appropriate for calendar names and the instructions you enter.

The connector stores canonical records in `connectors.db`. Each dataset keeps
calendar metadata and raw iCalendar resources, including recurrence rules,
exceptions, time zone definitions, alarms, and custom properties. Recurrences
are preserved rather than expanded into an unbounded list of occurrences.

On later runs, Moltis compares CalDAV ETags and fetches only new or changed
resources. Source state is retained for both included and filtered resources,
so an unchanged rejected event is not repeatedly downloaded. Changing the
compiled plan increments its revision and forces affected resources to be
evaluated again. Missing resources are marked deleted only after a complete,
successful calendar inventory. A failed or interrupted sync therefore cannot
erase the last valid local snapshot.

CalDAV date filtering currently compares each VEVENT component's `DTSTART`
date. The start bound is inclusive and the end bound is exclusive. Recurrence
rules are preserved but not expanded, so a recurring series is evaluated by
its stored components rather than every generated occurrence. Relative phrases
such as “the past year” are resolved to absolute dates when the plan is
compiled.

“Accepted by me” is evaluated locally by matching the account username against
an `ATTENDEE` value with `PARTSTAT=ACCEPTED`. Servers generally cannot apply
that predicate themselves.

Generated projections are written beneath:

```text
<data_dir>/connectors/exports/<dataset-name>-<dataset-id-hash>/
```

The exact path appears on the dataset card after projections are enabled.
`manifest.json` describes the snapshot, `items.jsonl` supports scripts and
static dashboards, and `items/*.md` contains generic Markdown records.

Connector passwords are stored as secrets and are encrypted through the
[Vault](vault.md) when encryption at rest is enabled and unlocked. API and UI
responses never return the password. Calendar contents remain sensitive local
data in `connectors.db` and projection files, so protect the Moltis data
directory and exported files accordingly.

HTTPS and public network targets are required by default. Advanced settings can
permit cleartext HTTP or private-network servers for explicitly trusted local
CalDAV installations. These options weaken the default network protections and
should only be enabled when required. Connector server URLs cannot contain
embedded credentials, query strings, or fragments; authentication belongs in
the dedicated username and password fields. Public hosts are resolved once,
every returned address is validated, and those addresses are pinned for the
CalDAV client so DNS rebinding cannot switch a validated host to a private
address during connection.

## Local agent access

Trusted agent turns receive a read-only `connectors` tool. It operates entirely
on `connectors.db`; it never contacts the CalDAV server. The available
operations are:

- `list_datasets` — list synchronized datasets and freshness state.
- `search_items` — bounded full-text search over one local dataset.
- `get_item` — retrieve one selected local item.

Search results are intentionally small previews, while `get_item` returns a
bounded full body. Results are marked `untrusted_external` because calendar
content can contain prompt injection and must be treated as data, not
instructions. The tool does not expose connector accounts, credentials,
absolute projection paths, synchronization controls, or mutation operations.
It is trusted-only by default and can be removed further through normal agent
tool policy.

The inbound connector intentionally does not expand recurring events, write
events, or use CalDAV push notifications. The separate live `caldav` tool can
still create, update, and delete events, while normal reads can use the faster
local `connectors` tool.

## Configuration

Add a `[caldav]` section to your `moltis.toml` (usually `~/.moltis/moltis.toml`):

```toml
[caldav]
enabled = true
default_account = "fastmail"

[caldav.accounts.fastmail]
provider = "fastmail"
username = "you@fastmail.com"
password = "app-specific-password"
```

### Multiple accounts

You can define as many accounts as you like. When only one account exists it is
used implicitly; otherwise specify `default_account` or pass `account` in each
tool call.

```toml
[caldav]
enabled = true
default_account = "work"

[caldav.accounts.work]
provider = "fastmail"
username = "work@fastmail.com"
password = "app-specific-password"

[caldav.accounts.personal]
provider = "icloud"
username = "you@icloud.com"
password = "app-specific-password"
```

### Supported providers

| Provider | `provider` value | Notes |
|----------|------------------|-------|
| **Fastmail** | `"fastmail"` | URL auto-discovered (`caldav.fastmail.com`). Use an [app password](https://www.fastmail.com/help/clients/apppassword.html). |
| **iCloud** | `"icloud"` | URL auto-discovered (`caldav.icloud.com`). Requires an [app-specific password](https://support.apple.com/en-us/102654). |
| **Generic** | `"generic"` | Any CalDAV server. You **must** set `url`. |

For generic servers, provide the CalDAV base URL:

```toml
[caldav.accounts.nextcloud]
provider = "generic"
url      = "https://cloud.example.com/remote.php/dav"
username = "admin"
password = "secret"
```

### Account fields

| Field | Required | Default | Description |
|-------|----------|---------|-------------|
| `provider` | no | `"generic"` | Provider hint (`"fastmail"`, `"icloud"`, `"generic"`) |
| `url` | depends | &mdash; | CalDAV base URL. Required for `generic`; optional for Fastmail/iCloud (well-known URL used). |
| `username` | yes | &mdash; | Authentication username |
| `password` | yes | &mdash; | Password or app-specific password |
| `timeout_seconds` | no | `30` | HTTP request timeout |

```admonish warning
Store passwords as app-specific passwords, never your main account password.
Passwords are stored in `moltis.toml` and redacted in logs, but the file itself
is plain text on disk. The Vault does not encrypt TOML configuration; use an app
password and restrict access to the configuration file. Web-managed connector
credentials use the vault-backed storage described above.
```

## How it works

When Moltis starts and CalDAV is enabled with at least one account, a `caldav`
tool is registered in the agent tool registry. The agent can then call it during
conversations to interact with your calendars.

Connections are established lazily on first use and cached for the lifetime of
the process. All communication uses HTTPS with system-native TLS roots.

## Operations

The agent calls the `caldav` tool with an `operation` parameter. Five
operations are available:

### `list_calendars`

Lists all calendars available on the account.

Returns: `href`, `display_name`, `color`, `description` for each calendar.

### `list_events`

Lists events in a specific calendar, optionally filtered by date range.
When both `start` and `end` are given, the filter runs server-side as a
CalDAV `calendar-query` REPORT with a `time-range` element (RFC 4791), so
only events intersecting the window are fetched. Recurring resources are
expanded by the server for that window, so results contain the matching
occurrences rather than an out-of-range recurrence master. If either bound is
omitted, all events are returned without recurrence expansion.

| Parameter | Required | Description |
|-----------|----------|-------------|
| `calendar` | yes | Calendar href (from `list_calendars`) |
| `start` | no | ISO 8601 start date/time (naive times are treated as UTC) |
| `end` | no | ISO 8601 end date/time (naive times are treated as UTC) |

Returns: `href`, `etag`, `uid`, `summary`, `start`, `end`, `all_day`,
`location` for each event.

### `create_event`

Creates a new calendar event.

| Parameter | Required | Description |
|-----------|----------|-------------|
| `calendar` | yes | Calendar href |
| `summary` | yes | Event title |
| `start` | yes | ISO 8601 start (e.g. `2025-06-15T10:00:00` or `2025-06-15` for all-day) |
| `end` | no | ISO 8601 end date/time |
| `all_day` | no | Boolean, default `false` |
| `location` | no | Event location |
| `description` | no | Event notes |

Returns: `href`, `etag`, `uid` of the created event.

### `update_event`

Updates an existing event. Uses ETag-based optimistic concurrency control to
prevent overwriting concurrent changes.

| Parameter | Required | Description |
|-----------|----------|-------------|
| `event_href` | yes | Event href (from `list_events`) |
| `etag` | yes | Current ETag (from `list_events`) |
| `summary` | no | New title |
| `start` | no | New start |
| `end` | no | New end |
| `all_day` | no | New all-day flag |
| `location` | no | New location |
| `description` | no | New description |

Returns: updated `href` and `etag`.

### `delete_event`

Deletes an event. Also requires the current ETag.

| Parameter | Required | Description |
|-----------|----------|-------------|
| `event_href` | yes | Event href |
| `etag` | yes | Current ETag |

## Concurrency control

Updates and deletes require an `etag` obtained from `list_events`. If the event
was modified on the server since the ETag was fetched (e.g. edited from a phone),
the server rejects the request with a conflict error. This prevents accidental
overwrites. The agent should re-fetch the event and retry.

## Example conversation

> **You:** What's on my calendar this week?
>
> The agent calls `list_calendars`, picks the primary calendar, then calls
> `list_events` with `start` / `end` spanning the current week.
>
> **Agent:** You have 3 events this week: ...
>
> **You:** Move the dentist appointment to Friday at 2pm.
>
> The agent calls `update_event` with the event's `href` and `etag`, setting
> the new `start` time.

## Disabling CalDAV

Set `enabled = false` or remove the `[caldav]` section entirely:

```toml
[caldav]
enabled = false
```

To disable at compile time, build without the feature:

```bash
cargo build --release --no-default-features --features lightweight
```

## Validation

`moltis config check` validates CalDAV configuration and warns about unknown
providers. Valid provider values are: `fastmail`, `icloud`, `generic`.
