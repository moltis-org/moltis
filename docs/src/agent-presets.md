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
- `sandbox.*` — per-agent sandbox overrides (`sandbox.mode`, `sandbox.force`, `sandbox.mounts`, `sandbox.run_as`)
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

`mode` on its own is a default: an explicit per-session setting, including the
sandbox toggle in the web UI, still wins over it.

## Forcing the Sandbox On

`sandbox.force` is the one thing that says "this agent may never run outside a
sandbox".

```toml
[agents.presets.walter.sandbox]
force = true                 # This agent always runs sandboxed
```

With `force = true` the session toggle cannot turn the sandbox off: the gateway
refuses a `sessions.patch` that tries, and the button in the web UI is shown
disabled for those sessions with the reason in its tooltip.

`mode = "off"` next to `force = true` is a config error, reported by
`moltis config check` with the agent's name. Drop one or the other rather than
leaving the contradiction for the runtime to resolve.

**`mounts` and `run_as` do not force the sandbox.** They sit in the `[sandbox]`
block because they configure the sandbox, and configuration applies when the
sandbox is active. An agent that declares mounts or a `run_as` without
`force = true` gets them on every turn that runs in a container, and runs
without them - on the host, with whatever the gateway process itself holds - on
a turn where the sandbox is off. That is the documented behaviour, not a bug:
add `force = true` when those values are a requirement rather than a
preference.

## Per-Agent Sandbox Mounts

Bind extra host paths into one agent's sandbox container. Each mount is an
array-of-tables entry with an absolute `source` on the host, an absolute
`target` inside the container, and an `access` mode.

```toml
[[agents.presets.walter.sandbox.mounts]]
source = "/srv/vault"        # Absolute host path
target = "/srv/vault"        # Absolute path inside the sandbox
access = "rw"                # "ro" (default) or "rw"
```

This is deliberately **per-agent only**. There is no global mount list,
because a global one cannot express "this agent and no other": the runtime
sandbox config is shared by every agent in the install.

Rules, enforced on the config file, on the RPC write path, and again in the
backend before the container starts:

- `source` and `target` must both be absolute paths. A relative `source` is
  rejected, because Docker turns one into a **named volume** rather than a
  bind mount: the path looks empty inside the container and every command
  still reports success.
- Neither may contain `..`, and neither may resolve to `/`. The comparison is
  made after normalization, so `//` and `/.` are refused too - the container
  runtime resolves all three to the same directory.
- Two mounts may not share a `target`, again compared after normalization.
- Neither may contain a colon or a comma. The wire shape is
  `source:target:access` and the markdown sidecar comma separates the list, so
  a path containing either separator cannot be expressed - it is refused where
  it is written rather than mangled on the way back in.
- `access` must be `"ro"` or `"rw"`. An unrecognised value is an error, not a
  silent fallback.
- The `source` may not be one of the host paths below, live under one, or
  contain one. `/srv` is refused as surely as `/run/docker.sock` when the
  socket lives under it.

Every `rw` mount produces a warning from `moltis config check` naming the
agent and the host path, because a writable host path handed to a model is a
privilege grant.

### Denied mount sources

A source on this list is refused wherever a mount is written - the config
file, the markdown sidecar, and the RPC write path:

| Source | Why |
|--------|-----|
| `/proc`, `/sys` | The host process table and kernel interface. |
| `/dev` | The host device tree. Individual device nodes are excepted: `/dev/dri`, `/dev/snd`, `/dev/kvm` and `/dev/net/tun` stay mountable, so a GPU, a sound card, KVM or a tun device can still be passed through. Everything else under `/dev` - `/dev/mem`, raw block devices - stays denied. |
| Any path whose file name is `docker.sock` or `podman.sock` | A container that can reach a runtime socket can start a sibling container with any mount it likes, which is root on the host by a longer route. For podman it also bypasses `allow_nested_podman` entirely. |
| `/var/run/docker.sock`, `/run/docker.sock`, `/run/podman/podman.sock`, `/var/run/podman/podman.sock` | The conventional socket paths, so a source *under* one is refused too. |
| `$XDG_RUNTIME_DIR/podman/podman.sock` | Where a rootless podman puts its socket. |
| The moltis data directory | It holds the credential store, and the credential store holds every API key and provider secret moltis knows. |
| `~/.ssh` | Host keys. |

The error names the entry that matched and the path it matched on.

#### What the denylist can and cannot see

Two limits, both deliberate, both worth knowing before treating this list as a
boundary. It is **defence in depth** behind the read-only sandbox API key, not
the boundary itself.

**Host paths versus container paths.** A mount `source` is a host path: the
container runtime resolves it on the host. The fixed entries above are the
same path on both sides, so they hold either way. The two derived entries are
not:

- The **data directory** is compared against the host spelling whenever moltis
  knows one - `[tools.exec.sandbox] host_data_dir` when it is set, and
  otherwise the path the runtime-mount detection works out at the first
  container start. When moltis runs directly on the host the two are the same
  path and nothing is needed. When moltis runs *in* a container and neither is
  available, only the container-side path is on the list and the host-side one
  is **not enforced**; setting `host_data_dir` is the supported way to close
  that.
- **`~/.ssh`** is derived from the moltis process's own `$HOME`. Inside a
  container that is the container's home, and the host user's home is not
  knowable from in there. So this entry protects a moltis running directly on
  the host and is **inert in a containerized deployment**. The same applies to
  `$XDG_RUNTIME_DIR`; the fixed `/run/podman/...` entries and the socket
  file-name rule do not depend on it.

**Symlinks.** The match is lexical - the same resolution the container runtime
does to the string - and a lexical match cannot see through a link:
`/srv/link -> /run/docker.sock` is not `/run/docker.sock` as a string. So the
source is also canonicalized and re-checked, but only where the moltis process
can see the path. On a host install it can, and the link is caught. In a
container it usually cannot: the host path is not in its mount namespace, the
resolution fails, and the check stays purely lexical.

Mounts only reach a sandbox backend that supports bind mounts. A backend that
does not errors the turn rather than dropping the mount silently.

## Per-Agent Sandbox User

Run one agent's sandbox container as a real `uid:gid` instead of root.

```toml
[agents.presets.walter.sandbox]
run_as = "1000:1000"         # uid:gid; uid 0 and gid 0 are refused
```

Without it the container runs as whatever user its image declares, which for
the sandbox images is root — so every file the agent writes into a bind mount
lands on the host owned by root, and the operator needs `sudo` to read, move
or delete it. Setting `run_as` to the owner of the mounted paths makes those
files land owned by that user instead.

The value is exactly two non-negative integers separated by one colon, with
neither part empty. It fails closed at every layer that can see it — the RPC
write path, the config file, the markdown sidecar, and the backend's own
re-check just before it emits `--user` — and there is **no fallback to root
anywhere**:

- A malformed value is an error, never a dropped field.
- A uid of `0` is refused. A `run_as` that silently meant root would be worse
  than not setting it at all.
- A gid of `0` is refused for the same reason: the root group is group-write
  on root-owned paths, so it hands back most of what refusing uid 0 took away.
- A backend that cannot run a container as a configured user errors the turn
  rather than starting one as root. Docker and Podman both support it; the
  Apple Container backend does not, because whether its CLI accepts `--user`
  could not be verified.

A `run_as` session never shares a HOME with a root session. With
`home_persistence` set to `shared` or `session` the sandbox home becomes a
per-uid directory — under the operator's `shared_home_dir` when one is set, and
`<data>/sandbox/home/user/<uid>` otherwise — so a root session cannot seed
root-owned files in it. With `home_persistence = "off"` the container gets a
writable tmpfs at `/home/sandbox` owned by that uid instead.

Changing `run_as` recreates the container on the next turn, the same way a
changed mount list does.

### Known gaps

**`run_as` is restricted to the gateway's own uid.** The per-uid sandbox home is
created by the gateway process, so it lands owned by the gateway's uid with mode
`0755`. The gateway then refuses to start the container unless the `run_as` uid
could write that directory, and POSIX resolves that to the "other" bits for any
uid but the owner - which are read and execute only. So any uid other than the
gateway's own hard-errors at container start with "sandbox home directory ... is
not writable by run_as", and the only way around it is for an operator to
pre-create that directory and grant the uid write access.

**Root-homed tooling in the image.** The generated sandbox image installs mise
before `ENV HOME`, so it lands in `/root/.local/bin`: on `PATH`, but unreadable
to a non-root uid. Anything else root-homed in the image has the same problem.
Packages installed through `[tools.exec.sandbox] packages` are unaffected.

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
`sandbox_force`, `sandbox_mounts`, `run_as`, `skills_allow`, and `skills_deny`.

`sandbox_mounts` is the flat spelling of the mount list: a comma-separated
string of `source:target:access` triples, which round trips losslessly to and
from the structured TOML form. It round trips because the rules refuse a colon
or a comma in a mount path, not because either separator can be escaped. A
sidecar that does not parse leaves its agent present but forced into the
sandbox, rather than dropping the agent and letting it fall back to the global
`[tools.exec.sandbox] mode`.

```markdown
---
name: walter
sandbox_mode: all
sandbox_force: true
sandbox_mounts: "/srv/vault:/srv/vault:rw, /srv/notes:/srv/notes:ro"
run_as: "1000:1000"
---
```
The markdown body becomes `system_prompt_suffix`.
