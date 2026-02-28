# Settings Terminal: Multi tmux Sessions/Windows/Panes Plan

Date: 2026-02-17
Owner: gateway/web-ui
Status: planned

## Problem
Current `Settings > Terminal` attaches to one host PTY (and optionally one tmux session), which is good for a single stream but weak for parallel workflows. Users need to:
- run multiple long-lived tasks at once,
- switch between tmux targets quickly,
- keep a clean browser UX (tabs), not manual tmux key choreography.

## Goals
- Add terminal tabs in web UI that map to tmux targets.
- Support multi-session, multi-window, and pane-level attach.
- Keep current authentication and origin checks as strict as chat WS.
- Preserve current fallback behavior when tmux is missing.

## Preferred UX direction
- Primary UX should focus on a single managed tmux session: `moltis-host-terminal`.
- Browser tabs should map to tmux windows in that session.
- Non-expert users should not need to know `Ctrl-b c`, `n`, or `p`.

## Non-goals (phase 1)
- Full tmux control-mode rendering in browser.
- Replacing xterm.js.
- Editing `.tmux.conf` from UI.

## Architecture

### 1) Terminal target model
Introduce an explicit target descriptor:
- `kind`: `ephemeral` | `tmux`
- `session`: string (tmux only)
- `window`: optional string/index
- `pane`: optional string/index
- `label`: user-facing name for tab

A tab is a `(target, wsConnection, xtermInstance)` tuple.

Phase-1 constraint:
- `session` is fixed to `moltis-host-terminal`.
- Tabs represent tmux windows only.

### 2) Backend API split
Keep streaming on WS, add lightweight REST discovery/control:
- `GET /api/terminal/targets`
  - returns tmux topology and defaults.
- `POST /api/terminal/targets`
  - create session/window/pane targets.
- `POST /api/terminal/targets/select`
  - validates target and returns normalized descriptor.

Continue using `/api/terminal/ws` for PTY byte streaming.
WS connection includes target in first client message:
- `{ "type": "attach", "target": { ... } }`

Server validates target and spawns the corresponding PTY command.

Phase-1 simplified API (window-first):
- `GET /api/terminal/windows` (for `moltis-host-terminal`)
- `POST /api/terminal/windows` (create new window/tab)
- `POST /api/terminal/windows/{id}/attach`
- Optional `DELETE /api/terminal/windows/{id}` (close window)

### 3) PTY spawn strategy
- `tmux available`: spawn client per tab targeting requested session/window/pane.
- `tmux missing`: allow only one `ephemeral` target; show install hint.
- Keep resize forwarding (already implemented) and mirror to tmux window size.

### 4) UI model
In `Settings > Terminal`:
- top tab strip with:
  - active tmux-window tabs (same `moltis-host-terminal` session),
  - `+` button for new tmux window,
  - optional overflow dropdown for many windows.
- each tab owns one xterm + websocket.
- closing a tab closes only browser PTY client, not tmux session by default.

## UX behavior
- First load:
  - if tmux available: open default tab for `moltis-host-terminal`.
  - if absent: open one ephemeral tab and show “Install tmux for persistence”.
- New tab:
  - quick options: new tmux window, new pane (split), attach existing target, ephemeral shell.
- Target switching:
  - done by selecting another tab (no tearing the active session down).
- Size indicator:
  - each tab shows live `cols×rows`.

## Security and auth
- Keep existing same-origin WS enforcement.
- Keep authenticated-header checks for WS and new REST routes.
- Enforce strict target validation server-side (never pass unchecked strings to shell).
- Add tests for unauthorized/cross-origin on new terminal REST and WS attach flow.

## Data and state
- Persist open tab descriptors in localStorage (UI convenience only).
- Do not persist secrets.
- If a persisted tmux target no longer exists, mark tab stale and offer reconnect/create.

## Implementation phases

### Phase 1: Target-aware backend and single-active tab UI
- Lock to `moltis-host-terminal` and windows-only tabs.
- Add `/api/terminal/windows` discovery/create/attach endpoints.
- Add WS `attach_window` handshake payload.
- UI: tab strip with one active terminal at a time.
- `+` creates a new tmux window and opens it as a new tab.

### Phase 2: Multi-live tabs
- Support multiple concurrent xterm+ws terminals.
- background tabs keep connection alive (configurable).
- add per-tab status and reconnect states.

### Phase 3: tmux management actions
- Expand from windows-only to sessions/panes as optional advanced mode.
- create/rename/kill session/window/pane actions.
- optional pane split presets (horizontal/vertical).

## Test plan
- Rust unit tests:
  - target parser/validator,
  - tmux command builder from descriptor,
  - auth guards on new routes.
- Integration tests:
  - WS attach success/failure by target type,
  - resize behavior per tab.
- Playwright:
  - create tab, attach existing target, switch tabs,
  - stale target recovery,
  - tmux-missing fallback UX.

## Open design decisions
- Whether inactive tabs should stay live by default (resource cost) or auto-suspend.
- Whether pane-level attach should be exposed immediately or only sessions/windows first.
- Whether to add keyboard shortcuts for tab switching (`Ctrl+1..9`, `Ctrl+Tab`).

## Rollout
- Behind a feature flag (`web_ui_terminal_tabs`) first.
- Enable by default after one release cycle if telemetry/error rate is stable.
