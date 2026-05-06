# Plan: Moltis Roadmap from Hermes Gap Analysis

**Status:** In Progress
**Priority:** High
**Date:** 2026-03-28
**Scope:** Turn the Hermes comparison into a concrete Moltis roadmap, focused on product positioning, persistent-agent workflows, and the few capability gaps that meaningfully affect user adoption.

## Implementation Update

Work landed on 2026-03-28:

- Phase 0: roadmap and gap analysis committed
- Phase 1: `sessions_search` shipped for cross-session recall
- Phase 2: automatic checkpoints shipped for built-in skill and memory mutations
- Phase 3: README and docs repositioned Moltis as a secure persistent personal agent server

Work still planned:

- Phase 4: remote execution and reach
- Phase 5: ecosystem, context-file hardening, optional identity modeling

## Background

Hermes Agent has strong public traction, 14.5k GitHub stars as of 2026-03-28, which is not an accident. The project is packaging a sharp, legible story:

- persistent server-first personal agent
- available on messaging surfaces where users already live
- remembers prior work
- creates and patches reusable skills
- feels like it gets better over time

Moltis already has a stronger foundation in several areas:

- stronger auth and secret handling
- better sandbox story
- better auditability
- stronger MCP operational model
- stronger browser automation and memory backend architecture

The gap is not mostly raw capability count. The gap is product shape, workflow defaults, and a few high-signal features that make Hermes feel like a living agent instead of a capable gateway.

## Problem Statement

Moltis currently reads as excellent infrastructure with a broad feature set.
Hermes reads as a product with a clear identity.

If Moltis wants to compete for attention and sustained usage, it should not copy Hermes wholesale. It should import the specific loops that make Hermes sticky:

1. cross-session recall
2. self-improving skill workflows
3. safer agentic editing
4. remote execution ergonomics
5. clearer persistent-agent positioning

## What Hermes Is Doing Right

### 1. Product framing is clearer

Hermes sells "an agent that grows with you", not just a runtime.
That message is easy to repeat and easy to understand.

### 2. Memory is productized, not just implemented

Hermes makes past-session recall and skill generation feel central to the agent's identity.
Moltis has strong memory internals, but the user-facing loop is less obvious.

### 3. Coding workflows feel agent-native

Three details matter:

- recall of prior sessions
- automatic safety checkpoints before edits
- subagent orchestration with visible progress

### 4. Remote-first execution broadens the story

Hermes can credibly say "run this on a cheap VPS, GPU cluster, or sleepy remote runtime".
Moltis currently feels much more local-first.

### 5. Ecosystem energy compounds

Portable skills and importable community workflows create community gravity.
Moltis should be careful here, but it should not ignore the effect.

## Planning Principles

1. Keep Moltis' core identity: secure, local-first, auditable, Rust-native.
2. Prefer features that improve the product loop, not just feature count.
3. Import Hermes ideas where they strengthen Moltis' existing architecture.
4. Avoid pulling research-side complexity into the core product.
5. Bias toward workflows users can immediately feel in day-to-day use.

## Non-Goals

These are explicitly lower priority:

- Hermes-style RL and trajectory generation stack
- copying Python meta-tools like `execute_code` into the core design
- replacing Moltis' security model with convenience-first defaults
- chasing every messaging surface before fixing the main product loop

## Roadmap Summary

## Phase 0: Product Narrative Reset

**Goal:** Make Moltis read like a coherent product.

### Deliverables

1. Rewrite homepage and README around a clearer thesis:
   "secure persistent personal agent server"
2. Reframe feature copy around durable loops:
   memory, recall, channels, automation, secure execution
3. Tighten docs landing pages around primary use cases:
   coding, personal assistant, remote operator, messaging agent
4. Add explicit "Why Moltis" positioning versus generic local agent tools

### Why this phase matters

Hermes is winning partly because people can explain it in one sentence.
Moltis needs a sentence ordinary humans can remember.

## Phase 1: Memory and Safety Loops

**Goal:** Close the highest-value workflow gaps.

### 1.1 First-class session recall

Add a `session_search` style tool over exported transcripts and session storage.

Expected behavior:

- keyword and semantic retrieval over past sessions
- focused summary of relevant prior work
- filtering by source, project, session type, date range
- default exclusion of the current session

Why:

- this is one of Hermes' most obvious user-visible wins
- Moltis already has the storage and memory primitives needed to support it

### 1.2 Automatic pre-edit checkpoints

Add transparent edit checkpoints before file-mutating operations.

Design target:

- shadow-git repository or equivalent rollback mechanism
- no pollution of the user's repo state
- one checkpoint per turn or per edit batch
- visible restore path in UI and CLI

Why:

- improves trust in autonomous editing
- pairs naturally with session branching and worktree isolation

### 1.3 Stronger self-improving skill loop

Keep current skill tools, but make the loop more opinionated:

- after complex workflows, suggest or auto-create a reusable skill
- when a skill fails or becomes stale, patch it immediately
- expose skill freshness and update history in the UI

Why:

- Moltis already has the primitives
- what is missing is the product behavior and visibility

## Phase 2: Better Agentic Coding UX

**Goal:** Make Moltis feel more native for long-running coding tasks.

### 2.1 Rich subagent orchestration UI

Improve the multi-agent experience with:

- live progress tree
- per-agent role and model indicators
- cost and iteration budgets
- clearer handoff and completion summaries

### 2.2 Context-file hardening and compatibility

Add:

- prompt-injection scanning for project context files
- optional compatibility with `.cursorrules` and `.cursor/rules`
- UI visibility into which files were loaded and why

Why:

- Hermes gets mileage from compatibility with existing editor ecosystems
- Moltis should be able to adopt the useful part without compromising safety

### 2.3 Better persistent specialist sessions

Build on current session tools and presets:

- named long-lived worker sessions
- easier coordinator patterns
- better session discovery and routing
- stronger per-specialist memory visibility

Why:

- Moltis already has strong primitives here
- packaging matters more than new core machinery

## Phase 3: Remote Execution and Reach

**Goal:** Expand beyond the local-machine story without weakening the core.

### 3.1 SSH worker backend

Ship an SSH execution backend first.

Requirements:

- explicit host configuration
- command routing policy
- secrets isolation
- per-host capability metadata
- clear audit trail

### 3.2 Remote worker and node model

After SSH, extend toward persistent remote workers:

- named nodes or workers
- host health and reachability
- optional queueing and routing
- project or session binding

### 3.3 Add Signal channel

Signal is the cleanest messaging-surface expansion after current channels.

Why:

- helps the "agent that lives where you do" story
- easier to explain than more niche channel work

### 3.4 Email channel, staged

Stage 1:

- outbound notifications and summaries

Stage 2:

- inbound threaded conversations

Why:

- strong assistant use case
- expands beyond chat apps without needing a whole new product story

## Phase 4: Ecosystem Layer

**Goal:** Capture some of the community gravity Hermes benefits from, without turning Moltis into a supply-chain liability.

### 4.1 Portable skill import/export

Support:

- export personal skills in a portable format
- import skills from trusted sources
- metadata for provenance, version, and review status

### 4.2 Curated skill registry

Ship a minimal curated registry with:

- signed or pinned sources
- local review before install
- clear trust indicators

### 4.3 Per-project trust controls

Decide and implement policy for:

- global trust
- per-project trust
- per-session trust

This should build on the existing skill marketplace hardening work, not bypass it.

## Phase 5: Optional Structured User Modeling

**Goal:** Explore whether Moltis should support richer user and agent identity models.

This phase is optional and should be isolated from the core.

Potential direction:

- plugin or sidecar integration
- user profile synthesis beyond plain memory files
- optional agent-self representation

Why this is late:

- it is interesting, but not necessary to make Moltis much more compelling
- it adds conceptual complexity quickly

## Recommended Execution Order

1. Phase 0 product narrative reset
2. Phase 1.1 session recall
3. Phase 1.2 automatic checkpoints
4. Phase 1.3 stronger self-improving skill loop
5. Phase 2.1 richer subagent orchestration UI
6. Phase 2.2 context-file hardening and compatibility
7. Phase 3.1 SSH worker backend
8. Phase 3.3 Signal
9. Phase 4.1 portable skill import/export
10. Phase 3.4 email channel
11. Phase 5 optional structured user modeling

## Proposed Milestones

### Milestone A: "Feels Smarter"

Target outcomes:

- session recall exists
- skill loop is more visible
- docs and homepage tell a clearer story

### Milestone B: "Feels Safer"

Target outcomes:

- automatic edit checkpoints
- clearer loaded-context visibility
- better context-file threat handling

### Milestone C: "Feels More Alive"

Target outcomes:

- better subagent UX
- Signal support
- richer persistent specialists

### Milestone D: "Runs Beyond the Laptop"

Target outcomes:

- SSH backend
- worker routing model
- early remote-node story

### Milestone E: "Builds Ecosystem Gravity"

Target outcomes:

- portable skills
- curated registry
- review and provenance UX

## Concrete Issues to Open

### Must-have

1. Session recall tool and transcript summarization
2. Edit checkpoint architecture and rollback UX
3. Product positioning rewrite for README, website, docs
4. Skill loop defaults and stale-skill patching
5. Context-file prompt-injection scanning
6. SSH execution backend

### Nice-to-have

1. Signal channel
2. Email channel
3. Rich subagent progress UI
4. Portable skill export/import
5. Curated registry and provenance model

### Explore later

1. Structured user-model sidecar
2. Remote sleepy runtimes like Modal or Daytona
3. Honcho-like memory identities

## Acceptance Criteria

This roadmap is successful if Moltis becomes easier to explain in one sentence and users can actually feel the difference in normal usage.

Concrete signs:

1. users can recover prior work without manually digging through sessions
2. autonomous file edits feel reversible and safer
3. skill creation and maintenance happen as part of normal use
4. Moltis can plausibly be described as a persistent agent, not only a gateway
5. remote execution story becomes credible without compromising local-first security

## Final Position

Moltis should not try to become Hermes.

Moltis should become:

- more legible as a persistent agent product
- better at recall and self-improvement workflows
- safer for autonomous editing
- more capable beyond the local machine

The strongest move is not "copy Hermes feature for feature".
The strongest move is to preserve Moltis' stronger core and import the few loops that make Hermes sticky.
