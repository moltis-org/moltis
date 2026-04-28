---
name: discrawl
description: Archive and search Discord guild messages, threads, and members via the discrawl CLI. Supports bot API sync, local cache import, full-text and semantic search, and Git-backed NDJSON snapshots.
platforms: [linux, macos]
homepage: https://github.com/steipete/discrawl
requires:
  bins: [discrawl]
  install:
    - kind: brew
      formula: steipete/tap/discrawl
      bins: [discrawl]
      os: [darwin]
    - kind: go
      module: "github.com/steipete/discrawl/cmd/discrawl@latest"
      bins: [discrawl]
---

# discrawl — Discord Guild Archive

`discrawl` mirrors Discord guild data (channels, threads, messages, members) into a local SQLite database for offline search and inspection.

This is different from the `discord` channel skill — discrawl is for archiving and searching historical guild data; the discord channel skill is for sending messages via Moltis.

## Secret Safety (MANDATORY)

- **Never** read, print, or send Discord bot tokens to LLM context.
- **Never** ask the user to paste tokens into chat.
- The user must set `DISCORD_BOT_TOKEN` in their environment or Moltis credential store manually.
- To verify credentials, only use: `discrawl doctor`.

## One-Time User Setup (user runs these outside the agent)

1. Create a Discord application at https://discord.com/developers/applications
2. Create a bot user and copy the bot token
3. Set the token:
   - Via env var: `export DISCORD_BOT_TOKEN=<token>`
   - Via Moltis credential store (web UI → Settings → Environment Variables)
   - Via discrawl config: `~/.discrawl/config.toml`
4. Invite the bot to target guilds with message read permissions
5. Verify: `discrawl doctor`

## Health Check

```bash
discrawl doctor
```

## Sync Guild Data

```bash
discrawl sync --guild <guild_id>
discrawl sync --guild <guild_id> --full    # Full resync
discrawl sync --all                         # All configured guilds
```

## List Guilds and Channels

```bash
discrawl guilds
discrawl guilds --json
discrawl channels --guild <guild_id>
discrawl channels --guild <guild_id> --json
```

## Search Messages

```bash
discrawl search "query"
discrawl search "query" --guild <guild_id>
discrawl search "query" --channel <channel_id> --limit 20
discrawl search "query" --json
discrawl search "query" --after 2026-01-01 --before 2026-04-28
```

## Export and Backup

```bash
discrawl snapshot --guild <guild_id>             # NDJSON snapshot for Git backup
discrawl snapshot --guild <guild_id> --output /path/to/export/
```

## SQL Queries

```bash
discrawl sql "SELECT * FROM messages WHERE content LIKE '%keyword%' ORDER BY timestamp DESC LIMIT 10"
```

## Saving to Memory

To archive Discord data into Moltis memory:

1. Run discrawl search or sync with `--json`
2. Summarize key conversations into a daily digest
3. Save as `memory/discord/YYYY-MM-DD.md`

```markdown
# Discord — 2026-04-28

## #general
- Team discussed deployment timeline for v2.0
- Decision: feature freeze on May 1st

## #engineering
- Bug report: auth timeout on mobile clients — assigned to Alice
- PR #234 merged: new caching layer

## #random
- Bob shared article on Rust async patterns
```

## Configuration

Config file: `~/.discrawl/config.toml`

```toml
[guilds.my-server]
id = "123456789"
channels = ["general", "engineering"]  # optional filter
```

## Notes

- Database: `~/.discrawl/discrawl.db` (SQLite + FTS5).
- Use `--json` for machine-readable output.
- Wiretap mode: import from local Discord Desktop cache (DM recovery, no bot token needed).
- Optional semantic search with embedding providers (OpenAI, Ollama, Nomic).
