# Message Composition with Himalaya v2

Himalaya v2 can compose simple messages from flags or send a prepared RFC
5322/MIME message from stdin. Rich MML composition is provided by the separate
[`mml`](https://github.com/pimalaya/mml) program, not by Himalaya's removed v1
template commands.

## Simple message

Compose and send without opening an editor:

```bash
himalaya --account=work message compose \
  --to=recipient@example.com \
  --subject="Status update" \
  --body="The deployment completed." \
  --send
```

Use `message compose --help`, `message reply --help`, or `message forward
--help` for the exact v2 flags supported by the installed build. Do not use the
v1 `message write`, `template send`, `template reply`, or `template forward`
commands.

## Prepared MIME message

Create a normal message with headers, a blank line, and a body:

```text
From: sender@example.com
To: recipient@example.com
Subject: Status update

The deployment completed.
```

Send it through the account's configured send backend:

```bash
himalaya --account=work message send < message.eml
```

SMTP is only a send backend. It is not used by `mailbox`, `envelope`, or
`message read` commands.

## Drafts

Add a prepared message to a configured drafts mailbox:

```bash
himalaya --account=work --backend=imap \
  message add --mailbox=drafts --flag=draft < message.eml
```

## Rich MIME and attachments

Use the standalone `mml` composer for multipart bodies, HTML, attachments,
signing, encryption, or editor-driven composition, then pipe its output to
Himalaya v2:

```bash
mml compose >(himalaya --account=work message send)
```

Consult `mml --help` for MML directives and attachment syntax. Keeping
composition in `mml` and delivery in `himalaya message send` avoids relying on
v1 template behavior.
