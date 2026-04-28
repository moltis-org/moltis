---
name: slacrawl
description: Archive and search Slack workspace messages, threads, and channels via the slacrawl CLI. Supports API sync, Slack export ZIP import, local desktop cache import, and full-text search.
platforms: [linux, macos]
homepage: https://github.com/vincentkoc/slacrawl
requires:
  bins: [slacrawl]
  install:
    - kind: brew
      formula: vincentkoc/tap/slacrawl
      bins: [slacrawl]
      os: [darwin]
    - kind: go
      module: "github.com/vincentkoc/slacrawl/cmd/slacrawl@latest"
      bins: [slacrawl]
---

# slacrawl — Slack Workspace Archive

`slacrawl` mirrors Slack workspace data into a local SQLite database for offline search, custom SQL queries, and inspection. Also ingests macOS Slack Desktop local data and Slack export ZIP files.

## Secret Safety (MANDATORY)

- **Never** read, print, or send Slack tokens to LLM context.
- **Never** ask the user to paste tokens into chat.
- The user must set tokens in their environment or Moltis credential store manually.
- To verify credentials, only use: `slacrawl doctor`.

## One-Time User Setup (user runs these outside the agent)

1. Create a Slack app at https://api.slack.com/apps
2. Add bot token scopes: `channels:history`, `channels:read`, `groups:history`, `groups:read`, `im:history`, `im:read`, `mpim:history`, `mpim:read`, `users:read`
3. Install the app to your workspace and copy the Bot User OAuth Token
4. Set tokens:
   - `SLACK_BOT_TOKEN` — required for standard sync
   - `SLACK_APP_TOKEN` — optional, for Socket Mode live tailing
   - `SLACK_USER_TOKEN` — optional, for fuller thread/DM coverage
5. Verify: `slacrawl doctor`

Tokens can be set via environment variables or Moltis credential store (web UI → Settings → Environment Variables).

## Health Check

```bash
slacrawl doctor
```

## Sync Workspace Data

```bash
slacrawl sync
slacrawl sync --full                        # Full resync
slacrawl sync --channel <channel_name>      # Specific channel
```

## Live Tail (Socket Mode)

```bash
slacrawl tail
```

Requires `SLACK_APP_TOKEN` with Socket Mode enabled.

## List Channels and Users

```bash
slacrawl channels
slacrawl channels --json
slacrawl users
slacrawl users --json
```

## Search Messages

```bash
slacrawl search "query"
slacrawl search "query" --channel <channel_name>
slacrawl search "query" --limit 20 --json
slacrawl search "query" --after 2026-01-01 --before 2026-04-28
```

## Import from Slack Export

```bash
slacrawl import /path/to/slack-export.zip
```

## Import from Local Desktop Cache

```bash
slacrawl desktop-import
```

Reads the macOS Slack Desktop local IndexedDB data (may require Node.js for blob decoding).

## SQL Queries

```bash
slacrawl sql "SELECT * FROM messages WHERE text LIKE '%deploy%' ORDER BY ts DESC LIMIT 10"
```

## Saving to Memory

To archive Slack data into Moltis memory:

1. Run slacrawl search with `--json`
2. Summarize key conversations into a daily digest
3. Save as `memory/slack/YYYY-MM-DD.md`

```markdown
# Slack — 2026-04-28

## #engineering
- Deployment of v2.1 completed successfully at 14:30 UTC
- Post-deploy metrics look healthy, no error spikes

## #product
- Decision: prioritize mobile onboarding flow for Q3
- Alice shared mockups for the new settings page

## DMs
- Bob: confirmed Friday 1:1 agenda — performance reviews
```

## Notes

- Database: `~/.slacrawl/slacrawl.db` (SQLite + FTS5).
- Use `--json` for machine-readable output.
- Use `--format log` for compact log-style output.
- Threads are synced automatically when the parent message is fetched.
