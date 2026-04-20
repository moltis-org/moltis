# Plan: Self-Improving Agent Loop

**Status:** Phase A + B + C Complete
**Priority:** High
**Date:** 2026-04-20
**Scope:** Close the three gaps that made Hermes Agent go viral — self-improving skills, agentic memory lifecycle, and the "$5 deployment" narrative — while preserving Moltis's stronger security and architecture foundations.

## Context

Hermes Agent reached 100k GitHub stars in 53 days. The viral hook is not a single feature but a product loop: the agent gets smarter with every interaction. Three concrete mechanisms drive this:

1. **Self-improving skills** — after complex tasks, the agent autonomously creates reusable procedures
2. **Agentic memory lifecycle** — memory isn't just a tool; it's a background process that extracts, prefetches, and evolves
3. **"Runs anywhere for $5"** — SSH backend + serverless hibernation + one-command gateway setup

Moltis already has stronger primitives than Hermes in most areas. What's missing is the **autonomous behavior loop** on top of those primitives.

### Prior work

- `plans/2026-03-28-plan-hermes-gap-roadmap.md` — broader roadmap (phases 0–5)
- `plans/2026-04-10-plan-skills-native-read-tool.md` — `read_skill` tool (landed)
- `crates/tools/src/skill_tools.rs` — `create_skill`, `update_skill`, `delete_skill`, `read_skill`, `write_skill_files` all exist
- `crates/agents/src/silent_turn.rs` — pre-compaction memory flush already runs a hidden LLM turn
- `crates/memory/` — FTS5 + vector embeddings, hybrid search, chunking, writer trait

## Problem Statement

Moltis has the tools for an agent to create skills and write memories, but **the agent never does so autonomously**. The create/update/delete skill tools exist, but the agent loop doesn't prompt or encourage their use. Memory writes only happen during the pre-compaction silent turn. There is no post-turn extraction, no prefetch, no session-end summary.

Hermes solves this with three mechanisms Moltis lacks:

1. **System prompt guidance** telling the agent to create skills after complex tasks
2. **Memory lifecycle hooks** (pre-turn prefetch, post-turn sync, session-end summary)
3. **Skill patching** — incremental fixes to skills the agent notices are stale or broken

## Non-Goals

- Copying Hermes's Python architecture or meta-tool patterns
- Adding SSH/serverless backends (separate plan)
- Building a skill hub or registry (separate plan, phase 4 in the roadmap)
- Replacing Moltis's security model with convenience-first defaults
- Adding new channels (separate effort)
- Hermes-style RL or trajectory generation

---

## Part 1: Self-Improving Skills

### What Hermes does

In `agent/prompt_builder.py`, the system prompt includes:

> "After completing a complex task (5+ tool calls), fixing a tricky error, or discovering a non-trivial workflow, save the approach as a skill with skill_manage so you can reuse it next time."

The `skill_manager_tool.py` provides `create`, `edit`, `patch`, `delete`, and `write_file` actions. The `patch` action does surgical find/replace updates without rewriting the full skill. Created skills are scanned by `skills_guard.py` for dangerous patterns before activation.

### What Moltis already has

- `create_skill` tool — creates SKILL.md with frontmatter + body
- `update_skill` tool — overwrites SKILL.md entirely
- `delete_skill` tool — removes skill directory
- `read_skill` tool — reads SKILL.md body + lists sidecar files
- `write_skill_files` tool — writes sidecar files (references, templates, scripts, assets)
- `moltis_skills::safety::scan_skill_body()` — injection pattern scanner (warn-only)
- Checkpoint system — automatic pre-mutation snapshots

### What's missing

#### 1.1 System prompt guidance for autonomous skill creation

**File:** `crates/skills/src/prompt_gen.rs` (or `crates/agents/src/prompt.rs`)

Add a section to the system prompt, appended when skill tools are registered, that tells the agent:

```
## Skill Self-Improvement

You have tools to create, read, update, and delete personal skills. Use them proactively:

- After completing a complex task (5+ tool calls), consider saving the approach as a reusable skill with `create_skill`
- After fixing a tricky error or discovering a non-obvious workflow, save it so you don't have to rediscover it
- When a skill you're using has stale or incorrect instructions, update it with `update_skill`
- When you notice a skill could benefit from reference data, use `write_skill_files` to add sidecar files

Do NOT create skills for trivial or one-off tasks. Good skills encode multi-step procedures, domain-specific knowledge, or workflows that are likely to recur.
```

**Implementation:**
- Add a `generate_skill_self_improvement_prompt()` function in `prompt_gen.rs`
- Call it from the system prompt builder when `create_skill` is in the tool registry
- Gate behind a config flag: `[skills] enable_self_improvement = true` (default true)
- Add the config field to `crates/config/src/schema.rs` and update `build_schema_map()` in `validate.rs`

**Tests:**
- Unit test: prompt contains the guidance text when skill tools are enabled
- Unit test: prompt omits the guidance text when `enable_self_improvement = false`

#### 1.2 Skill patch tool (incremental updates)

**File:** `crates/tools/src/skill_tools.rs`

The current `update_skill` requires rewriting the entire SKILL.md body. Hermes has a `patch` action that does find/replace. This is important because:

- The agent can fix a single line without re-generating the whole skill
- Reduces hallucination risk (less content to generate)
- Faster, cheaper (fewer tokens)

Add a `PatchSkillTool`:

```rust
pub struct PatchSkillTool {
    data_dir: PathBuf,
    checkpoints: CheckpointManager,
}

// Tool name: "patch_skill"
// Parameters:
//   name: string (required)
//   patches: array of { find: string, replace: string } (required, 1-10 items)
//   description: string (optional — update the frontmatter description)
```

**Behavior:**
1. Read current SKILL.md
2. For each patch, do exact string replacement (fail if `find` not found)
3. Optionally update frontmatter description
4. Write back with checkpoint
5. Run injection scan on the result

**Tests:**
- Happy path: single patch applied
- Multiple patches in order
- Patch not found → clear error
- Traversal protection (same as other skill tools)
- Injection scan runs on patched output

#### 1.3 Post-task skill suggestion (agent loop hook)

**File:** `crates/agents/src/runner/helpers.rs` (in `finish_agent_run`)

After the agent loop completes, if:
- The run used 5+ tool calls
- The run was successful (no error)
- Skill tools are available

Then append a gentle nudge to the agent's final context:

```
[System note: This task used {N} tool calls. If the workflow you just completed is reusable, consider saving it as a skill with create_skill.]
```

This is **not** a forced action — it's a system-level hint that the agent can choose to act on or ignore. The agent still decides whether the task is worth saving.

**Implementation:**
- Add a `skill_creation_nudge` field to `AgentRunResult` (bool + tool call count)
- In the gateway's post-run handling (where the response is sent back), if the nudge is set and the conversation is interactive, inject the note as a system message in the next turn
- Gate behind `[skills] enable_self_improvement`

**Alternative (simpler):** Instead of a post-run hook, just include the guidance in the system prompt (1.1 above) and trust the model to act on it. Hermes does it this way. Start with the simpler approach; add the post-run nudge only if models don't reliably self-improve with prompt-only guidance.

**Decision:** Start with 1.1 (prompt-only). Revisit 1.3 if needed.

---

## Part 2: Agentic Memory Lifecycle

### What Hermes does

Hermes's `MemoryManager` has lifecycle hooks:
- `on_turn_start(query)` → prefetch relevant memories based on the user's message
- `sync_all(user_content, assistant_content)` → after each turn, extract facts worth remembering
- `on_session_end` → summarize the session's key learnings
- `on_pre_compress` → save important context before compaction discards it
- `queue_prefetch_all()` → background async prefetch for the next turn

### What Moltis already has

- `silent_turn.rs` — pre-compaction memory flush (runs an LLM turn to extract memories before compacting). This is equivalent to Hermes's `on_pre_compress`.
- `memory_search` tool — agent can explicitly search memory
- `memory_save` tool — agent can explicitly write memory
- `memory_delete` tool — agent can remove memory entries
- `MemoryWriter` trait — shared by save tool and silent turn
- `MemoryRuntime` trait — search + sync operations
- Hybrid search (FTS5 + vector) — better than Hermes's FTS5-only built-in

### What's missing

#### 2.1 Memory prefetch on turn start

**Files:** `crates/chat/src/service/chat_impl.rs`, `crates/memory/src/runtime.rs`

Before building the system prompt for each turn, query memory with the user's message as the search query. Inject relevant results into the system prompt as a `<recalled_context>` block.

```rust
// In the chat service, before calling the agent loop:
if memory_enabled && !user_message.is_empty() {
    let results = memory_runtime.search(&user_message_text, 3).await?;
    if !results.is_empty() {
        let recalled = format_recalled_context(&results);
        system_prompt.push_str(&recalled);
    }
}
```

**Format:**
```xml
<recalled_context>
The following was recalled from your long-term memory as potentially relevant:

- [memory/2026-04-15.md] User prefers streaming API calls over batch...
- [MEMORY.md] Project uses Rust workspace with 46+ crates...
</recalled_context>
```

**Implementation details:**
- Add `prefetch_memory(query: &str, limit: usize) -> Vec<SearchResult>` to `MemoryRuntime`
- Call it in the chat service before prompt assembly
- Gate behind `[memory] enable_prefetch = true` (default true)
- Add config field to schema + validate
- Use a low limit (3-5 results) to avoid prompt bloat
- Skip prefetch for very short messages (< 10 chars) or system commands

**Tests:**
- Integration test: memory prefetch injects recalled context into prompt
- Test: prefetch skipped when disabled
- Test: prefetch skipped for very short messages
- Test: empty memory returns no `<recalled_context>` block

#### 2.2 Post-turn memory sync

**Files:** `crates/chat/src/service/chat_impl.rs`, `crates/agents/src/silent_turn.rs`

After each turn completes, run a lightweight background extraction. This is different from the pre-compaction flush — it runs after **every** turn, not just before compaction.

**Design options:**

**Option A: Background silent turn (heavier, more thorough)**
- After each turn, spawn a background task that runs a mini silent turn
- The mini turn reviews only the latest user message + assistant response (not the full history)
- Writes to `memory/YYYY-MM-DD.md` if it finds anything worth remembering
- Pros: LLM-powered extraction is highest quality
- Cons: doubles LLM cost per turn, adds latency if not truly async

**Option B: Rule-based extraction (lighter, deterministic)**
- After each turn, run heuristic extraction:
  - If assistant mentioned a user preference → extract it
  - If the turn resolved a technical issue → log the solution
  - If the turn involved >N tool calls → log the workflow summary
- Pros: zero extra LLM cost, instant
- Cons: limited quality, hard to maintain rules

**Option C: Periodic background flush (compromise)**
- Every N turns (configurable, default 5), run a silent turn that reviews the last N turns
- Between flushes, do nothing
- Pros: amortizes LLM cost, still gets good extraction
- Cons: can miss things if session ends before next flush

**Recommendation:** Start with **Option C** (periodic flush every N turns). It's the best cost/quality tradeoff:

```toml
[memory]
# Run a background memory extraction every N turns (0 = disabled, only pre-compaction)
auto_extract_interval = 5
```

**Implementation:**
- Add turn counter to chat service state
- After each turn, increment counter
- When counter hits threshold, spawn background silent turn with last N messages
- Reset counter
- Also flush on session end (new lifecycle event)

**Tests:**
- Unit test: counter increments and triggers at threshold
- Unit test: counter resets after flush
- Integration test: memories actually written after N turns

#### 2.3 Session-end memory summary

**File:** `crates/chat/src/service/chat_impl.rs`

When a session ends (user closes chat, explicit `/end`, or timeout), run a final silent turn that:
1. Summarizes what was accomplished
2. Notes any unfinished work
3. Saves to `memory/YYYY-MM-DD.md`

This ensures no conversation's context is lost, even if compaction never triggers.

**Implementation:**
- Add a `on_session_end` hook in the chat service
- Reuse the `run_silent_memory_turn` from `silent_turn.rs` with a different system prompt focused on session summarization
- Gate behind `[memory] enable_session_summary = true` (default true)

**System prompt for session-end:**
```
You are a session summarizer. Review the conversation and write a concise summary to memory.

Focus on:
- What was accomplished (key outcomes and decisions)
- What was left unfinished (follow-up items)
- Any preferences or patterns the user demonstrated
- Technical context that would help resume this work

Write to `memory/YYYY-MM-DD.md` (append, don't overwrite).
Be concise — 5-15 bullet points maximum.
```

**Tests:**
- Integration test: session end triggers summary write
- Test: summary disabled when config is false
- Test: empty session (no messages) doesn't trigger summary

---

## Part 3: Deployment Story ("Runs Anywhere for $5")

This is more narrative than engineering, but a few concrete deliverables make the story credible.

### 3.1 Systemd service template

**File:** `deploy/moltis.service` (new)

```ini
[Unit]
Description=Moltis Agent Server
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=moltis
ExecStart=/usr/local/bin/moltis serve
Restart=on-failure
RestartSec=5
Environment=MOLTIS_DATA_DIR=/var/lib/moltis
Environment=MOLTIS_CONFIG_DIR=/etc/moltis

[Install]
WantedBy=multi-user.target
```

Also add install instructions to `docs/src/deployment.md`.

### 3.2 One-command gateway setup wizard

**File:** `crates/cli/src/setup_commands.rs` (new or extend existing)

Add `moltis setup` interactive CLI command that:
1. Asks which channels to enable (Telegram, Discord, Slack, etc.)
2. Walks through API key collection for each
3. Generates `moltis.toml` with the right channel config
4. Tests connectivity
5. Prints "Your agent is ready at..." message

This is the "$5 VPS" onboarding path. A user should be able to:
```bash
curl -sSL https://install.moltis.org | sh
moltis setup
moltis serve
```

### 3.3 Minimal Docker Compose for VPS deployment

**File:** `deploy/docker-compose.yml` (new)

```yaml
services:
  moltis:
    image: ghcr.io/moltis/moltis:latest
    restart: unless-stopped
    volumes:
      - ./data:/data
      - ./config:/config
    environment:
      - MOLTIS_DATA_DIR=/data
      - MOLTIS_CONFIG_DIR=/config
    ports:
      - "3000:3000"
```

With a companion `deploy/README.md` explaining the $5 VPS setup.

### 3.4 Documentation: "Deploy Moltis on a $5 VPS"

**File:** `docs/src/deploy-vps.md` (new)

Step-by-step guide:
1. Provision a $5 DigitalOcean/Hetzner/Linode droplet
2. Install Moltis (binary or Docker)
3. Run `moltis setup` to configure channels
4. Start as systemd service
5. Talk to your agent from Telegram/Discord/Slack

---

## Implementation Order

### Phase A: Self-Improving Skills (1-2 sessions)

1. **1.1** System prompt guidance for skill creation — `prompt_gen.rs`
2. **1.2** `PatchSkillTool` — `skill_tools.rs`
3. Config fields — `schema.rs`, `validate.rs`
4. Tests for all of the above

### Phase B: Agentic Memory (2-3 sessions)

5. **2.1** Memory prefetch on turn start — `chat_impl.rs`, `runtime.rs`
6. **2.2** Periodic background memory extraction — `chat_impl.rs`, `silent_turn.rs`
7. **2.3** Session-end memory summary — `chat_impl.rs`
8. Config fields — `schema.rs`, `validate.rs`
9. Tests for all of the above

### Phase C: Deployment Story (1 session)

10. **3.1** Systemd service template
11. **3.2** Docker Compose for VPS
12. **3.3** `moltis setup` wizard (can be a follow-up PR)
13. **3.4** VPS deployment docs

### Dependencies

```
1.1 ──────────────────────► (no deps, start here)
1.2 ──────────────────────► (no deps, can parallel with 1.1)
2.1 ──────────────────────► (no deps, can parallel with Phase A)
2.2 ── depends on 2.1 ───► (needs prefetch infrastructure)
2.3 ── depends on 2.2 ───► (reuses periodic flush machinery)
3.* ──────────────────────► (no deps, can parallel with everything)
```

## Config Changes Summary

All new fields in `crates/config/src/schema.rs`, with corresponding `build_schema_map()` updates in `validate.rs`:

```toml
[skills]
# Enable system prompt guidance for autonomous skill creation (default: true)
enable_self_improvement = true

[memory]
# Prefetch relevant memories at the start of each turn (default: true)
enable_prefetch = true
# Maximum memories to prefetch per turn (default: 3)
prefetch_limit = 3
# Run background memory extraction every N turns (0 = disabled) (default: 5)
auto_extract_interval = 5
# Write a session summary to memory when a session ends (default: true)
enable_session_summary = true
```

## Validation Checklist

Before opening PRs:

- [ ] `just format-check` passes
- [ ] `just lint` passes
- [ ] `just test` passes
- [ ] New config fields in `schema.rs` + `validate.rs`
- [ ] No secrets in code
- [ ] Conventional commits
- [ ] Config template updated (`crates/config/src/template.rs`)
- [ ] Docs updated where user-facing behavior changes

## Success Criteria

1. **A user can have a multi-session conversation where the agent noticeably improves** — it recalls prior work, reuses skills it created, and surfaces relevant context without being asked.
2. **An agent that completes a 10-step debugging workflow autonomously creates a skill for it** — without the user explicitly asking.
3. **Memory prefetch surfaces relevant context in >50% of turns** where prior conversation history exists.
4. **A new user can go from zero to running Moltis on a VPS with Telegram in under 15 minutes** using the setup wizard and docs.

## References

- Hermes Agent codebase: `~/code/hermes-agent/`
  - `agent/prompt_builder.py:545-559` — skill creation guidance in system prompt
  - `tools/skills_tool.py` — skill_manage tool with create/edit/patch
  - `agent/memory_manager.py` — memory lifecycle hooks
  - `tools/skills_tool.py:831` — injection pattern list
  - `hermes_state.py` — SQLite + FTS5 session storage
- Moltis existing code:
  - `crates/tools/src/skill_tools.rs` — current skill tools (create, update, delete, read, write_files)
  - `crates/agents/src/silent_turn.rs` — pre-compaction memory flush
  - `crates/agents/src/memory_writer.rs` — MemoryWriter trait
  - `crates/memory/src/runtime.rs` — MemoryRuntime trait
  - `crates/chat/src/service/chat_impl.rs:520` — where silent turn is called
  - `crates/skills/src/prompt_gen.rs` — skill prompt generation
  - `crates/config/src/schema.rs` — config schema
  - `crates/config/src/validate.rs` — config validation
- Prior plans:
  - `plans/2026-03-28-plan-hermes-gap-roadmap.md`
  - `plans/2026-04-10-plan-skills-native-read-tool.md`
