# Skills: ship a native `read_skill` tool

## Beads tracking

**This plan is tracked in beads. Before starting, read the issues and claim the
one you're about to work on.** Run these commands from the repo root:

```bash
bd show moltis-1sk    # epic
bd show moltis-6p0    # 1. Add ReadSkillTool
bd show moltis-0vb    # 2. Register in gateway
bd show moltis-hjy    # 3. Rewrite generate_skills_prompt
bd show moltis-5xp    # 4. Prompt-injection scanner (safety)
bd show moltis-mcv    # 5. E2E test
bd show moltis-avv    # 6. (follow-up) Token-budget fallback
bd ready              # to see what's unblocked right now
```

The dependency chain is:

```
moltis-1sk (epic)
 ├── moltis-6p0  Add ReadSkillTool               ──┐
 │                                                 ├─► moltis-0vb  Register in gateway ──► moltis-mcv  E2E test
 │                                                 └─► moltis-hjy  Rewrite prompt
 ├── moltis-5xp  Prompt-injection scanner (parallel with 6p0)
 └── moltis-avv  Token-budget fallback (follow-up, optional for the bug fix)
```

Claim with `bd update <id> --status=in_progress` before writing code, close with
`bd close <id> --reason "..."` when done. Do **not** use TodoWrite or markdown
checklists for progress tracking — the beads issues are the source of truth.

If you discover new work while implementing, create a linked issue:

```bash
bd create --title="..." --description="..." -t task -p 2 --deps discovered-from:<parent-id>
```

## Problem

Moltis injects an `<available_skills>` XML block into the system prompt with
absolute `SKILL.md` paths and instructs the model to *"read its SKILL.md file
for full instructions"* (`crates/skills/src/prompt_gen.rs:4`). It then ships
**no built-in read tool**. Only `create_skill`, `update_skill`, `delete_skill`,
and `write_skill_files` are registered
(`crates/gateway/src/server.rs:3867-3882`).

When the user has no correctly-scoped filesystem MCP server, the model falls
into a fugue:

1. Tries a wired-up filesystem MCP (e.g. `mcp__filesystem__read_text_file`) and
   either hits a schema mismatch or a path-not-allowed error — that server is
   usually rooted somewhere other than `~/.moltis/skills/`.
2. Tries other MCP tools it knows about (`mcp__knylehub__read_note`, etc.) with
   fabricated relative paths.
3. Falls back to `memory_search` and **hallucinates** a plausible skill body
   from the prompt's description and its own training data.

This was reported by a user running `qwen/qwen3.5-35b-a3b`, attempting to use
an `inbox-contacts` skill. The model never read a single byte of the real
`SKILL.md` and invented the entire workflow.

## Design (what to build)

Copy the **hermes-agent** pattern, not openclaw's. Both solve this correctly,
but hermes is the better fit:

- Moltis already has the sidecar-file concept on the write side
  (`crates/tools/src/skill_tools.rs:352` `WriteSkillFilesTool`). Hermes's
  `skill_view(name, file_path)` is the exact read-side mirror of that.
- Name-based activation lets us drop the absolute-path leak from the system
  prompt entirely (right now every session prompt contains the user's home
  directory).
- A domain-specific tool is a much smaller security surface than a generic
  `read` tool, and matches Moltis's existing posture (containment checks,
  `#[must_use]`, `RequireAuth`, Origin validation).

Reference implementations:

- `~/code/hermes-agent/tools/skills_tool.py:751-900` — `skill_view` handler.
- `~/code/hermes-agent/tools/skills_tool.py:1249-1287` — tool schema + registry
  registration.
- `~/code/hermes-agent/agent/prompt_builder.py:545-559` — target prompt shape
  (no paths, just category/name/description, instruction points to
  `skill_view(name)`).

### Target prompt shape

Current (`prompt_gen.rs:11-34`):

```xml
## Available Skills

<available_skills>
<skill name="inbox-contacts" source="skill" path="/Users/penso/.moltis/skills/inbox-contacts/SKILL.md">
Email contact tracker - analyzes two-way email relationships...
</skill>
</available_skills>

To activate a skill, read its SKILL.md file (or the plugin's .md file at
the given path) for full instructions.
```

New:

```xml
## Available Skills

<available_skills>
<skill name="inbox-contacts" source="skill">
Email contact tracker - analyzes two-way email relationships...
</skill>
</available_skills>

To activate a skill, call the read_skill tool with its name. For sidecar
files (references/, templates/, scripts/), pass the file_path argument.
```

No absolute paths. No instruction to use a filesystem tool. The instruction
names the exact tool and argument shape.

### Target tool

```rust
// crates/tools/src/skill_tools.rs

pub struct ReadSkillTool {
    data_dir: PathBuf,
}

#[async_trait]
impl AgentTool for ReadSkillTool {
    fn name(&self) -> &str { "read_skill" }

    fn description(&self) -> &str {
        "Load a skill's full content or access its linked files \
         (references, templates, scripts). First call returns the SKILL.md \
         body plus a list of available sidecar files. To read those, call \
         again with the file_path parameter."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["name"],
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Skill name (use the names from <available_skills>)"
                },
                "file_path": {
                    "type": "string",
                    "description": "Optional: path to a sidecar file inside the skill directory (e.g. 'references/api.md')"
                }
            }
        })
    }

    async fn execute(&self, params: Value) -> anyhow::Result<Value> {
        // 1. Resolve name via discoverer (same registry the prompt was built from).
        // 2. If file_path is None:
        //      - call moltis_skills::registry::load_skill_from_path(skill_dir)
        //      - scan body for injection patterns (warn-only, do not block)
        //      - list sidecar files (skill_dir/references, /templates, /scripts)
        //      - return { frontmatter, body, linked_files, path_inside_skill_dir }
        // 3. If file_path is Some:
        //      - canonicalise and enforce containment with Component::Normal guard
        //        (same helper as WriteSkillFilesTool)
        //      - enforce MAX_SIDECAR_FILE_BYTES (reuse the constant)
        //      - return { content, bytes }
    }
}
```

The registry lookup has to use **the same discoverer the prompt was built
from**, so names the model sees always resolve. Pass in an `Arc<dyn
SkillDiscoverer>` (or a `DiscoveredSkills` snapshot) at construction time from
`crates/gateway/src/server.rs`, alongside the existing data_dir.

### Safety guards (copy from hermes + reuse existing Moltis helpers)

- **Containment**: resolve the final path, then `path.strip_prefix(skill_dir)`.
  Must be under the resolved skill dir. The existing `Component::Normal`-only
  walk used by `WriteSkillFilesTool` is the right primitive — extract it into a
  small shared helper and call it from both.
- **Size cap**: reuse `MAX_SIDECAR_FILE_BYTES` (128 KB) from
  `crates/tools/src/skill_tools.rs:18`.
- **Injection scan** (moltis-5xp): grep the body for the hermes pattern list
  (`:831`), emit `tracing::warn!` with skill name + first pattern hit. Do
  *not* block — hermes is warn-only here and the user's own skills can
  legitimately contain some of these strings.
- **Platform gate** (deferred — Moltis doesn't currently have a platform
  field on skills; skip for this bug fix).

## Implementation steps

Do the work in this order. Each step corresponds to a beads issue.

### 1. `moltis-6p0` — Add `ReadSkillTool`

File: `crates/tools/src/skill_tools.rs`

- Add the struct and `AgentTool` impl as sketched above.
- Extract the existing `Component::Normal`-only path guard from the write tool
  into a module-private helper (`fn join_inside_skill_dir(skill_dir: &Path, rel:
  &str) -> crate::Result<PathBuf>`) and call it from both the read and write
  tools — don't duplicate.
- Use `moltis_skills::registry::load_skill_from_path` for the body read (it
  already exists at `crates/skills/src/registry.rs:109` and parses frontmatter
  + body).
- Enumerate sidecar files by walking `skill_dir/references`,
  `skill_dir/templates`, `skill_dir/scripts` one level deep, returning a
  `Vec<{ relative_path, bytes }>`. Cap at `MAX_SIDECAR_FILES_PER_CALL = 16`
  (already defined).
- Unit tests (all in the same file, match the existing test pattern around
  line 734):
  - happy path: create a skill with a known body, read it, assert body.
  - unknown name: returns a friendly error mentioning `skills_list`-style hint.
  - sidecar happy path: seed `references/api.md`, read via `file_path`.
  - traversal: `file_path="../../etc/passwd"` → rejected.
  - traversal via symlink inside `references/`: rejected.
  - oversized sidecar (>128 KB): rejected with clear error.
  - primary call lists sidecar files in the response.

### 2. `moltis-0vb` — Register in the gateway (blocked by 6p0)

File: `crates/gateway/src/server.rs:3867-3882`

- Register alongside the existing `CreateSkillTool::new(data_dir.clone())`
  block.
- Thread the same skills discoverer the prompt build uses so names resolve
  identically. If the discoverer is rebuilt per-request, the tool needs a
  handle that reflects the current state.
- Add an integration-level assertion somewhere in the existing services test
  harness that `read_skill` is present in the runtime tool list when skills
  are enabled.

### 3. `moltis-hjy` — Rewrite `generate_skills_prompt` (blocked by 6p0)

File: `crates/skills/src/prompt_gen.rs`

- Drop the `path=` attribute from the `<skill>` element entirely.
- Keep `name`, `source`, and the description.
- Rewrite the trailing instruction block to the new text (see "Target prompt
  shape" above).
- Update:
  - existing unit tests in `prompt_gen.rs` (lines 44–85) — remove assertions
    that check for `SKILL.md` in the prompt, add assertions that no absolute
    path appears.
  - `crates/agents/src/prompt.rs` test around line 1082 that checks
    `<available_skills>` is emitted — still passes but should gain an
    assertion that `read_skill` is mentioned.
  - `docs/src/system-prompt.md:152-163` — show the new shape.

### 4. `moltis-5xp` — Prompt-injection scanner

File: new `crates/skills/src/safety.rs` (or extend `parse.rs` if it's small)

```rust
const INJECTION_PATTERNS: &[&str] = &[
    "ignore previous instructions",
    "ignore all previous",
    "you are now",
    "disregard your",
    "forget your instructions",
    "new instructions:",
    "system prompt:",
    "<system>",
    "]]>",
];

pub fn scan_skill_body(skill_name: &str, body: &str) -> Vec<&'static str> { ... }
```

Case-insensitive match. `ReadSkillTool::execute` calls this and logs
`tracing::warn!(skill = %name, patterns = ?hits, "skill body contains
potential prompt-injection patterns")` if the return is non-empty. Warn-only.

Tests: each pattern triggers, a clean body doesn't.

### 5. `moltis-mcv` — E2E test (blocked by 0vb)

New test file under `crates/gateway/tests/` (mirror the style of existing
gateway integration tests).

- Seed `<tempdir>/skills/test-skill/SKILL.md` with known frontmatter + body.
- Seed `<tempdir>/skills/test-skill/references/notes.md` for the sidecar test.
- Spin up the gateway with `data_dir = tempdir`.
- Resolve the `read_skill` tool from the tool registry.
- Positive case: `{ name: "test-skill" }` → body matches, `linked_files`
  contains `references/notes.md`.
- Positive sidecar: `{ name: "test-skill", file_path: "references/notes.md" }`
  → content matches.
- Negative case: `{ name: "does-not-exist" }` → returns an error (asserted on
  the error shape, not on a panic).
- Traversal: `{ name: "test-skill", file_path: "../../etc/passwd" }` →
  rejected.

This is the test that would have caught the bug in the original report.

### 6. `moltis-avv` — Token-budget fallback (follow-up, optional)

Not required for the bug fix. Ship separately once steps 1–5 land.

- Port `openclaw/src/agents/skills/workspace.ts:567` `applySkillsPromptLimits`:
  full → compact → binary-search prefix.
- Add `[skills] max_skills_in_prompt` (default 64) and
  `[skills] max_skills_prompt_chars` (default 8000) to
  `crates/config/src/schema.rs`. Update `build_schema_map()` in `validate.rs`
  per the CLAUDE.md rules.
- Compact mode: `<skill name="..." source="...">` with no description, for
  when the full form doesn't fit.

## What **not** to do in this PR

- **No generic `read_file` tool.** We could copy openclaw's approach
  (`pi-coding-agent/dist/core/tools/read.js`) and have skills resolve via the
  filesystem like any other file, but that's a much larger security surface
  and a separate decision. This PR is scoped to skills.
- **No platform gate.** Moltis's skill frontmatter doesn't currently have a
  platform field. Adding one is a separate design question — punt.
- **No changes to the write-side tools.** `CreateSkillTool`, `UpdateSkillTool`,
  `DeleteSkillTool`, `WriteSkillFilesTool` stay exactly as they are, except
  for the extracted containment helper which they share with the new read
  tool.
- **No changes to how plugins expose skills.** `SkillSource::Plugin` still
  points at a `.md` file via `path` in the metadata; the new tool handles
  plugin skills the same as personal/project skills by resolving through the
  registry.
- **No CHANGELOG.md entry.** Per CLAUDE.md, changelogs are auto-generated
  from conventional commits — just use `feat(skills): ...` and `fix(skills):
  ...` commits.

## Validation checklist

Before opening the PR:

- [ ] `just format-check` passes (pinned nightly rustfmt).
- [ ] `just lint` passes.
- [ ] `just test` passes — includes the new unit tests in `skill_tools.rs`,
      `prompt_gen.rs`, `safety.rs`, and the new gateway integration test.
- [ ] Manual check: start Moltis with a seeded `~/.moltis/skills/test-skill/`
      directory, open the web UI, verify the skill appears in `<available_skills>`
      in the system prompt **without** an absolute path, verify the model can
      read it by calling `read_skill` in a live session.
- [ ] Grep the codebase for the old instruction text (`"read its SKILL.md
      file"`) to make sure no stale copy lingers.
- [ ] `./scripts/local-validate.sh <PR_NUMBER>` once the PR exists.

## PR description template

```md
## Summary
- Add a native `read_skill` agent tool (name-based, with optional sidecar
  `file_path`) so the model can actually load skill bodies without an
  external filesystem MCP server.
- Drop the absolute `SKILL.md` path from the `<available_skills>` prompt
  block; point the instruction at `read_skill(name)`.
- Warn on prompt-injection patterns inside skill bodies.

## Validation
### Completed
- [ ] `just format-check`
- [ ] `just lint`
- [ ] `just test`
- [ ] New unit tests in `crates/tools/src/skill_tools.rs`
- [ ] New gateway integration test (`crates/gateway/tests/...`)

### Remaining
- [ ] `./scripts/local-validate.sh <PR>`

## Manual QA
1. Seed `~/.moltis/skills/demo/SKILL.md` with a known body.
2. Start Moltis, open the web UI, start a chat.
3. Verify the system prompt contains `<skill name="demo" source="skill">` with
   **no** `path=` attribute.
4. Ask the model to use the `demo` skill.
5. Verify the tool log shows a `read_skill` call (not `mcp__filesystem__*` or
   `memory_search`).
6. Verify the assistant response reflects the actual seeded body.
```

## References

- Current Moltis state:
  - `crates/skills/src/prompt_gen.rs:4` — prompt builder
  - `crates/skills/src/registry.rs:68,109` — existing `load_skill` helpers
  - `crates/tools/src/skill_tools.rs:17-18,352` — sidecar constants and write tool
  - `crates/gateway/src/server.rs:3867-3882` — tool registration site
  - `crates/agents/src/prompt.rs:698` — where the prompt block is appended
  - `docs/src/system-prompt.md:152-163` — docs to update
- Reference implementations:
  - `~/code/hermes-agent/tools/skills_tool.py:751-1287` — `skill_view` tool
  - `~/code/hermes-agent/agent/prompt_builder.py:500-568` — prompt shape
  - `~/code/openclaw/src/agents/skills/workspace.ts:544-600` — token-budget fallback (for moltis-avv)
  - `~/code/openclaw/node_modules/@mariozechner/pi-coding-agent/dist/core/tools/read.js` — the alternative (generic `read`) approach we chose not to take

## Session completion

Per CLAUDE.md's "Landing the Plane" section, work is not complete until
`git push` succeeds. At the end of each work session on this plan:

1. `bd close <id> --reason "..."` for each finished issue.
2. `git add -p && git commit -m "feat(skills): ..."` (conventional commit, no
   `Co-Authored-By` trailer).
3. `git pull --rebase && bd dolt commit && git push && git status`.
4. If any follow-up work was discovered, create linked issues with
   `--deps discovered-from:<parent-id>` before closing the parent.
