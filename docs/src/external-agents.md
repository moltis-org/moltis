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
- ACP terminal capability is not enabled yet; ACP servers can read/write text files through the client bridge, but terminal requests are rejected.
- Live external processes are not restored automatically after a Moltis gateway restart.
