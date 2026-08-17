---
name: himalaya
description: Himalaya v2 CLI for reading and managing email through IMAP, JMAP, Gmail, Microsoft Graph, Maildir, M2dir, and SMTP send operations.
origin:
  source: hermes-agent
  url: https://github.com/nousresearch/hermes-agent
  version: 9f22977f
requires:
  bins: [himalaya]
  install:
    - kind: brew
      formula: himalaya
      bins: [himalaya]
      os: [darwin]
    - kind: cargo
      package: himalaya
      bins: [himalaya]
      label: "Install Himalaya v2 (CLI email)"
---

# Himalaya v2 Email CLI

Use this skill only with Himalaya major version 2. Its command and configuration
syntax differs from v1.

## References

- `references/configuration.md` for v2 config paths, accounts, and backends.
- `references/message-composition.md` for v2 compose and send flows.

## Install and configure

Install a v2 build and verify it before continuing:

```bash
himalaya --version
```

Official installation options include Homebrew and the current Git repository:

```bash
brew install himalaya
cargo install --locked --git https://github.com/pimalaya/himalaya.git
```

If the package manager installs v1, use a v2 release or the Git installation.
Run the v2 setup wizard with no subcommand:

```bash
himalaya
```

The wizard prints TOML that can be saved or merged into the config. There is no
v2 `account configure` command. Check the result with:

```bash
himalaya --json account list
himalaya --account=work account check
```

## Command conventions

Use v2's global account and backend selectors before the subcommand. Mailbox
selectors belong to the leaf command:

```bash
himalaya --json --account=work --backend=imap envelope list --mailbox=INBOX
```

- `--json` selects structured output; v2 does not use `--output json`.
- `--account` and `--backend` are global selectors. `--mailbox` is a leaf-command
  option; these can also be written as `-a`, `-b`, and `-m`.
- Shared read backends are `imap`, `jmap`, `gmail`, `msgraph`, `maildir`, and
  `m2dir`. SMTP sends messages; it does not fetch them.
- Message IDs are mailbox-relative for some backends. Re-list after moving mail
  or changing mailboxes.

## Read operations

List mailboxes and envelopes:

```bash
himalaya --json --account=work --backend=imap mailbox list
himalaya --json --account=work --backend=imap \
  envelope list --mailbox=INBOX --page=1 --page-size=20
```

Search with the v2 shared query language:

```bash
himalaya --json --account=work --backend=imap \
  envelope search --mailbox=INBOX -- \
  from alice@example.com and subject meeting order by date desc
```

Read a rendered message or raw RFC 5322/MIME:

```bash
himalaya --account=work --backend=imap message read --mailbox=INBOX -- 42
himalaya --account=work --backend=imap message read --mailbox=INBOX --raw -- 42
```

The shared `envelope search` query language is not implemented by the v2 Gmail
or Microsoft Graph backends. Use their protocol-specific APIs when native
search is required:

```bash
himalaya --json --account=work gmail messages list -q "from:alice is:unread"
himalaya --json --account=work msgraph mail-folder list
```

## Manage messages

Use explicit source and destination mailboxes for copy or move operations:

```bash
himalaya --account=work --backend=imap message copy --from=INBOX --to=Archives -- 42
himalaya --account=work --backend=imap message move --from=INBOX --to=Archives -- 42
```

The shared message command in Himalaya v2.0.0 has no delete operation. Use a
backend-specific command when deletion is required.

Manage flags and attachments:

```bash
himalaya --account=work --backend=imap flag add --mailbox=INBOX --flag=seen -- 42
himalaya --account=work --backend=imap flag remove --mailbox=INBOX --flag=seen -- 42
himalaya --account=work --backend=imap attachment list --mailbox=INBOX -- 42
himalaya --account=work --backend=imap attachment download --mailbox=INBOX -- 42
```

## Debugging

Every command provides v2-specific help. Use logging on stderr when needed:

```bash
himalaya envelope search --help
himalaya --log=debug --account=work --backend=imap mailbox list
himalaya --log=trace --log-file=/tmp/himalaya.log \
  --account=work --backend=imap mailbox list
```
