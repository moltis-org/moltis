---
name: birdclaw
description: Local-first Twitter/X workspace for archiving, searching, and triaging tweets, DMs, likes, bookmarks, and mentions. Includes a web UI for inbox triage and AI scoring.
platforms: [darwin]
homepage: https://github.com/steipete/birdclaw
requires:
  any_bins: [birdclaw]
  install:
    - kind: brew
      formula: steipete/tap/birdclaw
      bins: [birdclaw]
      os: [darwin]
---

# birdclaw — Twitter/X Local Workspace

`birdclaw` is a local-first Twitter workspace that archives, manages, and triages Twitter data (tweets, DMs, likes, bookmarks, mentions) in a self-hosted SQLite database. Includes a web UI for triage.

This complements the `xurl` skill — xurl is for interacting with the X API (posting, searching, engaging); birdclaw is for archiving and triaging your Twitter data locally.

## Secret Safety (MANDATORY)

- birdclaw delegates API access to `xurl` when configured — same secret safety rules apply.
- **Never** read, print, or send `~/.birdclaw/config.json` or `~/.xurl` to LLM context.
- **Never** ask the user to paste API credentials into chat.
- Optional OpenAI API key for inbox AI scoring — set via `OPENAI_API_KEY` env var or Moltis credential store.

## One-Time User Setup

birdclaw works in multiple modes:

1. **Local-only** (no auth): Import Twitter archive ZIPs, search local data
2. **With xurl** (recommended): Full API access for syncing timeline, mentions, DMs
3. **With bird CLI** (cookie-based): Read-only access via browser cookies

For full API access, set up `xurl` first (see the xurl skill), then:

```bash
birdclaw setup
birdclaw doctor
```

## Health Check

```bash
birdclaw doctor
```

## Import Twitter Archive

```bash
birdclaw import /path/to/twitter-archive.zip
```

## Sync via API

```bash
birdclaw sync                    # Sync all (timeline, mentions, DMs, likes, bookmarks)
birdclaw sync --timeline         # Just timeline
birdclaw sync --mentions         # Just mentions
birdclaw sync --dms              # Just DMs
birdclaw sync --likes            # Just likes
birdclaw sync --bookmarks        # Just bookmarks
```

## Search

```bash
birdclaw search "query"
birdclaw search "query" --limit 20 --json
birdclaw search "query" --from @handle
birdclaw search "query" --after 2026-01-01
```

## Triage (CLI)

```bash
birdclaw inbox                   # Show untriaged items
birdclaw inbox --json
birdclaw inbox score             # Run AI scoring on inbox items
```

## Web UI

```bash
birdclaw serve                   # Start web UI at localhost:3000
```

Views: Home, Mentions, Likes, Bookmarks, DMs, Inbox (triage), Blocks.

## Export

```bash
birdclaw export --json                            # Full JSON export
birdclaw export --format jsonl --output /path/    # JSONL shards for Git backup
```

## Saving to Memory

To archive Twitter data into Moltis memory:

1. Run birdclaw search or inbox with `--json`
2. Summarize key items into a daily digest
3. Save as `memory/twitter/YYYY-MM-DD.md`

```markdown
# Twitter/X — 2026-04-28

## Mentions
- @alice replied to our thread about Rust async — positive feedback
- @bob asked about the API rate limits in v2

## Bookmarks
- Thread by @techwriter on database migration patterns (saved for reference)
- Article: "Zero-downtime deployments with blue-green"

## DMs
- @carol: confirmed speaking slot at RustConf
```

## Notes

- Database: `~/.birdclaw/birdclaw.db` (SQLite + FTS5).
- Use `--json` for machine-readable output.
- Web UI runs on localhost:3000 by default.
- AI inbox scoring requires `OPENAI_API_KEY` — optional, not needed for basic triage.
- birdclaw is primarily a Node.js/TypeScript tool — requires Node.js runtime.
