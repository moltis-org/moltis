# Agent Presets

Agent presets let `spawn_agent` run sub-agents with role-specific configuration.
Use them to control model cost, tool access, session visibility, and behavior.

They are different from [modes](modes.md): modes are temporary overlays for the
current chat session, while agent presets configure delegated sub-agents.

## Built-In Presets

Moltis ships with these presets on every install:

| Preset | Role |
|--------|------|
| `research` | Evidence gathering and synthesis. This is the default when `spawn_agent.preset` is omitted. |
| `coder` | Scoped implementation, debugging, cleanup, and focused verification. |
| `reviewer` | Code review for correctness, regressions, security, and missing tests. |
| `qa` | End-to-end behavior validation, repro steps, and pass/fail reporting. |
| `ux` | UX, accessibility, interaction, and visual quality review. |
| `docs` | User-facing documentation, examples, and config reference updates. |
| `coordinator` | Delegation-first planning and result integration. |

User TOML presets and markdown agent definitions with the same name override
the built-in preset. The built-ins do not set a model or tool allow/deny
policy, so they inherit the session's provider and normal tool access. The
`coordinator` preset sets `delegate_only = true`, restricting it to delegation,
session, and task-list tools.

## Quick Start

```toml
[agents.presets.researcher]
identity.name = "scout"
identity.emoji = "🔍"
identity.theme = "thorough and methodical"
model = "anthropic/claude-haiku-3-5-20241022"
tools.allow = ["Read", "Glob", "Grep", "web_search", "web_fetch"]
tools.deny = ["exec", "Write"]
system_prompt_suffix = "Gather facts and report clearly."

[agents.presets.coordinator]
identity.name = "orchestrator"
delegate_only = true
tools.allow = ["spawn_agent", "sessions_list", "sessions_history", "sessions_search", "sessions_send", "task_list"]
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
- `mcp` — MCP server access: `allow_servers` or `deny_servers`
- `sandbox.*` — per-agent sandbox overrides
- `skills.allow`, `skills.deny`
- `system_prompt_suffix`
- `max_iterations`, `timeout_secs` (override `[tools]` runtime limits for matching direct sessions and spawned sub-agents)
- `sessions.*` access policy
- `memory.scope`, `memory.max_lines`
- `delegate_only`

## Tool Policy Behavior

- If `tools.allow` is empty, all tools start as allowed.
- If `tools.allow` is non-empty, only those tools are allowed.
- `tools.deny` is applied after allow-list filtering.
- For normal sub-agents, `spawn_agent` is always removed to avoid recursive runaway spawning.
- For `delegate_only = true`, the registry is restricted to delegation/session tools:
  `spawn_agent`, `sessions_list`, `sessions_history`, `sessions_search`, `sessions_send`,
  `task_list`.

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

## MCP Server Access Control

Each preset can restrict which MCP servers are visible. Use `allow_servers` for
a positive allow-list, or `deny_servers` for a deny-list. The two are mutually
exclusive — set one or the other, not both.

```toml
# Only allow specific MCP servers:
[agents.presets.restricted.mcp]
allow_servers = ["github", "memory"]

# Block specific MCP servers:
[agents.presets.open.mcp]
deny_servers = ["home-assistant"]
```

When `allow_servers` is set, every configured MCP server not in the list is
denied. An empty `allow_servers = []` blocks all MCP tools.

## Per-Agent Sandbox Mode

Override the global sandbox mode per agent.

```toml
[agents.presets.kids.sandbox]
mode = "all"                 # Always sandbox this agent
```

Available values: `"off"`, `"all"`, `"non-main"`. The override is applied
as a per-session setting on the sandbox router.

## Per-Agent Skill Policy

Filter which skills are visible to an agent by name or category.

```toml
# Only allow specific skills:
[agents.presets.focused.skills]
allow = ["web_search", "research"]

# Block specific categories:
[agents.presets.safe.skills]
deny = ["gaming", "social-media"]
```

When `allow` is non-empty, only matching skills (by name or category) are
visible. `deny` is then applied on top.

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

The web UI uses the user-global markdown location for sub-agent preset edits:

- Open **Settings → Agents → Sub-Agents**.
- Choose **New Sub-Agent** to create `~/.moltis/agents/<id>.md`.
- Choose **Edit** on a built-in preset to create a user-global markdown override.
- Choose **Delete** on a custom/overridden preset to remove that markdown file.

This keeps `moltis.toml` small while still leaving every web-created sub-agent
editable on disk. If a preset with the same name exists in `moltis.toml`, the
TOML preset wins over the markdown file.

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
`theme`, `delegate_only`, `max_iterations`, `timeout_secs`, `display_name`,
`reasoning_effort`, `mcp_allow_servers`, `mcp_deny_servers`, `sandbox_mode`,
`skills_allow`, and `skills_deny`.
The markdown body becomes `system_prompt_suffix`.
