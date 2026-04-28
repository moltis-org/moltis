---
name: data-sync
description: Sync and archive data from messaging platforms (WhatsApp, Discord, Slack, Twitter/X, Google) into Moltis memory as daily digest summaries. Orchestrates crawl tools and writes structured markdown to the memory system.
---

# data-sync — Crawl-to-Memory Pipeline

Orchestrate data archiving tools to sync external platform data into Moltis memory as searchable daily digest summaries.

## Supported Sources

| Source | Tool | Auth Required |
|--------|------|---------------|
| WhatsApp | `wacrawl` | None (macOS local files) |
| Discord | `discrawl` | `DISCORD_BOT_TOKEN` |
| Slack | `slacrawl` | `SLACK_BOT_TOKEN` |
| Twitter/X | `birdclaw` | Optional (`xurl` for API) |
| Google Suite | `gog` | Google OAuth2 |

## Workflow

For each source the user wants to sync:

1. **Check prerequisites**: Run `<tool> doctor` to verify the tool is installed and authenticated
2. **Sync latest data**: Run the tool's sync command to pull fresh data
3. **Search/export with `--json`**: Extract relevant messages and data
4. **Summarize into daily digest**: Create a structured markdown summary
5. **Write to memory**: Save as `memory/<source>/YYYY-MM-DD.md`

## Memory Directory Convention

```
memory/
  whatsapp/YYYY-MM-DD.md    # wacrawl digests
  discord/YYYY-MM-DD.md     # discrawl digests
  slack/YYYY-MM-DD.md       # slacrawl digests
  twitter/YYYY-MM-DD.md     # birdclaw digests
  google/YYYY-MM-DD.md      # gog digests
```

## Daily Digest Format

Each digest file should follow this structure for optimal memory indexing (section headers create natural chunk boundaries for search):

```markdown
# <Source> — YYYY-MM-DD

## <Channel/Category 1>
- Key point or conversation summary
- Decisions made, action items
- Notable links or attachments shared

## <Channel/Category 2>
- ...

## Action Items
- [ ] Follow up on X with @person
- [ ] Review document shared by Y
```

## Sync Commands by Source

### WhatsApp (macOS only)
```bash
wacrawl sync
wacrawl search "query" --json --limit 50
```

### Discord
```bash
discrawl sync --all
discrawl search "query" --json --after YYYY-MM-DD
```

### Slack
```bash
slacrawl sync
slacrawl search "query" --json --after YYYY-MM-DD
```

### Twitter/X
```bash
birdclaw sync
birdclaw inbox --json
birdclaw search "query" --json --after YYYY-MM-DD
```

### Google
```bash
gog gmail list --query "after:YYYY/MM/DD" --json --limit 50
gog calendar events --from YYYY-MM-DD --to YYYY-MM-DD --json
```

## Full Sync Example

When the user asks to "sync all my messages" or "update my data":

1. Check which tools are installed: `which wacrawl discrawl slacrawl birdclaw gog`
2. For each available tool, run doctor to check auth
3. Sync each authenticated source
4. Search for today's data with `--json`
5. Write daily digest summaries to memory

## Guidelines

- **Append, never overwrite**: Daily digest files are append-only. If syncing multiple times per day, append new sections.
- **Keep digests concise**: Summarize conversations, don't dump raw messages. Aim for key decisions, action items, and notable information.
- **Date in filename**: Use `YYYY-MM-DD.md` format. The memory system indexes by filename date.
- **50 KB limit**: Each memory write is limited to 50 KB. For high-volume days, focus on the most important items.
- **No secrets in memory**: Never write API tokens, passwords, or private keys into memory files.
- **Ask before first sync**: On first use, confirm with the user which sources they want to sync and how much history to pull.
