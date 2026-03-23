# Agent Presets

Agent presets let `spawn_agent` run sub-agents with role-specific configuration.
Use them to control model cost, tool access, session visibility, and behavior.

## Quick Start

```toml
[agents.presets.researcher]
identity.name = "scout"
identity.emoji = "🔍"
identity.theme = "thorough and methodical"
model = "anthropic/claude-haiku-3-5-20241022"
tools.allow = ["read_file", "glob", "grep", "web_search", "web_fetch"]
tools.deny = ["exec", "write_file"]
system_prompt_suffix = "Gather facts and report clearly."

[agents.presets.coordinator]
identity.name = "orchestrator"
delegate_only = true
tools.allow = ["spawn_agent", "sessions_list", "sessions_history", "sessions_send", "task_list"]
sessions.can_send = true
```

Then call `spawn_agent` with a preset:

```json
{
  "task": "Find all auth-related code paths",
  "preset": "researcher"
}
```

## Config Fields

Top-level:

- `[agents] default_preset` (optional preset name)
- `[agents] presets` (map of named presets)

Per preset (`[agents.presets.<name>]`):

- `identity.name`, `identity.emoji`, `identity.theme`
- `model`
- `tools.allow`, `tools.deny`
- `system_prompt_suffix`
- `max_iterations`, `timeout_secs`
- `sessions.*` access policy
- `memory.scope`, `memory.max_lines`
- `delegate_only`

## Tool Policy Behavior

- If `tools.allow` is empty, all tools start as allowed.
- If `tools.allow` is non-empty, only those tools are allowed.
- `tools.deny` is applied after allow-list filtering.
- For normal sub-agents, `spawn_agent` is always removed to avoid recursive runaway spawning.
- For `delegate_only = true`, the registry is restricted to delegation/session tools:
  `spawn_agent`, `sessions_list`, `sessions_history`, `sessions_send`, `task_list`.

## Session Access Policy

`sessions` policy controls what a preset can see/send across sessions:

- `key_prefix`: optional session-key prefix filter
- `allowed_keys`: explicit allow-list
- `can_send`: allow/disallow `sessions_send`
- `cross_agent`: permit cross-agent session access

See [Session Tools](session-tools.md) for full details.

## Per-Agent Memory

Each preset can have persistent memory loaded from a `MEMORY.md` file at spawn
time. The memory content is injected into the sub-agent system prompt.

- `memory.scope` determines where the file is stored:
  - `user` (default): `~/.moltis/agent-memory/<preset>/MEMORY.md`
  - `project`: `.moltis/agent-memory/<preset>/MEMORY.md`
  - `local`: `.moltis/agent-memory-local/<preset>/MEMORY.md`
- `memory.max_lines` limits how much is injected (default: 200).

The directory is created automatically so agents can write to it.

```toml
[agents.presets.researcher.memory]
scope = "project"
max_lines = 100
```

## Model Selection Order

When `spawn_agent` runs, model choice is:

1. Explicit `model` parameter in tool call
2. Preset `model`
3. Parent/default provider model

## Markdown Agent Definitions

Presets can also be defined as markdown files with YAML frontmatter, discovered from:

- `~/.moltis/agents/*.md` (user-global)
- `.moltis/agents/*.md` (project-local)

Project-local files override user-global files with the same `name`.
TOML presets always take precedence over markdown definitions.

Example `~/.moltis/agents/reviewer.md`:

```markdown
---
name: reviewer
tools: Read, Grep, Glob
model: sonnet
emoji: 🔍
theme: focused and efficient
max_iterations: 20
timeout_secs: 60
---
You are a code reviewer. Focus on correctness and security.
```

Frontmatter fields: `name` (required), `tools`, `deny_tools`, `model`, `emoji`,
`theme`, `delegate_only`, `max_iterations`, `timeout_secs`.
The markdown body becomes `system_prompt_suffix`.
