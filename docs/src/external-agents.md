# External Agents

Moltis can bind a chat session to an external CLI coding agent. When a session is bound, `chat.send` persists the user turn in Moltis, sends the prompt and recent session context to the external process, streams the CLI output back to the web UI, and persists the assistant response.

ACP, the Agent Client Protocol, is a JSON-RPC protocol for connecting editor or coding agents to a host application. Moltis can run ACP-compatible command-line agents as external agents, route their permission prompts through Moltis approvals, and show them in the session header selector as `ACP: <agent>`. The canonical ACP agent catalog is https://agentclientprotocol.com/get-started/agents.

Supported agent kinds:

| Kind | Default command | Notes |
|------|-----------------|-------|
| `claude-code` | `claude -p --output-format json` | Print mode with `session_id` capture; later turns add `--resume <id>`. |
| `codex` | `codex app-server` | Persistent app-server process; Moltis reuses the Codex `threadId` across turns. |
| `acp` | `acp` | Persistent ACP JSON-RPC stdio session configured by `[external_agents.agents.acp]`. |
| `acp-copilot` | `copilot --acp` | Named ACP session shown as `ACP: Copilot` in the session header. |
| `acp-codex` | `codex-acp` | Codex via Zed's ACP adapter, shown as `ACP: Codex`. |
| `acp-claude` | `claude-agent-acp` | Claude Agent SDK via https://github.com/agentclientprotocol/claude-agent-acp, shown as `ACP: Claude`. |
| `acp-pi` | `pi-acp` | Pi via the `pi-acp` adapter, shown as `ACP: Pi`. |
| `acp-opencode` | `opencode acp` | Named ACP session shown as `ACP: opencode` in the session header. |
| `acp-gemini` | `gemini --experimental-acp` | Named ACP session shown as `ACP: Gemini` in the session header. |
| `acp-augment` | `auggie --acp` | Augment/Auggie ACP mode. |
| `acp-kiro` | `kiro-cli acp` | Kiro CLI ACP mode. |
| `acp-openclaw` | `openclaw acp` | OpenClaw ACP bridge. |
| `acp-openhands` | `openhands acp` | OpenHands CLI ACP mode. |
| `acp-kimi` | `kimi acp` | Kimi CLI ACP mode. |
| `acp-stakpak` | `stakpak acp` | Stakpak ACP mode. |
| `acp-fast-agent` | `fast-agent-acp` | fast-agent ACP server. |

## Default ACP detection

External agent discovery is enabled by default. On startup, Moltis checks for the built-in ACP agent commands on `$PATH`:

| Selector label | Config key | Command checked |
|----------------|------------|-----------------|
| `ACP: Copilot` | `acp-copilot` | `copilot --acp` |
| `ACP: Codex` | `acp-codex` | `codex-acp` |
| `ACP: Claude` | `acp-claude` | `claude-agent-acp` |
| `ACP: Pi` | `acp-pi` | `pi-acp` |
| `ACP: opencode` | `acp-opencode` | `opencode acp` |
| `ACP: Gemini` | `acp-gemini` | `gemini --experimental-acp` |
| `ACP: Augment` | `acp-augment` | `auggie --acp` |
| `ACP: Kiro` | `acp-kiro` | `kiro-cli acp` |
| `ACP: OpenClaw` | `acp-openclaw` | `openclaw acp` |
| `ACP: OpenHands` | `acp-openhands` | `openhands acp` |
| `ACP: Kimi` | `acp-kimi` | `kimi acp` |
| `ACP: Stakpak` | `acp-stakpak` | `stakpak acp` |
| `ACP: fast-agent` | `acp-fast-agent` | `fast-agent-acp` |

Installed ACP agents appear automatically in each chat session's external-agent selector. Missing commands are hidden from the selector, so a fresh install with no ACP agents available continues to show only the normal Moltis agent.

Default detection only checks whether the named command exists on Moltis' `$PATH`; it does not verify the binary publisher or installation source. Only install ACP agents from trusted sources, keep untrusted directories out of the service `$PATH`, and use explicit `binary = "/absolute/path/to/agent"` overrides when you want to pin the executable Moltis may launch after a user selects that agent for a session.

If you want to disable all external agent discovery, set:

```toml
[external_agents]
enabled = false
```

After changing `moltis.toml`, restart Moltis so the gateway reloads the external agent registry.

## Add ACP agents

Use the named ACP keys when you need to override the auto-detected defaults. Each entry can override the executable path, startup args, environment variables, working directory, timeout, and tmux behavior.

```toml
[external_agents]
enabled = true

[external_agents.agents.acp-copilot]
binary = "copilot"                  # or an absolute path
args = ["--acp"]

[external_agents.agents.acp-codex]
binary = "codex-acp"
args = []

[external_agents.agents.acp-claude]
binary = "claude-agent-acp"          # npm package: @agentclientprotocol/claude-agent-acp
args = []

[external_agents.agents.acp-pi]
binary = "pi-acp"
args = []

[external_agents.agents.acp-opencode]
binary = "opencode"
args = ["acp"]

[external_agents.agents.acp-gemini]
binary = "gemini"
args = ["--experimental-acp"]

[external_agents.agents.acp-augment]
binary = "auggie"
args = ["--acp"]

[external_agents.agents.acp-kiro]
binary = "kiro-cli"
args = ["acp"]

[external_agents.agents.acp-openclaw]
binary = "openclaw"
args = ["acp"]

[external_agents.agents.acp-openhands]
binary = "openhands"
args = ["acp"]

[external_agents.agents.acp-kimi]
binary = "kimi"
args = ["acp"]

[external_agents.agents.acp-stakpak]
binary = "stakpak"
args = ["acp"]

[external_agents.agents.acp-fast-agent]
binary = "fast-agent-acp"
args = []
```

Claude ACP support is not provided by plain `claude`; Moltis detects the separate `claude-agent-acp` adapter binary. Install it from the upstream package/repository and ensure that binary is on the Moltis service `$PATH`, or set `binary = "/absolute/path/to/claude-agent-acp"`.

Cursor CLI supports ACP with `agent acp`, but Moltis does not auto-detect it because `agent` is a generic executable name that can collide with unrelated tools. Configure Cursor manually with the generic `acp` entry and an absolute `binary` path if you want to use it.

For advanced/manual ACP servers that are not one of the named options, use the generic `acp` kind:

```toml
[external_agents]
enabled = true

[external_agents.agents.acp]
binary = "/path/to/acp-agent"
args = ["--stdio"]
```

You can pass additional environment variables to an ACP agent without putting them in the global process environment:

```toml
[external_agents.agents.acp-copilot]
binary = "/opt/acp/copilot-agent"
working_dir = "/srv/my-project"
timeout_secs = 300

[external_agents.agents.acp-copilot.env]
COPILOT_AGENT_MODE = "workspace"
```

## Add native CLI agents

Moltis also supports native, non-ACP external agent integrations:

```toml
[external_agents]
enabled = true

[external_agents.agents.claude-code]
binary = "claude"
timeout_secs = 300

[external_agents.agents.codex]
binary = "codex"
```

## Select an ACP agent for a session

The session header in the web UI exposes an external-agent selector when agents are configured. ACP entries are labeled with the protocol and agent name, such as `ACP: Copilot`. Select `Moltis agent` to unbind and return the session to the normal provider-backed Moltis agent.

Binding is per session. You can bind one chat session to `ACP: Copilot`, another to `ACP: Claude`, and leave other sessions on the normal Moltis agent.

Moltis keeps live external sessions in memory while the gateway process is running. Binding, unbinding, clearing, resetting, deleting, or clearing all sessions shuts down the matching live external process. Persisted external session IDs are stored in session metadata for UI/status visibility and for runtimes that can resume from their own IDs.

## ACP permissions and capabilities

ACP agents can ask Moltis for permission before running tools. Moltis converts ACP permission requests into normal Moltis approval prompts and selects the matching ACP allow or reject option based on the user decision.

Moltis advertises ACP file-system and terminal capabilities to agents. File reads and writes are handled through the ACP client bridge. Terminal requests are supported by the ACP bridge but run inside the Moltis gateway environment, so configure `working_dir` and environment variables deliberately.

## Troubleshooting ACP agents

- If an ACP agent does not appear in the selector, confirm `[external_agents] enabled = true`, restart Moltis, and verify the configured `binary` exists on `$PATH` or is an absolute path.
- If an ACP entry appears as unavailable, run the configured command manually from the same shell or service environment that starts Moltis.
- If the wrong ACP agent is bound, use the session header selector and choose the desired `ACP: <agent>` entry; choose `Moltis agent` to unbind.
- If the agent needs project-local context, set `working_dir` in that agent's config entry.

Current limitations:

- Claude Code persistence uses print-mode `--resume`; it does not yet keep an interactive PTY alive.
- Live external processes are not restored automatically after a Moltis gateway restart.
