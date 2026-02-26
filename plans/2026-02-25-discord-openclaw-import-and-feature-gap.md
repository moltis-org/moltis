# Discord: OpenClaw import and feature gap analysis

## Context

OpenClaw has a mature Discord implementation (~50 source files in `src/discord/`)
with extensive features. Moltis now has a working Discord channel integration
(gateway connection, DM/guild messaging, streaming, embeds) but is missing many
features that openclaw users expect. Additionally, the openclaw import system
detects Discord configs but flags them as "unsupported" — tokens are not migrated.

This plan covers two areas:
1. Enabling Discord bot token import from openclaw during onboarding
2. Feature gaps between moltis and openclaw Discord implementations

---

## 1. Import Discord bot tokens from openclaw

### Current state

- `crates/openclaw-import/src/types.rs`: Discord config stored as
  `Option<serde_json::Value>` (raw, untyped)
- `crates/openclaw-import/src/detect.rs`: Discord is pushed to
  `unsupported_channels` (line 289)
- `crates/openclaw-import/src/channels.rs`: Only Telegram accounts are imported;
  Discord appears in the "unsupported" warnings
- Onboarding UI (`onboarding-view.js` line 2742) shows: "Unsupported channels
  (coming soon): discord"

### OpenClaw Discord config shape

From `src/config/types.discord.ts`, openclaw stores Discord config in two forms:

```json5
// Flat (single account)
{
  channels: {
    discord: {
      token: "BOT_TOKEN",
      enabled: true,
      dmPolicy: "pairing",    // or "allowlist", "open", "disabled"
      allowFrom: ["user_id"], // DM allowlist
      groupPolicy: "allowlist",
      guilds: {
        "GUILD_ID": {
          requireMention: true,
          users: ["USER_ID"],
          channels: { "CHANNEL_ID": { allow: true } }
        }
      }
    }
  }
}

// Multi-account
{
  channels: {
    discord: {
      accounts: {
        "my-bot": {
          token: "BOT_TOKEN",
          dmPolicy: "pairing",
          allowFrom: ["user_id"],
          // ...same fields as flat
        }
      }
    }
  }
}
```

### Changes needed

#### `crates/openclaw-import/src/types.rs`

Add typed Discord config structs (mirror the Telegram pattern):

```rust
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct OpenClawDiscordConfig {
    pub accounts: Option<HashMap<String, OpenClawDiscordAccount>>,
    // Flat top-level fields (single-account form)
    pub token: Option<String>,
    #[serde(rename = "dmPolicy")]
    pub dm_policy: Option<String>,
    #[serde(rename = "allowFrom", default)]
    pub allow_from: Vec<String>,
    #[serde(rename = "groupPolicy")]
    pub group_policy: Option<String>,
    pub enabled: Option<bool>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct OpenClawDiscordAccount {
    pub token: Option<String>,
    #[serde(rename = "dmPolicy")]
    pub dm_policy: Option<String>,
    #[serde(rename = "allowFrom", default)]
    pub allow_from: Vec<String>,
    #[serde(rename = "groupPolicy")]
    pub group_policy: Option<String>,
    pub enabled: Option<bool>,
    pub name: Option<String>,
}
```

Change `OpenClawChannelsConfig.discord` from `Option<serde_json::Value>` to
`Option<OpenClawDiscordConfig>`.

#### `crates/openclaw-import/src/detect.rs`

Remove `"discord"` from `scan_unsupported_channels()` — it is now a supported
channel.

#### `crates/openclaw-import/src/channels.rs`

- Add `ImportedDiscordChannel` struct (account_id, bot_token, dm_policy,
  group_policy, allowlist)
- Add `discord: Vec<ImportedDiscordChannel>` to `ImportedChannels`
- Add Discord extraction logic in `import_channels()`, following the same
  accounts-map-then-flat-fallback pattern as Telegram
- Map openclaw's `"pairing"` dm_policy to moltis's `"allowlist"` (moltis does
  not yet have pairing)

#### `crates/openclaw-import/src/lib.rs`

- Add `discord_accounts` count to `ImportScan`
- Update `channels_available` to include Discord accounts
- Call `persist_discord_channels()` during import

Add `persist_discord_channels()` to write imported Discord accounts to
`[channels.discord.<id>]` in `moltis.toml`, and add `"discord"` to
`channels.offered` if not already present.

#### `crates/web/src/assets/js/onboarding-view.js`

- Show imported Discord account count in the scan preview (next to Telegram)
- Remove Discord from the "unsupported channels" display when accounts are found

#### Tests

- Parse flat Discord config
- Parse multi-account Discord config
- Skip disabled Discord accounts
- Unsupported channels no longer includes "discord"
- Persist round-trip to moltis.toml

---

## 2. Feature gap: openclaw vs moltis Discord

### Already implemented in moltis

| Feature | Notes |
|---------|-------|
| Gateway WebSocket (serenity) | Persistent connection, no public URL needed |
| DM + guild message handling | Full inbound pipeline |
| Bot mention detection/stripping | `<@bot_id>` and `<@!bot_id>` |
| Access control (DM/group policy, mention mode) | `allowlist`/`open`/`disabled` |
| Guild allowlist | Flat list of guild IDs |
| User allowlist for DMs | Discord usernames |
| Text chunking (2000 chars) | Markdown-aware, respects code fences |
| Media attachments | Base64 data URIs decoded to files |
| Typing indicators | `broadcast_typing` |
| Edit-in-place streaming | 3-phase: accumulate → send → throttled edits |
| Activity log embeds | Color-coded (red/green), tool error details |
| Multi-account | Per-account state map |
| Token secrecy | `Secret<String>`, redacted Debug |
| Web UI (onboarding + management) | Full setup flow, invite URL generation |
| Location sharing | Google Maps URL |

### Missing features (prioritized)

#### P0 — Critical for openclaw parity

**Pairing flow (DM policy: "pairing")**
- OpenClaw's default DM policy; users migrating will expect it
- Flow: unknown user DMs bot → bot sends OTP code → user approves via
  existing channel or CLI
- Moltis already has OTP/pairing for Telegram; needs Discord adapter
- Files: `crates/discord/src/handler.rs`, `crates/channels/src/gating.rs`

**Slash commands**
- OpenClaw registers native Discord slash commands (`/model`, `/help`, etc.)
- Requires `applications.commands` OAuth2 scope
- Serenity supports command registration via `Command::create_global_command`
- Files: new `crates/discord/src/commands.rs`

**Ack reactions**
- OpenClaw sends an emoji reaction (default: agent emoji or "👀") when
  processing starts, removes it when done
- Requires `Add Reactions` permission
- Serenity: `Message::react()` / `Message::delete_reaction()`
- Files: `crates/discord/src/handler.rs`

#### P1 — Important for usability

**Per-guild/per-channel configuration**
- OpenClaw supports `guilds.<id>.channels.<id>` with per-channel:
  tools, skills, users, roles, systemPrompt, requireMention
- Moltis has flat `guild_allowlist` only
- Files: `crates/discord/src/config.rs`, `crates/discord/src/handler.rs`

**Reply threading (replyToMode)**
- OpenClaw supports `off`/`first`/`all` — controls whether bot replies
  as Discord reply-to-message
- Serenity: `CreateMessage::reference_message()`
- Files: `crates/discord/src/outbound.rs`

**History context**
- OpenClaw injects recent channel history as context (`historyLimit`,
  `dmHistoryLimit`)
- Moltis uses session-scoped history only
- Files: `crates/discord/src/handler.rs`, chat engine

**Bot presence/activity**
- Custom status text, activity type (Playing/Streaming/Listening/Watching/
  Custom/Competing)
- Serenity: `Context::set_presence()`
- Files: `crates/discord/src/plugin.rs`

#### P2 — Medium-term enhancements

**Thread support**
- Forum channels, thread-bound sessions, `/focus`/`/unfocus`
- Auto-create threads for subagent sessions
- Each thread gets its own session key

**Reaction notifications**
- Forward emoji reactions to the agent as events
- Modes: `off`/`own`/`all`/`allowlist` per guild

**Interactive components (buttons, selects, modals)**
- Discord Components v2 containers
- Buttons with per-user authorization
- Select menus (string/user/role/channel)
- Modal forms with up to 5 fields
- Requires significant new infrastructure

**Exec approval buttons**
- Button-based approve/deny for exec tool calls
- Can be sent to DMs or originating channel
- Requires interactive components

**Voice channels**
- Join/leave voice channels, TTS playback
- `/vc join|leave|status` command
- Auto-join on startup
- Requires `songbird` or similar voice crate

**Per-guild role-based agent routing**
- Route guild members to different agents by Discord role ID
- Extends existing binding/routing system

**PluralKit support**
- Resolve proxied messages to system member identity
- Allowlists accept `pk:<memberId>`

**Voice messages (waveform)**
- OGG/Opus with waveform metadata
- Requires ffmpeg for audio conversion

**Block streaming modes**
- `block` mode: emit draft-sized chunks instead of editing one message
- Configurable `draftChunk` (minChars, maxChars, breakPreference)

**Proxy support**
- Route Discord gateway connections through HTTP(S) proxy
- `channels.discord.proxy` config field

**Config writes from chat**
- Allow the agent to update moltis.toml from Discord commands
- `channels.discord.configWrites` toggle

---

## Recommended implementation order

### Phase 1: Import + quick wins (this branch)

1. Import Discord tokens from openclaw (section 1 above)
2. Ack reactions (small, high visibility)
3. Reply-to-message support (`replyToMode: "first"`)

### Phase 2: Core parity

4. Pairing flow for DMs
5. Slash command registration (`/model`, `/help`, `/session`)
6. Per-guild/per-channel config with user/role allowlists
7. Bot presence/activity

### Phase 3: Advanced features

8. Thread support (forum channels, thread sessions)
9. History context injection
10. Reaction notifications
11. Interactive components (buttons, selects)
12. Exec approval buttons

### Phase 4: Specialized

13. Voice channels
14. Voice messages (waveform)
15. PluralKit support
16. Block streaming modes
17. Role-based agent routing
