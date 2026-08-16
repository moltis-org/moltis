# Slack Slash Command Manifest Snippet

Date: 2026-07-23

## Problem

Moltis has Slack slash-command handlers for commands like `/new`, `/model`,
`/stop`, and `/help`, but Slack only sends those payloads if each slash command
is registered in the Slack app manifest. If a user types `/new` in a Slack DM
without the command registered, Slack handles it itself instead of delivering a
message or command event to Moltis.

Moltis already has a command manifest generator in `crates/slack/src/commands.rs`,
but the Slack channel setup flow does not surface it where users need it.

## Goals

1. Show a copyable Slack manifest snippet when adding or editing a Slack channel.
2. Use Moltis' configured public URL for webhook-based Slack channels.
3. Use a valid placeholder URL for Socket Mode channels, with clear wording that
   Slack requires the field but Socket Mode delivery does not use it.
4. Make it obvious that slash commands are optional but recommended and require
   updating/reinstalling the Slack app.
5. Keep the generated command list sourced from `moltis_channels::commands` so it
   stays in sync with supported channel commands.

## Non-Goals

1. Automatically registering Slack slash commands at runtime. Slack does not
   support this the way Discord does.
2. Making text messages like `/new` in Slack DMs bypass Slack command handling.
   Slack intercepts slash-prefixed input before Moltis sees it.
3. Adding a second command registry specific to Slack.

## Design

### Manifest generation

Reuse `moltis_slack::commands::generate_manifest_snippet(request_url_base)` as
the source of truth. The generated YAML should continue to derive commands from
`moltis_channels::commands::all_commands()`.

For webhook / Events API mode, use Moltis' public base URL:

```yaml
slash_commands:
  - command: /new
    url: https://moltis.example.com/api/channels/slack/{{account_id}}/commands
    description: "Start a new session"
    usage_hint: ""
    should_escape: false
```

For Socket Mode, use a valid-looking placeholder URL because Slack manifests
still require `url` in slash command definitions even though delivery happens via
Socket Mode:

```yaml
slash_commands:
  - command: /new
    url: https://example.com/moltis-socket-mode-placeholder
    description: "Start a new session"
    usage_hint: ""
    should_escape: false
```

The UI copy should explain:

> Slack requires a URL for slash commands in the app manifest. When Socket Mode
> is enabled, Slack delivers these commands over Socket Mode and this placeholder
> URL is not used.

### UI placement

In the Slack channel add/edit flow, add a section after the connection mode and
account ID are known:

1. Title: `Slack slash commands`
2. Short explanation: `Paste this into your Slack app manifest under features.slash_commands, save, then reinstall the app.`
3. Copyable code block with the generated snippet.
4. Checklist:
   - Paste under `features.slash_commands`.
   - Save the manifest.
   - Reinstall or update the Slack app in the workspace.
   - Try `/help` or `/new` in Slack.

If the public URL is missing for webhook mode, show a warning and either disable
the generated snippet or render it with `https://YOUR_MOLTIS_PUBLIC_URL` so the
user knows what must be configured.

### API shape

Expose the generated snippet through whichever layer currently backs channel
configuration UI. Prefer a small Slack-specific helper over duplicating manifest
formatting in the frontend.

Suggested response fields:

```json
{
  "slash_commands_manifest": "slash_commands:\n  - command: /new\n...",
  "slash_commands_url_mode": "webhook",
  "slash_commands_note": "Paste into features.slash_commands and reinstall the Slack app."
}
```

Use `slash_commands_url_mode = "socket_mode_placeholder"` when the channel is in
Socket Mode.

## Implementation Plan

1. Locate the Slack channel add/edit UI and the backend endpoint/model that feeds
   it.
2. Add a backend helper that computes the request URL base from:
   - Slack account ID.
   - Slack connection mode.
   - Moltis configured public URL for webhook mode.
   - A constant placeholder URL for Socket Mode.
3. Thread the generated manifest snippet into the Slack channel setup response or
   view model.
4. Add a copyable code block and checklist in the Slack channel setup UI.
5. Add tests for:
   - Webhook mode uses the configured public URL and account ID.
   - Socket Mode uses the placeholder URL.
   - Generated snippet includes `/new`, `/model`, and `/help`.
   - Missing public URL in webhook mode produces a clear warning or placeholder.

## Files To Inspect

- `crates/slack/src/commands.rs` — manifest generation.
- `crates/slack/src/socket.rs` — Socket Mode slash command handling.
- `crates/slack/src/webhook.rs` — webhook slash command handling.
- `crates/httpd/src/server/gateway.rs` — Slack command webhook route.
- Slack channel add/edit UI files under `crates/web/ui`.
- Config schema for the public/external URL used by Moltis.

## Manual QA

1. Add a Slack channel in Socket Mode.
2. Confirm the setup screen shows a manifest snippet with placeholder URLs.
3. Paste commands into the Slack app manifest and reinstall the app.
4. Type `/help` in a Slack DM and confirm Moltis responds with command help.
5. Type `/new` in a Slack DM and confirm Moltis creates a new session.
6. Repeat with webhook mode and confirm generated URLs point at the configured
   public Moltis URL plus `/api/channels/slack/{account_id}/commands`.
