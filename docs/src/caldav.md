# CalDAV (Calendars)

Moltis can read and manage remote calendars through the
[CalDAV](https://en.wikipedia.org/wiki/CalDAV) protocol. Once configured, the
agent gains a `caldav` tool that can list calendars, query events, and create,
update, or delete entries on your behalf.

The feature is compiled in by default (the `caldav` cargo feature) and can be
disabled at build time with `--no-default-features`.

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
is plain text on disk. Consider using [Vault](vault.md) for encryption at rest.
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

| Parameter | Required | Description |
|-----------|----------|-------------|
| `calendar` | yes | Calendar href (from `list_calendars`) |
| `start` | no | ISO 8601 start date/time |
| `end` | no | ISO 8601 end date/time |

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
