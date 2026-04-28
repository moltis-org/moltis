---
name: gogcli
description: CLI for Google Suite services — Gmail, Calendar, Chat, Drive, Docs, Sheets, Slides, Contacts, Tasks, and more. Search emails, manage calendar events, read and create documents, and interact with Google Workspace.
platforms: [linux, macos]
homepage: https://github.com/steipete/gogcli
requires:
  any_bins: [gogcli, gog]
  install:
    - kind: brew
      formula: steipete/tap/gogcli
      bins: [gogcli]
      os: [darwin]
    - kind: go
      module: "github.com/steipete/gogcli/cmd/gog@latest"
      bins: [gog]
---

# gogcli — Google Suite CLI

`gogcli` (also available as `gog`) is a comprehensive CLI for Google services: Gmail, Calendar, Chat, Classroom, Drive, Docs, Slides, Sheets, Forms, Apps Script, Contacts, Tasks, People, Admin, and Groups.

## Secret Safety (MANDATORY)

- **Never** read, print, or send OAuth credentials, refresh tokens, or keyring data to LLM context.
- **Never** ask the user to paste Google credentials into chat.
- The user must complete `gogcli auth login` manually, outside the agent session.
- Credentials are stored in the platform keyring or encrypted file backend — never access these directly.
- To verify credentials, only use: `gogcli auth status`.

## One-Time User Setup (user runs these outside the agent)

1. Create a Google Cloud project at https://console.cloud.google.com
2. Enable APIs: Gmail, Calendar, Drive, Docs, Sheets (as needed)
3. Create OAuth2 credentials (Desktop application type)
4. Download the credentials JSON
5. Authenticate:
   ```bash
   gogcli auth login --credentials /path/to/credentials.json
   ```
6. Verify: `gogcli auth status`

Tokens persist in the platform keyring or `~/.config/gogcli/` encrypted file backend. Multi-account support available.

## Health Check

```bash
gogcli auth status
```

## Gmail

```bash
gogcli gmail list --limit 20
gogcli gmail list --query "from:alice subject:meeting" --json
gogcli gmail read <message_id>
gogcli gmail read <message_id> --json
gogcli gmail search "invoice 2026" --limit 10
gogcli gmail send --to "alice@example.com" --subject "Meeting" --body "See you at 3pm"
gogcli gmail send --to "alice@example.com" --subject "Report" --attach /path/to/report.pdf
```

## Calendar

```bash
gogcli calendar list                              # List calendars
gogcli calendar events --limit 10                 # Upcoming events
gogcli calendar events --from 2026-04-28 --to 2026-05-05 --json
gogcli calendar create --title "Team sync" --start "2026-04-29T10:00:00" --duration 30m
gogcli calendar create --title "All-hands" --start "2026-05-01T14:00:00" --attendees "alice@example.com,bob@example.com"
```

## Drive

```bash
gogcli drive list
gogcli drive list --query "name contains 'report'" --json
gogcli drive download <file_id> --output /path/to/save/
gogcli drive upload /path/to/file.pdf
gogcli drive upload /path/to/file.pdf --folder <folder_id>
```

## Docs

```bash
gogcli docs read <doc_id>
gogcli docs read <doc_id> --json
gogcli docs create --title "Meeting Notes" --body "# Agenda\n- Item 1\n- Item 2"
```

## Sheets

```bash
gogcli sheets read <spreadsheet_id>
gogcli sheets read <spreadsheet_id> --range "Sheet1!A1:D10" --json
gogcli sheets append <spreadsheet_id> --range "Sheet1!A:D" --values '[["Name","Value"],["test","123"]]'
```

## Contacts / People

```bash
gogcli contacts list --limit 20
gogcli contacts search "Alice" --json
```

## Tasks

```bash
gogcli tasks list
gogcli tasks list --json
gogcli tasks create --title "Review PR #456"
gogcli tasks complete <task_id>
```

## Saving to Memory

To archive Google data into Moltis memory:

1. Run gogcli with `--json` output
2. Summarize relevant data into a daily digest
3. Save as `memory/google/YYYY-MM-DD.md`

```markdown
# Google — 2026-04-28

## Gmail
- Alice: sent updated project timeline (attachment: timeline-v3.pdf)
- Newsletter: Rust weekly #142 — async trait stabilization
- Invoice from Acme Corp for April hosting ($450)

## Calendar
- 10:00 Team sync — discussed Q3 priorities
- 14:00 1:1 with Bob — performance review prep
- Tomorrow: All-hands at 15:00

## Drive
- Shared "Q2 OKR Tracker" spreadsheet updated by Carol
```

## Output Format

All commands support `--json` for structured JSON output and `--plain` for TSV format suitable for piping to other tools.

## Notes

- Config: `~/.config/gogcli/` (platform-specific config directory, JSON5 format).
- Multi-account: `gogcli auth login --account work` then `gogcli --account work gmail list`.
- Credentials persist in platform keyring (macOS Keychain, Linux Secret Service) or encrypted file fallback.
- Binary is named `gog` when installed via `go install`; `gogcli` via Homebrew. Both work.
