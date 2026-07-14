# External Agents

Moltis can bind a chat session to an external CLI coding agent. When a session is bound, `chat.send` persists the user turn in Moltis, sends the prompt and recent session context to the external process, streams the CLI output back to the web UI, and persists the assistant response.

Supported agent kinds:

| Kind | Default command | Notes |
|------|-----------------|-------|
| `claude-code` | `claude -p --output-format json` | Print mode with `session_id` capture; later turns add `--resume <id>`. |
| `codex` | `codex app-server` | Persistent app-server process; Moltis reuses the Codex `threadId` across turns. |
| `acp` | `acp` | Persistent ACP JSON-RPC stdio session configured by `[external_agents.agents.acp]`. |

Enable the bridge in `moltis.toml`:

```toml
[external_agents]
enabled = true
# Disabled by default. When true, allowlisted channel users can use /tmux
# to inspect or send input to a live tmux pane.
channel_tmux_control = false

[external_agents.agents.claude-code]
binary = "claude"
timeout_secs = 300

[external_agents.agents.codex]
binary = "codex"

[external_agents.agents.acp]
binary = "/path/to/acp-agent"
args = []
```

The session header in the web UI exposes an external-agent selector when agents are configured. Select `Moltis agent` to unbind and return the session to the normal provider-backed Moltis agent.

Moltis keeps live external sessions in memory while the gateway process is running. Binding, unbinding, clearing, resetting, deleting, or clearing all sessions shuts down the matching live external process. Persisted external session IDs are stored in session metadata for UI/status visibility and for runtimes that can resume from their own IDs.

Current limitations:

- Claude Code persistence uses print-mode `--resume`; it does not yet keep an interactive PTY alive.
- ACP terminal requests run bounded commands and capture output; they are not interactive PTY sessions.
- `/tmux` channel control is an explicit opt-in for live terminal control. It requires `external_agents.enabled = true`, `external_agents.channel_tmux_control = true`, and an allowlisted channel sender.
- Live external processes are not restored automatically after a Moltis gateway restart.

Channel tmux control supports:

```text
/tmux status <target>
/tmux capture <target>
/tmux send <target> <text>
```

Use tmux target strings such as `moltis:0.1`, `@12`, or `%34`. Delivery returns an explicit receipt (`applied`, `busy`, or `unknown`) instead of assuming `tmux send-keys` means the application consumed the input.
