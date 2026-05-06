# Channel Integration Checklist

Use this when adding a new channel crate or bringing an existing one to parity.

## Minimum product bar

- The channel must be configurable from the web UI.
- The onboarding flow must expose the same core settings needed for first-run success.
- Any channel-specific settings not modeled in dedicated fields must still be reachable through the shared advanced JSON config editor.
- The UI must state clearly that channel settings added or edited in the web UI are stored in `data_dir()/moltis.db`, not written back to `moltis.toml`.
- Editing a channel from the UI must preserve omitted settings and redacted secrets instead of silently wiping them.

## Backend checklist

- Add the crate under `crates/<channel>/`.
- Add dependencies through workspace dependencies only.
- Expose `tracing` and `metrics` features, gate instrumentation with those features.
- Register the plugin in the channel registry and descriptor surfaces.
- Implement config parsing with typed structs, sane defaults, and redacted JSON output.
- Update config schema, semantic validation, and the config template.
- Add account lifecycle support: start, stop, status, config export, runtime update.
- Ensure runtime update paths merge config safely instead of replacing whole objects.

## Feature inventory across current channels

- Inbound connection modes: `none`, polling, persistent gateway loop, Socket Mode, and webhook.
- Outbound text plus basic rich text or HTML rendering.
- Configurable streaming responses.
- Interactive flows, native in some channels and fallback-driven in others.
- Reply support and thread support where the platform has a thread model.
- Voice or audio ingest in Telegram, WhatsApp, and Matrix.
- Pairing in WhatsApp.
- OTP approval flow for unknown DM users in Telegram, WhatsApp, and Matrix.
- Reaction support in Slack and Matrix.
- Location send or receive support in Telegram, Microsoft Teams, Discord, and Matrix.
- Access control patterns: DM policy, group or room policy, allowlists, mention mode, auto-join or equivalent.
- Web UI support: add, edit, onboarding, icons, setup help, advanced JSON patch editor.

## Must-have features

- Connect, disconnect, status, and runtime update support.
- Typed config with defaults, redacted JSON export, and safe merge semantics on update.
- Inbound text handling and outbound text replies.
- No silent failures for access control, unsupported message types, or provider errors.
- DM policy plus allowlist support, and group or room activation controls when the platform supports shared spaces.
- Web UI add and edit flows.
- Onboarding support if the channel is offered there.
- Advanced JSON config editor support in the web UI so unmodeled settings remain reachable.
- Clear UI and docs note that web-managed channel settings are stored in `data_dir()/moltis.db`, not written back to `moltis.toml`.
- Config template, schema validation, docs, and tests updated in the same PR.
- Unit coverage for config, access control, and any channel-specific parsing or merge logic.

## Nice-to-have features

- HTML or markdown rendering with a sane plain-text fallback.
- Streaming mode controls such as throttle, initial delay, and completion behavior.
- Media upload and inbound media handling.
- Native reply and thread support.
- Interactive actions, native when possible and deliberate fallback when not.
- Reactions, add and remove.
- Voice ingest.
- Location send and receive.
- Pairing or OAuth bootstrap for channels that support it.
- Per-room, per-channel, or per-user model overrides.
- Rich onboarding help, setup links, and smart defaults like real public-server placeholders.

## Web UI checklist

- Add flow in `Settings -> Channels`.
- Edit flow in `Settings -> Channels`.
- Onboarding flow support if the channel is offered there.
- Channel-specific labels, icons, and validation help.
- Access-token or credential guidance where needed.
- Tests for at least one happy-path add flow and one advanced-config path.
- Preferred direction: model channel settings declaratively so HTML form fields and advanced JSON help can come from the same source instead of hand-maintaining parallel UIs.

## Docs checklist

- User-facing docs page for the channel under `docs/src/` when appropriate.
- Update `docs/src/channels.md`.
- Update `docs/src/SUMMARY.md` when adding a new page.
- Keep the config template comments in sync with the real settings.

## Tests checklist

- Unit tests for config defaults and validation.
- Unit tests for access control and OTP behavior.
- Unit tests for any parsing helpers, thread/reaction/media helpers, and merge logic.
- Web UI E2E coverage for add or edit flows.
- Run the targeted Rust tests, JS formatting/linting, and relevant Playwright specs.

## Matrix-specific lessons worth reusing

- Do not assume UI edits send a full config object. Preserve omitted fields server-side.
- Redacted secrets from status JSON must not overwrite the stored secret on save.
- Auto-generated account IDs reduce setup friction for channels that already have stable native IDs.
- Public-server defaults matter. A real placeholder like `https://matrix.org` is much better than a fake example.
- If the channel has a DM approval or OTP flow, expose it in both onboarding and settings. Users expect the bot to answer on first contact.
