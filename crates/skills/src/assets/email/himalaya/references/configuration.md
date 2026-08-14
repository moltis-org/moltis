# Himalaya v2 Configuration

Himalaya v2 loads the first valid file from:

- `$XDG_CONFIG_HOME/himalaya/config.toml`
- `~/.config/himalaya/config.toml`
- `~/.himalayarc`

Override it with `himalaya --config <path>` (or `-c`). Run bare `himalaya` to
generate account TOML, then validate configured accounts with `himalaya account
list` and `himalaya account check`. V2 has no `account configure` subcommand.

## IMAP and SMTP

This minimal account reads with IMAP and sends with SMTP:

```toml
[accounts.work]
default = true

imap.server = "imaps://imap.example.com:993"
imap.sasl.plain.username = "user@example.com"
imap.sasl.plain.password.command = "pass show email/imap"

smtp.server = "smtp://smtp.example.com:587"
smtp.starttls = true
smtp.sasl.plain.username = "user@example.com"
smtp.sasl.plain.password.command = "pass show email/smtp"

mailbox.alias.inbox = "INBOX"
mailbox.alias.sent = "Sent"
mailbox.alias.drafts = "Drafts"
mailbox.alias.trash = "Trash"
```

V2 uses backend-specific keys such as `imap.server` and
`imap.sasl.plain.password.command`. The v1 `backend.type`, `backend.host`,
`backend.auth.*`, and `message.send.backend.*` keys are not v2 configuration.

SMTP is send-only. Commands that list, search, or read mail must use a readable
backend such as IMAP, JMAP, Gmail, Microsoft Graph, Maildir, or M2dir.

## Secrets

Prefer a command that prints the secret on stdout:

```toml
imap.sasl.plain.password.command = "pass show email/imap"
smtp.sasl.plain.password.command = ["security", "find-generic-password", "-a", "user@example.com", "-s", "smtp", "-w"]
```

Raw values are suitable only for temporary testing:

```toml
imap.sasl.plain.password.raw = "app-password"
```

Native keyring integration and built-in OAuth flows were removed in v2. Use a
secret-manager CLI or token broker as the relevant `command` value.

## Gmail

Gmail over IMAP/SMTP requires an app password when using SASL PLAIN:

```toml
[accounts.gmail]
default = true

imap.server = "imaps://imap.gmail.com:993"
imap.sasl.plain.username = "you@gmail.com"
imap.sasl.plain.password.command = "pass show gmail"

smtp.server = "smtps://smtp.gmail.com:465"
smtp.sasl.plain.username = "you@gmail.com"
smtp.sasl.plain.password.command = "pass show gmail"

mailbox.alias.inbox = "INBOX"
mailbox.alias.sent = "[Gmail]/Sent Mail"
mailbox.alias.drafts = "[Gmail]/Drafts"
mailbox.alias.trash = "[Gmail]/Trash"
mailbox.alias.archive = "[Gmail]/All Mail"
```

To use Himalaya's native Gmail REST backend instead, provide a short-lived
access token through an external broker:

```toml
[accounts.gmail-api]
gmail.auth.token.command = ["ortie", "token", "show", "-a", "gmail"]
```

Select it with `--account=gmail-api --backend=gmail`. Token refresh belongs to
the external command.

## JMAP and Microsoft Graph

Example JMAP account:

```toml
[accounts.fastmail]
jmap.server = "https://api.fastmail.com/jmap/session"
jmap.auth.bearer.token.command = "pass show fastmail/api-token"
```

Example native Microsoft Graph account:

```toml
[accounts.outlook]
msgraph.auth.token.command = ["ortie", "token", "show", "-a", "msgraph"]
```

Select native Graph with `--account=outlook --backend=msgraph`. Himalaya v2's
shared `envelope search` query language is not supported by the Gmail or
Microsoft Graph backends; use protocol-specific commands for native queries.

## Local backends and multiple accounts

Maildir uses a local root:

```toml
[accounts.local]
maildir.root = "~/Mail/local"
```

Declare additional `[accounts.<name>]` tables and select one globally:

```bash
himalaya --json account list
himalaya --account=work --backend=imap mailbox list
```

When an account has multiple backends, always pass `--backend` to avoid relying
on backend ordering.
