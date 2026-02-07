# Skill Marketplace Safety Hardening Plan

**Status:** Proposed
**Priority:** Critical
**Scope:** Skills, plugins, tool execution, install flow

## Background

Recent marketplace malware campaigns show a repeatable chain:

1. Publish a plausible skill/plugin.
2. Tell users to install a required dependency.
3. Route through staging pages to execute obfuscated shell.
4. Pull second-stage payloads and disable platform protections.
5. Exfiltrate credentials, keys, sessions, and workspace data.

The key lesson is that warning banners are not enough. Safety must be enforced
by runtime controls and fail-closed defaults.

## Goals

- Prevent untrusted skills/plugins from reaching arbitrary code execution.
- Enforce least privilege at runtime, not only in metadata/UI.
- Add durable provenance and trust decisions for installed content.
- Provide incident response controls and auditable security events.

## Non-goals

- Fully automated malware verdicting with zero false positives.
- Replacing user approvals for every benign command.

## Threat Model

- Malicious SKILL.md instructions and dependency install commands.
- Malicious repo updates after initial install.
- Obfuscated command strings and encoded payload staging.
- Data exfiltration via shell output, network requests, and secrets in env.

## Phase Plan

### P0 - Immediate Risk Reduction

1. **Wire approval config into runtime**
   - Use `tools.exec.approval_mode`, `tools.exec.security_level`, and
     `tools.exec.allowlist` to construct `ApprovalManager`.
   - Remove hardcoded default-only behavior at gateway startup.

2. **Gate dependency installation as high-risk execution**
   - Route `skills.install_dep` through approval checks.
   - Fail closed when sandbox is unavailable unless explicit unsafe override is
     enabled in config.

3. **Restrict auto-safe execution patterns**
   - Keep common read-only commands auto-safe.
   - Require explicit approval for network fetch/pipe execution patterns and
     quarantine-bypass commands.

4. **Untrusted-by-default lifecycle**
   - Distinguish `installed` from `trusted` from `enabled`.
   - Require explicit trust action before enable.

### P1 - Supply Chain and Least Privilege

1. **Enforce tool policy at runtime**
   - Apply resolved policy to tool registry before dispatch.
   - Denylisted tools must be unavailable even if registered.

2. **Enforce skill `allowed_tools`**
   - Treat declared tools as runtime constraints, not informational metadata.

3. **Provenance pinning**
   - Record source URL, commit SHA, install time, and trust decision metadata in
     manifests.
   - Require re-trust when upstream commit changes.

4. **Installer hardening**
   - Reject tar entries with absolute paths, `..` traversal, or symlink escape.

### P2 - Detection and Response

1. **Audit log**
   - Append-only security events for install/trust/enable, approvals,
     suspicious command attempts, and overrides.

2. **Panic switch**
   - Single control to disable third-party skills/plugins immediately.

3. **Periodic scanner**
   - Rule-based and AI-assisted scan of installed skills/plugins for risky
     patterns (obfuscation, staged payload fetchers, credential scraping hints).

4. **Risk UX**
   - Persistent risk badge and provenance info in UI (not one-time dismissal).

## Implementation Backlog

### PR1: Runtime Approval Wiring

- Files:
  - `crates/gateway/src/server.rs`
  - `crates/tools/src/approval.rs`
- Deliverables:
  - Parse config values and instantiate `ApprovalManager` with explicit config.
  - Validate unknown values with safe fallback and warning.
  - Unit tests for mapping and behavior.

### PR2: Secure `skills.install_dep`

- Files:
  - `crates/gateway/src/services.rs`
  - `crates/skills/src/requirements.rs`
- Deliverables:
  - Approval-gated dependency install path.
  - Sandbox requirement with fail-closed behavior.
  - Risk signature checks for dangerous install patterns.
  - Tests for denied/approved/sandboxed paths.

### PR3: Runtime Tool Restriction

- Files:
  - `crates/gateway/src/chat.rs`
  - `crates/tools/src/policy.rs`
- Deliverables:
  - Apply effective tool policy in tool dispatch path.
  - Enforce `allowed_tools` when skills are activated.
  - Tests ensuring blocked tools cannot execute.

### PR4: Trust and Provenance Model

- Files:
  - `crates/skills/src/manifest.rs`
  - `crates/skills/src/types.rs`
  - `crates/gateway/src/services.rs`
  - `crates/gateway/src/assets/js/page-skills.js`
- Deliverables:
  - Add trust state and provenance fields.
  - Block enabling untrusted content.
  - Re-trust requirement on source change.

### PR5: Installer Path Safety

- Files:
  - `crates/skills/src/install.rs`
  - `crates/plugins/src/install.rs`
- Deliverables:
  - Canonical path checks for tar extraction.
  - Rejection tests for path traversal and symlink tricks.

### PR6: Audit + Kill Switch + UI

- Files:
  - `crates/gateway/src/services.rs`
  - `crates/gateway/src/assets/js/page-skills.js`
  - `docs/src/security.md`
- Deliverables:
  - Append-only security audit events.
  - Emergency disable for third-party content.
  - Persistent risk/provenance indicators in UI.

## Success Criteria

- Dependency installs cannot run silently on host.
- Untrusted skill install does not imply executable trust.
- Tool policy and `allowed_tools` are enforced at runtime.
- Every high-risk action has approval, audit trail, or both.
- Updates from previously trusted repos require re-validation.

## Rollout Strategy

1. Ship P0 with compatibility defaults and clear migration notes.
2. Add metrics counters for blocked actions and approval prompts.
3. Roll out P1/P2 in follow-up releases with docs updates.

## Open Questions

- Should trust decisions be global or per-session/per-project?
- Should org/repo allowlists be enabled by default in cloud deployments?
- Should we support signed skill manifests (future enhancement)?
