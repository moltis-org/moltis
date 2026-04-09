# Microsoft Teams

Moltis can connect to Microsoft Teams as a bot, letting you chat with your
agent from any Teams workspace, group chat, or direct message. The integration
uses the [Bot Framework](https://learn.microsoft.com/en-us/azure/bot-service/)
with an inbound webhook — your Moltis instance must be reachable from the
internet over HTTPS.

## How It Works

```
                          Microsoft Bot Service
                      (bot.botframework.com / Teams)
                                    │
                          HTTP POST (webhook)
                                    ▼
┌──────────────────────────────────────────────────────────────┐
│                      moltis-msteams crate                     │
│  ┌──────────────┐  ┌────────────┐  ┌──────────────────────┐  │
│  │  JWT + Secret │  │  Outbound  │  │      Plugin          │  │
│  │  Verification │  │ (replies,  │  │  (lifecycle, cards,  │  │
│  │  (auth)       │  │  streaming)│  │   attachments)       │  │
│  └──────────────┘  └────────────┘  └──────────────────────┘  │
└──────────────────────────┬───────────────────────────────────┘
                           │
                           ▼
┌──────────────────────────────────────────────────────────────┐
│                     Moltis Gateway                            │
│           (chat dispatch, tools, memory)                      │
└──────────────────────────────────────────────────────────────┘
```

Teams sends each user message as an HTTP POST to your webhook endpoint.
Moltis verifies the request (JWT signature and/or shared secret), processes
it, and replies via the Bot Framework REST API.

Streaming uses **edit-in-place** — an initial message is posted once enough
tokens arrive, then updated at a throttled interval until the response is
complete.

## Prerequisites

Before configuring Moltis you need to register a bot in Azure. There are two
approaches; the **Teams Developer Portal** route is quickest for most users.

### Option A: Teams Developer Portal (recommended)

1. Open the [Teams Developer Portal — Tools — Bot Management](https://dev.teams.microsoft.com/bots)
2. Click **+ New Bot**
3. Give the bot a name and click **Add**
4. On the bot's page, go to **Configure** and note the **Bot ID** (this is your `app_id`)
5. Under **Client secrets**, click **Add a client secret for your bot** and copy the generated value (this is your `app_password`)
6. Set the **Endpoint address** to your Moltis webhook URL (see [Webhook Endpoint](#webhook-endpoint) below)

### Option B: Azure Portal

1. Go to the [Azure Portal — Create a resource](https://portal.azure.com/#create/Microsoft.AzureBot)
2. Search for **Azure Bot** and click **Create**
3. Fill in a bot handle, subscription, and resource group
4. Under **Microsoft App ID**, select **Create new** (single-tenant is fine)
5. Click **Create** and wait for deployment
6. Go to the new bot resource, then **Configuration**:
   - Copy the **Microsoft App ID** (`app_id`)
   - Click **Manage Password** to go to **Certificates & secrets**
   - Click **+ New client secret**, copy the value (`app_password`)
7. Still in **Configuration**, set the **Messaging endpoint** to your Moltis webhook URL

### Install the bot in Teams

After creating the bot, you need to install it in your Teams organization:

1. In the [Teams Developer Portal](https://dev.teams.microsoft.com/), go to **Apps**
2. Click **+ New app**, give it a name, and fill in the required fields
3. Under **App features**, click **Bot** and select your existing bot
4. Choose the scopes: **Personal** (DMs), **Team** (channels), **Group Chat**
5. Click **Publish** → **Publish to your org** (or use **Preview in Teams** for testing)
6. In Teams, go to **Apps → Built for your org** and install the app

```admonish warning
The App Password is a secret — treat it like a password. Never commit it to
version control. Moltis stores it with `secrecy::Secret` and redacts it from
logs, but your `moltis.toml` file is plain text on disk. Consider using
[Vault](vault.md) for encryption at rest.
```

## Webhook Endpoint

Teams sends messages to a webhook URL on your server. The URL pattern is:

```
https://<your-domain>/api/channels/msteams/<account-id>/webhook?secret=<webhook-secret>
```

- **`<your-domain>`** — your public HTTPS domain (e.g. `bot.example.com`)
- **`<account-id>`** — the account identifier in your `moltis.toml` (e.g. `my-bot`)
- **`<webhook-secret>`** — an optional shared secret for additional verification

The Moltis web UI and CLI both generate this URL for you. Paste it into your
bot's **Messaging endpoint** field in the Azure Portal or Teams Developer Portal.

```admonish info title="HTTPS required"
Teams requires HTTPS. For local development, use a tunnel like
[ngrok](https://ngrok.com/) (`ngrok http 8080`) or
[Cloudflare Tunnel](https://developers.cloudflare.com/cloudflare-one/connections/connect-networks/).
```

## Configuration

Add a `[channels.msteams.<account-id>]` section to your `moltis.toml`:

```toml
[channels.msteams.my-bot]
app_id = "12345678-abcd-efgh-ijkl-000000000000"
app_password = "your-client-secret-here"
```

### Configuration Fields

| Field | Required | Default | Description |
|-------|----------|---------|-------------|
| `app_id` | **yes** | — | Azure App ID (Bot ID) from the bot registration |
| `app_password` | **yes** | — | Azure client secret for the bot |
| `tenant_id` | no | `"botframework.com"` | Azure AD tenant for JWT validation. Set to your tenant ID for single-tenant bots |
| `webhook_secret` | no | — | Shared secret appended as `?secret=...` to the webhook URL |
| `dm_policy` | no | `"allowlist"` | Who can DM the bot: `"open"`, `"allowlist"`, or `"disabled"` |
| `group_policy` | no | `"open"` | Who can talk to the bot in group chats / channels: `"open"`, `"allowlist"`, or `"disabled"` |
| `mention_mode` | no | `"mention"` | When the bot responds in groups: `"always"`, `"mention"`, or `"none"` |
| `allowlist` | no | `[]` | AAD object IDs or user IDs allowed to DM the bot |
| `group_allowlist` | no | `[]` | Conversation/team IDs allowed for group messages |
| `model` | no | — | Override the default model for this channel |
| `model_provider` | no | — | Provider for the overridden model |
| `stream_mode` | no | `"edit_in_place"` | `"edit_in_place"` (live updates) or `"off"` (send once complete) |
| `edit_throttle_ms` | no | `1500` | Minimum milliseconds between streaming edits |
| `text_chunk_limit` | no | `4000` | Maximum characters per message before splitting |
| `reply_style` | no | `"top_level"` | `"top_level"` or `"thread"` (reply in thread) |
| `welcome_card` | no | `true` | Send an Adaptive Card welcome message in DMs |
| `group_welcome_card` | no | `false` | Send a welcome message when the bot is added to a group |
| `bot_name` | no | `"Moltis"` | Display name shown on welcome cards |
| `prompt_starters` | no | `[]` | Prompt starter buttons on the welcome card |
| `max_retries` | no | `3` | Max retry attempts for failed sends |
| `retry_base_delay_ms` | no | `250` | Base delay (ms) for exponential backoff |
| `retry_max_delay_ms` | no | `10000` | Maximum retry delay (ms) |
| `history_limit` | no | `50` | Max messages fetched for thread context (requires Graph API permissions) |
| `graph_tenant_id` | no | — | Tenant ID for Graph API operations (thread history, reactions, search) |

### Full Example

```toml
[channels]
offered = ["telegram", "msteams", "discord", "slack"]

[channels.msteams.my-bot]
app_id = "12345678-abcd-efgh-ijkl-000000000000"
app_password = "your-client-secret-here"
tenant_id = "your-azure-tenant-id"
webhook_secret = "a-long-random-secret"
dm_policy = "allowlist"
group_policy = "open"
mention_mode = "mention"
allowlist = ["00000000-0000-0000-0000-000000000001"]
stream_mode = "edit_in_place"
edit_throttle_ms = 1500
welcome_card = true
bot_name = "My Assistant"
prompt_starters = ["What can you do?", "Help me write an email"]
reply_style = "top_level"
model = "claude-sonnet-4-20250514"
model_provider = "anthropic"
```

### Per-Team / Per-Channel Overrides

You can override settings for specific Teams teams or channels:

```toml
[channels.msteams.my-bot.teams.team-id-here]
reply_style = "thread"
mention_mode = "always"

[channels.msteams.my-bot.teams.team-id-here.channels.general-channel-id]
model = "gpt-4o"
model_provider = "openai"
mention_mode = "mention"
```

## Access Control

Teams uses the same gating system as other channels:

### DM Policy

Controls who can send direct messages to the bot.

| Value | Behavior |
|-------|----------|
| `"allowlist"` | Only users listed in `allowlist` can DM (default) |
| `"open"` | Anyone who can reach the bot can DM it |
| `"disabled"` | DMs are silently ignored |

### Group Policy

Controls who can interact with the bot in group chats and team channels.

| Value | Behavior |
|-------|----------|
| `"open"` | Bot responds in any group chat (default) |
| `"allowlist"` | Only conversations listed in `group_allowlist` are allowed |
| `"disabled"` | Group messages are silently ignored |

### Mention Mode

Controls when the bot responds in group chats (does not apply to DMs).

| Value | Behavior |
|-------|----------|
| `"mention"` | Bot only responds when @mentioned (default) |
| `"always"` | Bot responds to every message in allowed groups |
| `"none"` | Bot never responds in groups (DM-only) |

### Finding User IDs

To add users to the allowlist, you need their **AAD Object ID**. You can find
this in:

- **Azure Portal** → Azure Active Directory → Users → select user → Object ID
- **Microsoft 365 Admin Center** → Users → Active users → select user → Properties
- **Teams Admin Center** → Users → select user

Alternatively, set `dm_policy = "open"` initially and check the Moltis web UI
under **Channels → Senders** to see user IDs as messages arrive.

## Streaming

By default, Teams uses **edit-in-place** streaming:

1. After ~20 characters accumulate, an initial message is posted with "..."
2. Every 1.5 seconds (configurable via `edit_throttle_ms`), the message is
   edited with the latest accumulated text
3. When the response is complete, a final edit removes the "..." suffix

Set `stream_mode = "off"` to disable streaming and send the complete response
as a single message.

```toml
[channels.msteams.my-bot]
stream_mode = "edit_in_place"
edit_throttle_ms = 1500
```

## Welcome Cards

When a user first messages the bot in a DM, Moltis sends an
[Adaptive Card](https://learn.microsoft.com/en-us/adaptive-cards/) with a
greeting and optional prompt starter buttons:

```toml
[channels.msteams.my-bot]
welcome_card = true
bot_name = "My Assistant"
prompt_starters = ["What can you do?", "Help me write an email", "Summarize this document"]
```

Set `welcome_card = false` to disable. Set `group_welcome_card = true` to also
send a text welcome when the bot is added to a group chat.

```admonish note
Welcome card tracking is in-memory and resets when the gateway restarts. After
a restart, the bot may re-send welcome cards to conversations that already
received one. This is a known limitation.
```

## Interactive Messages

The bot supports Adaptive Cards for interactive button menus. When an agent
returns an interactive message (buttons), Moltis renders it as an Adaptive
Card with `Action.Submit` buttons. Clicking a button sends the callback data
back to the bot as a regular message.

## Attachments

### Inbound

When users send images or files in Teams, Moltis downloads the attachments
using the bot's access token and passes them to the LLM as multimodal content
(if the model supports it).

### Outbound

- **Images in DMs**: sent inline as base64 data URLs
- **External URLs**: sent as Bot Framework URL attachments
- **Large files**: currently not supported (requires SharePoint/OneDrive integration)

## Thread Context (Graph API)

With Graph API permissions, the bot can read conversation history for
multi-turn context in group chats. This requires:

1. An **app registration** with `Chat.Read.All` or `ChannelMessage.Read.All`
   permissions (application-level, not delegated)
2. **Admin consent** for these permissions in your Azure AD tenant
3. Set `graph_tenant_id` in the config

```toml
[channels.msteams.my-bot]
graph_tenant_id = "your-azure-tenant-id"
history_limit = 50
```

## Reactions

The bot supports adding and removing reactions (like, heart, laugh, surprised,
sad, angry) via the Graph API beta endpoints. This requires the same Graph API
permissions as thread context.

## Web UI Setup

You can configure Teams through the web interface:

1. Open **Settings → Channels**
2. Click **Connect Microsoft Teams**
3. Enter your **App ID** and **App Password** from the Azure bot registration
4. Optionally enter a **Webhook Secret** (one is generated if left blank)
5. Set the **Public Base URL** to your Moltis server's HTTPS URL
6. Click **Bootstrap Teams** to generate the messaging endpoint
7. Click **Copy Endpoint** and paste it into your bot's Messaging Endpoint in the Azure Portal or Teams Developer Portal
8. Adjust DM policy, mention mode, and allowlist as needed
9. Click **Connect Microsoft Teams**

The same form is available during onboarding when `"msteams"` is in
`channels.offered`.

## CLI Setup

Use the CLI bootstrap command for a quick setup:

```bash
moltis channels teams bootstrap \
  --account-id my-bot \
  --app-id 12345678-abcd-efgh-ijkl-000000000000 \
  --app-password your-client-secret \
  --base-url https://bot.example.com
```

This generates the webhook endpoint URL and writes the configuration to
`moltis.toml`. Add `--dry-run` to preview without saving, or `--open` to
launch the Azure documentation in your browser.

## Crate Structure

```
crates/msteams/
├── Cargo.toml
└── src/
    ├── lib.rs                       # Public exports
    ├── activity.rs                  # Teams activity model & parsing
    ├── attachments.rs               # Inbound download + outbound media
    ├── auth.rs                      # OAuth2 token acquisition + caching
    ├── cards.rs                     # Adaptive Card builders (welcome, polls)
    ├── channel_webhook_verifier.rs  # Shared-secret webhook verification
    ├── chunking.rs                  # Message text chunking
    ├── config.rs                    # MsTeamsAccountConfig + per-team overrides
    ├── errors.rs                    # Error classification + retry logic
    ├── graph.rs                     # Graph API (history, reactions, search, pins)
    ├── jwt.rs                       # Bot Framework JWT validation (JWKS)
    ├── outbound.rs                  # ChannelOutbound + streaming + reactions
    ├── plugin.rs                    # ChannelPlugin + ChannelThreadContext
    ├── state.rs                     # AccountState + JWT validator
    └── streaming.rs                 # Edit-in-place streaming session
```

The crate implements the same trait set as other channel crates:

| Trait | Purpose |
|-------|---------|
| `ChannelPlugin` | Start/stop accounts, lifecycle management |
| `ChannelOutbound` | Send text, media, cards, typing indicators, reactions |
| `ChannelStreamOutbound` | Handle streaming responses (edit-in-place) |
| `ChannelStatus` | Health probes (connected / waiting for first activity) |
| `ChannelThreadContext` | Fetch conversation history via Graph API |

## Troubleshooting

### Bot doesn't respond

- Verify the **Messaging endpoint** in the Azure Portal matches your Moltis webhook URL
- Check that `app_id` and `app_password` are correct
- Ensure your server is reachable from the internet over HTTPS
- Check `dm_policy` — if set to `"allowlist"`, make sure your user ID is listed
- Check `mention_mode` — in group chats, you may need to @mention the bot
- Look at logs: `RUST_LOG=moltis_msteams=debug moltis`

### "Teams token acquisition failed"

- The `app_password` may have expired — rotate the client secret in the Azure Portal
- Check that `oauth_tenant` and `oauth_scope` are correct (defaults should work for most setups)
- Network issues: Moltis must be able to reach `login.microsoftonline.com`

### Webhook returns 401 / 403

- If using JWT validation: check that `tenant_id` matches your Azure AD tenant
- If using shared secret: check that the `?secret=...` in the webhook URL matches `webhook_secret` in the config
- The JWKS endpoint (`login.botframework.com`) must be reachable for JWT validation

### Bot responds in DMs but not in groups

- Check `mention_mode` — if set to `"mention"`, you must @mention the bot
- Check `group_policy` — if `"disabled"`, group messages are ignored
- Check `group_allowlist` — if non-empty, the group/team must be listed

### Messages are duplicated

- Moltis returns `202 Accepted` immediately to prevent Teams retry timeouts.
  If you see duplicates, check for multiple Moltis instances pointing at the
  same webhook URL, or verify the deduplication middleware is working
  (duplicate activity IDs are filtered automatically)

### Streaming doesn't work

- Set `stream_mode = "edit_in_place"` (the default)
- Streaming requires the bot to have permission to update activities
- Some Teams clients (older mobile versions) may not render edits in real time
