# Phase 4 Roadmap: Remote Execution and Reach

**Date:** 2026-03-28
**Status:** Proposed
**Goal:** Extend Moltis from a strong local-first gateway into a persistent
agent that can reliably operate across remote machines and more messaging
surfaces.

## Why This Phase Exists

Hermes feels "alive" partly because it is easy to imagine it running somewhere
else and reaching you anywhere. Moltis already has strong foundations here:

- node transport and node execution routing already exist
- channel architecture is capability-driven
- session metadata already tracks `node_id`
- `exec` can already route to connected nodes

The gap is product shape, host lifecycle, and staged channel expansion.

## Current Building Blocks

Relevant entry points:

- `crates/gateway/src/node_exec.rs`
- `crates/gateway/src/nodes.rs`
- `crates/tools/src/nodes.rs`
- `crates/gateway/src/server.rs`
- `docs/src/nodes.md`
- `docs/src/channels.md`

## Phase 4A: SSH Worker Backend

### Outcome

Allow a session or agent to execute commands on explicitly configured SSH
workers without requiring a full Moltis node process on the target host.

### Deliverables

1. Add an SSH execution provider behind the existing `NodeExecProvider` shape
   or a sibling trait if transport-specific metadata becomes awkward.
2. Model SSH workers as named resources with:
   - host
   - port
   - user
   - auth method reference
   - optional working directory
   - capability tags
3. Reuse existing session-level node selection where possible so the UI and
   tool story stays coherent.
4. Record an audit trail for SSH-routed commands comparable to node execution.

### Suggested File Touchpoints

- `crates/tools/src/exec.rs`
- `crates/gateway/src/node_exec.rs`
- `crates/gateway/src/nodes.rs`
- `crates/gateway/src/methods/node.rs`
- `crates/config/src/schema.rs`
- `crates/config/src/template.rs`

### Constraints

- no implicit host trust
- no environment-variable forwarding by default
- secrets must stay in existing credential storage patterns
- command routing must remain explicit and inspectable

## Phase 4B: Richer Remote Worker Model

### Outcome

Make remote execution feel first-class instead of a hidden transport detail.

### Deliverables

1. Add worker capability metadata:
   - languages
   - package managers
   - GPU availability
   - sandbox availability
2. Add health state:
   - connected
   - idle
   - busy
   - degraded
3. Add sticky session binding so a coding session can stay on the same worker.
4. Expose worker routing hints in runtime context and session metadata.

### Suggested File Touchpoints

- `crates/gateway/src/state.rs`
- `crates/gateway/src/services.rs`
- `crates/gateway/src/session.rs`
- `crates/chat/src/lib.rs`
- `crates/web/src/assets/js/page-agents.js`

## Phase 4C: Signal

### Outcome

Add Signal as the next messaging surface because it strengthens the "reach me
where I live" story more than another enterprise channel.

### Deliverables

1. Define Signal channel capabilities using the same matrix as current
   channels.
2. Start with outbound + inbound text, then streaming if the transport
   tolerates message edits cleanly.
3. Reuse OTP / allowlist controls where possible.

### Suggested File Touchpoints

- `crates/channels/`
- `crates/gateway/src/channel*.rs`
- `docs/src/channels.md`
- `docs/src/SUMMARY.md`

## Phase 4D: Email, Staged

### Outcome

Add a low-risk send-first channel that makes status updates and proactive
summaries more useful before attempting threaded inbound conversation.

### Stage 1

- send-only email delivery
- cron and proactive notification support
- delivery status in logs and UI

### Stage 2

- inbound threading
- sender mapping
- attachment policy
- reply attribution

## Sequencing

Recommended order:

1. SSH worker backend
2. Worker metadata and sticky binding
3. Signal
4. Email send-only
5. Email inbound

## Success Metric

Moltis should feel credible as a persistent agent that can:

- run on more than one machine
- pick the right machine for a task
- notify the user on the surfaces they already check
