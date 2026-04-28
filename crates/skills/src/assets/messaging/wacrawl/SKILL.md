---
name: wacrawl
description: Read-only local archive and full-text search of WhatsApp Desktop messages via the wacrawl CLI. Use for searching chat history, exporting conversations, and creating encrypted backups. macOS only (reads local WhatsApp Desktop SQLite databases).
platforms: [darwin]
homepage: https://github.com/steipete/wacrawl
requires:
  bins: [wacrawl]
  install:
    - kind: brew
      formula: steipete/tap/wacrawl
      bins: [wacrawl]
      os: [darwin]
    - kind: go
      module: "github.com/steipete/wacrawl/cmd/wacrawl@latest"
      bins: [wacrawl]
---

# wacrawl — WhatsApp Desktop Archive (Read-Only)

`wacrawl` is a read-only local archive tool for WhatsApp Desktop on macOS. It snapshots the local SQLite databases (ChatStorage.sqlite, ContactsV2.sqlite) without modifying them. Use it for searching chat history, exporting conversations, and creating encrypted backups.

This is different from `wacli` — wacrawl is read-only archaeology of local data; wacli is an active WhatsApp Web client for sending/receiving messages.

## Secret Safety

- wacrawl reads local files only — no API tokens needed.
- The optional age encryption key at `~/.wacrawl/age.key` is auto-generated and must not be printed to agent context.
- **Never** read, print, or send `~/.wacrawl/age.key` to LLM context.

## Prerequisites

- macOS only (reads WhatsApp Desktop's local SQLite databases).
- WhatsApp Desktop must be installed and have synced messages.
- Full Disk Access may be required for the terminal app to read WhatsApp's container.

## Health Check

```bash
wacrawl doctor
```

## Sync Local Data

```bash
wacrawl sync
```

Snapshots the current state of WhatsApp Desktop's databases into wacrawl's local store.

## List Chats

```bash
wacrawl chats
wacrawl chats --limit 20
wacrawl chats --json
```

## Search Messages

```bash
wacrawl search "query"
wacrawl search "invoice" --limit 20
wacrawl messages --chat <jid> --limit 50
wacrawl messages --chat <jid> --json
```

## Encrypted Backup

```bash
wacrawl backup
wacrawl backup --output /path/to/backup/
```

Creates age-encrypted JSONL.gz shards suitable for Git-backed storage.

## Saving to Memory

To save search results or chat exports to Moltis memory for future reference:

1. Run wacrawl with `--json` output
2. Summarize the relevant conversations into a daily digest
3. Save as `memory/whatsapp/YYYY-MM-DD.md`

Example workflow:

```bash
wacrawl search "project meeting" --json --limit 20
```

Then write a summary to memory:

```markdown
# WhatsApp — 2026-04-28

## Key Conversations
- Discussion with Alice about project timeline — agreed on June deadline
- Bob shared the updated budget spreadsheet
- Team group: standup notes, blockers discussed
```

## Notes

- Store dir: `~/.wacrawl` (override with `--store`).
- Use `--json` for machine-readable output when parsing.
- JIDs: direct chats look like `<number>@s.whatsapp.net`; groups look like `<id>@g.us`.
- This tool cannot work inside a Linux sandbox — it requires macOS WhatsApp Desktop files.
