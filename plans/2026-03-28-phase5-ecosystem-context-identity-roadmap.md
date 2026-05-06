# Phase 5 Roadmap: Ecosystem, Context Hardening, and Identity

**Date:** 2026-03-28
**Status:** Proposed
**Goal:** Make Moltis feel cumulative and trustworthy over time by improving
skill portability, hardening project context ingestion, and exploring optional
structured identity layers without bloating the core.

## Why This Phase Exists

Hermes gets compounding value from three loops:

- skills are reusable and shareable
- project context feels editor-native
- the agent appears to "remember who you are"

Moltis already has pieces of each:

- runtime skill creation and sidecar files
- hierarchical context loading from `CLAUDE.md`, `.claude/rules`, and `AGENTS.md`
- identity, soul, user, and workspace prompt layers

The missing work is packaging, hardening, and compatibility.

## Current Building Blocks

Relevant entry points:

- `crates/tools/src/skill_tools.rs`
- `crates/skills/src/`
- `crates/projects/src/context.rs`
- `docs/src/system-prompt.md`
- `docs/src/skill-tools.md`
- `crates/gateway/src/agent_persona.rs`

## Phase 5A: Portable Skill Ecosystem

### Outcome

Turn skills from a local-only mutation surface into a portable ecosystem with
clear trust boundaries.

### Deliverables

1. Import/export for personal skills, including sidecar files.
2. Skill provenance metadata:
   - source
   - author
   - imported_at
   - checksum
3. Quarantine mode for third-party skills before activation.
4. UI surfacing for stale or frequently patched skills.

### Suggested File Touchpoints

- `crates/tools/src/skill_tools.rs`
- `crates/gateway/src/services.rs`
- `crates/skills/src/discover.rs`
- `crates/skills/src/watcher.rs`
- `docs/src/skill-tools.md`
- `docs/src/skills-security.md`

## Phase 5B: Context-File Hardening and Compatibility

### Outcome

Load more of the files users already have, but do it safely and transparently.

### Deliverables

1. Add compatibility for:
   - `.cursorrules`
   - `.cursor/rules/*`
2. Add a context-ingestion report:
   - loaded files
   - skipped files
   - size truncation
   - risk flags
3. Add prompt-injection scanning before context files are injected.
4. Add explicit UI visibility into the final context bundle per session.

### Suggested File Touchpoints

- `crates/projects/src/context.rs`
- `crates/agents/src/prompt.rs`
- `crates/chat/src/lib.rs`
- `docs/src/system-prompt.md`
- `docs/src/agent-presets.md`

### Guardrails

- no silent compatibility mode that loads new files without visibility
- risk signals must reach the UI and logs
- project-local overrides should stay deterministic

## Phase 5C: Optional Structured Identity Layer

### Outcome

Explore a richer user/agent model without making it mandatory for normal use.

### Deliverables

1. Keep current prompt identity files as the default path.
2. Add an optional structured layer for:
   - user preferences
   - long-term agent commitments
   - recurring project roles
3. Ensure the structured layer feeds prompts and memory, but can be fully
   disabled.
4. Do not mix this into the core memory path until the simpler checkpoint and
   recall loops have proven sticky.

### Suggested File Touchpoints

- `crates/gateway/src/agent_persona.rs`
- `crates/chat/src/lib.rs`
- `crates/memory/src/`
- `docs/src/system-prompt.md`

## Sequencing

Recommended order:

1. Skill import/export and provenance
2. Context-file compatibility with explicit visibility
3. Injection scanning and risk reporting
4. Optional structured identity experiments

## Success Metric

Moltis should feel like it:

- learns reusable workflows
- understands the same project context the user already maintains elsewhere
- stays explainable instead of turning into an opaque preference blob
