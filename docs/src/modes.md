# Modes

Modes are session-scoped prompt overlays. They change how the currently selected
chat agent works right now without creating a new agent, changing memory, or
changing sub-agent presets.

Use modes when you want a temporary working style:

| Mode | Use it for |
|------|------------|
| `concise` | Short, direct answers |
| `technical` | Detailed technical analysis |
| `creative` | Broad ideation and alternatives |
| `teacher` | Step-by-step explanation |
| `plan` | Scoping and sequencing before implementation |
| `build` | Implementation-focused work |
| `review` | Bug-focused code review |
| `research` | Evidence-first investigation |
| `elevated` | Extra caution for risky operations |

Switch the active session from the web chat input or any channel that supports
regular slash commands:

```text
/mode
/mode review
/mode 3
/mode none
```

`/mode` lists available modes. `/mode none` clears the overlay.

## Configure Modes

Built-in modes are available on every install. Add or override modes in
`moltis.toml`:

```toml
[modes.presets.incident]
name = "Incident"
description = "production incident response"
prompt = "Prioritize impact, timeline, mitigation, rollback, logs, and clear status updates."
```

Mode presets are intentionally small. For durable identity, memory, and chat
history, create a chat agent. For delegated work through `spawn_agent`, use an
agent preset.
