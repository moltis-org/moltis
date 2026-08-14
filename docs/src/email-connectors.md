# Email Connectors

Moltis provides two read-only email connectors:

- **Gmail** reads the Gmail API using Moltis' existing Google Workspace OAuth
  authorization.
- **Himalaya** invokes an existing host-side
  [Himalaya](https://github.com/pimalaya/himalaya) v2 account and selected read
  backend.

Both use the shared connector envelope for accounts, datasets, runs,
projections, and atomic commits. Their message schemas, synchronization logic,
and agent tools remain provider-owned. A failed or interrupted fetch leaves the
previous valid dataset snapshot unchanged.

## Credentials

The Gmail connector reuses `<data_dir>/google_token.json`. Authorize the Gmail
service with the bundled Google Workspace skill first; the token must include a
Gmail read-capable scope. The connector references that file and does not copy
OAuth credentials into `connectors.db`.

The Himalaya connector reuses an account already configured for the `himalaya`
binary on the Moltis host. By default, Himalaya v2 loads the first available
configuration from `$XDG_CONFIG_HOME/himalaya/config.toml`,
`~/.config/himalaya/config.toml`, or `~/.himalayarc`. Moltis stores only the
account name and selected backend, not the config file, passwords, or tokens.

## Setup and test

### Gmail

1. Complete Google Workspace OAuth setup with the bundled skill, requesting the
   least-privilege Gmail scope, and verify that `<data_dir>/google_token.json`
   exists:

   ```bash
   python "$SKILL_DIR/scripts/setup.py" --client-secret /path/to/client_secret.json
   python "$SKILL_DIR/scripts/setup.py" --auth-url --services gmail-readonly
   python "$SKILL_DIR/scripts/setup.py" --auth-code "THE_REDIRECT_URL"
   python "$SKILL_DIR/scripts/setup.py" --check
   ```
2. Open **Settings -> Connectors -> Connections**, select **Add Gmail
   connection**, name it, and save it.
3. Select **Test connection**. Moltis reads the Gmail profile with the existing
   token and reports the authorized address.
4. Open **Datasets**, create an email dataset, set a Gmail search query, message
   limit, body option, schedule, and optional projections, then run it.

The query uses Gmail search syntax and may be blank for the newest messages in
the account. `maxMessages` is bounded from 1 to 1,000. `includeBody` stores a
bounded plain-text body when available; HTML is converted to text. When it is
off, headers, snippet, labels, and attachment metadata are still retained.

### Himalaya v2

1. Install Himalaya and confirm that `himalaya --version` reports major version
   2. Version 1 is rejected.
2. Configure and test the host account with Himalaya itself. The v2 setup wizard
   is the bare `himalaya` command; it is not `himalaya account configure`.
3. Verify the exact account and backend that Moltis will use:

   ```bash
   himalaya --json account list
   himalaya --account=work account check
   himalaya --json --account=work --backend=imap mailbox list
   himalaya --json --account=work --backend=imap \
     envelope list --mailbox=INBOX --page=1 --page-size=20
   himalaya --account=work --backend=imap \
     message read --mailbox=INBOX --raw -- 42
   ```

4. Open **Settings -> Connectors -> Connections**, select **Add Himalaya
   connection**, enter that exact account name and backend, save, and select
   **Test connection**. The test discovers mailboxes through Himalaya.
5. Open **Datasets**, select 1 to 32 discovered mailbox IDs, set a total message
   limit from 1 to 1,000, choose whether to store bodies, and optionally set a
   schedule and projections.

Select the backend explicitly: `imap`, `jmap`, `gmail`, `msgraph`, `maildir`, or
`m2dir`. The message limit is shared across the selected mailboxes. For IMAP,
JMAP, Maildir, and M2dir, an optional query uses Himalaya v2's shared
`envelope search` query language. The Himalaya v2 shared search command is not
supported by its Gmail or Microsoft Graph backends, so those connector datasets
must leave the query blank. Use mailbox selection and the message limit to
bound them.

SMTP is a send-only Himalaya backend. It is not a connector backend and is
never used to list, search, or fetch synchronized messages.

## Local agent access

Email datasets are exposed through separate trusted-only tools so provider
schemas do not leak into the shared connector API:

- `gmail_connector`: `list_datasets`, `search_messages`, and `get_message`.
- `himalaya_connector`: `list_datasets`, `search_messages`, and `get_message`.

These operations read only `connectors.db`. They cannot synchronize, execute
Himalaya commands, mutate or send mail, download attachments, or reveal account
configuration. Searches and results are bounded.

```admonish warning
Email is untrusted external content and can contain prompt injection. Treat
message subjects, bodies, addresses, and filenames as data, never as
instructions. Keep `gmail_connector` and `himalaya_connector` available only to
trusted operators and agents.
```

## Storage and retention

Synchronized headers, snippets, selected message bodies, provider metadata, and
search text are stored in plaintext in `<data_dir>/connectors.db`. Enabled JSONL
and Markdown projections under `<data_dir>/connectors/exports/` are also
plaintext. Protect the data directory and exports according to the sensitivity
of the selected mailboxes.

Datasets retain attachment metadata only, such as filename, content type, size,
and a provider identifier when available. Attachment payloads are not stored or
exposed through the connector tools. Disabling the source account stops future
fetches without erasing existing snapshots. Removing a dataset or connection
deletes its synchronized items and projections.
