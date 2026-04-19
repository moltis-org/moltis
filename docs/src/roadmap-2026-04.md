# Moltis Product Roadmap

**Status:** Proposed
**Last updated:** 2026-04-17
**Based on:** Competitive intelligence report, internal `plans/` archive, Hermes gap analysis, community signals

---

## Positioning

> Moltis: The secure, Rust-native multi-agent server for operators who refuse to compromise on safety, performance, or control.

Not "another OpenClaw alternative." The **multi-agent, security-first, local-first** personal agent platform. Zero unsafe code. Zero security alerts.

---

## Strategic Pillars

| Pillar | Why |
|--------|-----|
| **Visibility** | Internal plans that users can't see don't exist. Publish or perish. |
| **Migration** | 135K exposed OpenClaw instances. 12,812 RCE-exploitable. Corporate bans accelerating. Every migrant is a potential advocate. |
| **Multi-Agent Depth** | Only Rust option with shipped orchestration. Deepen before OpenClaw v4.0 solidifies. |
| **Channel Breadth** | 8 channels vs ZeroClaw's 28+. Each missing channel is a migration blocker. |
| **Ecosystem** | OpenClaw's 13.7K skills on ClawHub are their deepest moat. Build a secure alternative. |
| **Enterprise Readiness** | NemoClaw, CrowdStrike, Palo Alto advisories prove buyers pay for agent security. |

---

## Phase 0: Foundation & Visibility

**Goal:** Stop being invisible. Capture migration demand at the door.

### 0.1 Publish This Roadmap
- Promote `ROADMAP.md` to repo root (synthesized from this document)
- Reference from README and docs landing page
- Tag releases against roadmap milestones
- CHANGELOG-driven milestone tracking
- Keep this document as the single source of truth — archive `plans/` docs as historical reference

### 0.2 Product Narrative Reset
- Rewrite README around the positioning statement
- Lead with security: `#![forbid(unsafe_code)]`, zero alerts, WASM sandbox
- Add "Why Moltis" comparison page in docs (vs OpenClaw, ZeroClaw, IronClaw)
- Create 60-second demo video/GIF for GitHub social preview
- Publish: "Why We Built Moltis in Rust" (technical blog post)
- Publish: "After the OpenClaw Security Crisis" (migration guide blog post)

### 0.3 OpenClaw Migration Tool
- Build `moltis migrate openclaw` CLI subcommand
- Import: config (JSON→TOML auto-conversion), channel settings, memory files, provider keys, workspace files
- Validate imported config, warn on unsupported channels, suggest Moltis equivalents
- `--dry-run` flag for read-only preview (match ZeroClaw's UX)
- Document migration path in dedicated docs page
- **This is the single highest-leverage action in the entire roadmap.**

### 0.4 One-Click Install
- Single-command install script (`curl -sSL https://moltis.org/install | sh` or equivalent)
- Official Docker image with `docker-compose.yml` stack
- Systemd service file for bare-metal Linux
- Homebrew formula for macOS
- Verify releases with `gh attestation verify` (already documented)

### 0.5 Community Presence
- Add GitHub topics, social preview image, description
- Cross-post to r/LocalLLaMA, Hacker News, r/rust
- Respond to every issue within 48 hours
- Add `CONTRIBUTING.md` and label good-first-issues
- Add GitHub Discussions for community Q&A

**Milestone:** Moltis is explainable in one sentence, discoverable via search, and trivially installable.

---

## Phase 1: Multi-Agent Moat

**Goal:** Deepen the multi-agent advantage into an unassailable differentiator. ZeroClaw and IronClaw have no multi-agent. OpenClaw just merged PR #27382. Moltis must be demonstrably ahead.

### 1.1 Agent Coordination Protocol
- **Supervisor pattern:** One coordinator agent routes tasks to specialists by declared capability
- **Shared memory bus:** Agents read/write to a common memory store with scoped access controls per agent
- **Task queue:** Lightweight internal queue for agent-to-agent delegation (no Redis, no external deps — SQLite-backed)
- **Failure handling:** Automatic retry with configurable limits, reassignment to alternate specialists, escalation to human operator
- **Cost budgets:** Per-task and per-session token/cost limits with enforcement

### 1.2 Agent Discovery & Registry
- Agents declare capabilities as typed tags on startup (e.g., `coding`, `research`, `writing`, `scheduling`)
- Dynamic routing: supervisor matches inbound requests to agent capabilities
- Health monitoring: `connected`, `idle`, `busy`, `degraded` states per agent
- **Sticky session binding:** Coding sessions pin to the same worker agent for context continuity
- Agent hot-reload: add/remove specialist agents without coordinator restart

### 1.3 Sub-Agent Orchestration UX
- **Live progress tree** in web UI: visualize supervisor → specialist → task hierarchy in real time
- Per-agent indicators: role label, model name, token usage, iteration count
- **Iteration budgets:** Max loops per task, configurable per-agent
- **Timeout controls:** Per-task and per-session timeouts with graceful degradation
- Handoff summaries: when a specialist completes, generate a structured summary for the supervisor
- **Named specialist sessions:** Long-lived agents (e.g., "code-reviewer", "research-assistant") that persist across conversations

### 1.4 Multi-Agent Testing Harness
- Contract tests for agent-to-agent communication protocol
- Mock agent framework for integration testing (inject test agents without real LLM calls)
- Load testing: simulate concurrent multi-agent coordination under realistic patterns
- Chaos testing: kill specialist agents mid-task, verify supervisor recovery

### 1.5 Multi-Agent Patterns Library
- Ship pre-built coordination patterns as templates:
  - **Research → Draft → Review:** research agent gathers context, writing agent drafts, review agent critiques
  - **Code → Test → Deploy:** coding agent writes code, test agent validates, deployment agent ships
  - **Triage → Route → Resolve:** intake agent classifies requests, router assigns, specialist resolves
- Each pattern is a YAML config that maps channels → agents → capabilities
- Users can define custom patterns

**Milestone:** Moltis is the only Rust agent platform with production-grade multi-agent orchestration, documented patterns, and a testing harness. OpenClaw has primitives; Moltis has a system.

---

## Phase 2: Security & Trust

**Goal:** Become the undisputed security leader in the agent space. Capitalize on OpenClaw's crisis. Make "zero unsafe code + zero alerts" the headline that wins migrations.

### 2.1 Deny-by-Default Permission Model
- **All tool access denied unless explicitly granted** — current allowlist model is too permissive for security-first positioning
- Granularity: per-agent, per-project, per-session, per-channel
- **Permission templates** for common workflows: "coding agent", "research agent", "chat agent", "admin agent"
- Each template defines which tools/paths/networks the agent can access
- **Audit log:** Every permission grant, deny, and escalation recorded with timestamp, agent ID, session ID
- Permission inheritance: project-level → session-level → agent-level (more specific overrides less specific)

### 2.2 Automatic Edit Checkpoints (Hardening)
- `checkpoint_restore` already exists — productize and expand
- **Shadow-git** or equivalent rollback mechanism before every file mutation
- No pollution of user's repo state (separate `.moltis-checkpoints/` or equivalent)
- One checkpoint per turn or per edit batch
- **Visible restore path** in both web UI and CLI
- Checkpoint metadata: timestamp, agent ID, session ID, files changed, diff summary
- Automatic checkpoint pruning (keep last N per project, configurable)

### 2.3 Context File Security
- **Prompt-injection scanning** before any project context file is injected into agent prompts
- Scanner flags: hidden instructions, role manipulation, tool abuse patterns, encoding tricks
- **Compatibility with `.cursorrules` and `.cursor/rules/*`** — load them but scan first
- **Context ingestion report** (per session):
  - Files loaded (with size)
  - Files skipped (with reason)
  - Size truncation warnings
  - Risk flags (injection patterns detected)
- **UI visibility:** Show the final context bundle composition in the web UI per session
- Configurable context size limits per file and per session

### 2.4 Supply Chain Hardening
- **Skill provenance metadata:** source URL, author identity, imported_at timestamp, content checksum (SHA-256)
- **Quarantine mode:** Third-party skills are inactive by default. Operator reviews and approves before activation.
- **Signed skill packages** (optional): GPG signatures for skill distributions, verify before install
- **Stale-skill detection:** Monitor skill usage patterns, flag skills that haven't been invoked in N days
- **Auto-patching:** When a skill's source checksum changes, alert the operator and offer to review/update
- **Skill dependency declaration:** Skills declare what tools and permissions they need; deny if requirements exceed policy

### 2.5 Network Security
- **Egress control:** Configurable allowlist for outbound network connections per agent
- **TLS pinning** for provider API connections (optional, for high-security deployments)
- **DNS resolution controls:** Prevent DNS exfiltration via agent tool calls
- **Secret scrubbing:** Scan all outbound messages (tool responses, agent outputs) for accidental credential/secret leaks

**Milestone:** External security audit commissioned. Results published. Blog post: "Moltis Security Architecture." Target: OWASP Agentic Top 10 full coverage.

---

## Phase 3: Channel Expansion

**Goal:** Close the channel gap from 8 to 15+. Each missing channel is a migration blocker. Prioritize channels that ZeroClaw supports but Moltis doesn't.

### Priority Order

| Priority | Channel | Rationale | Implementation Approach |
|----------|---------|-----------|------------------------|
| 1 | **Signal** | Highest-demand privacy messaging app. Migration from OpenClaw's #1 requested alternative. | signal-cli bridge or libsignal bindings |
| 2 | **Email (SMTP/IMAP)** | Universal reach. Every professional has email. No app install required. | SMTP for outbound, IMAP for inbound. Staged: outbound first. |
| 3 | **Webhook / HTTP** | Generic integration surface. Connect anything: CI/CD, monitoring, custom apps. | REST endpoint with HMAC auth. Configurable payload format. |
| 4 | **Matrix** | Self-hosted community standard. Federated. Overlaps with Moltis' values. | matrix-nio (Rust) or HTTP API |
| 5 | **Slack** | Enterprise requirement. Many OpenClaw migrants come from Slack-first orgs. | Slack Bolt SDK equivalent or HTTP API |
| 6 | **IRC** | Low complexity, high signal for technical communities. | IRC client library |
| 7 | **WebChat** | Embedded chat widget for websites. Zero install for end users. | WebSocket endpoint + embeddable JS snippet |

### Channel SDK
- **Extract common channel interface into a Rust trait:**
  ```
  trait Channel {
      async fn send_message(&self, ctx: &ChannelContext, msg: Message) -> Result<()>;
      async fn receive_messages(&self, ctx: &ChannelContext) -> Result<MessageStream>;
      async fn health_check(&self) -> Result<ChannelHealth>;
      fn capabilities(&self) -> ChannelCapabilities;
  }
  ```
- **Plugin-based channel loading:** Channels loaded at runtime from compiled Rust dylibs or WASM modules
- **Per-channel configuration:** Rate limits, message size limits, allowed content types, retry policies
- **Unified message format:** All channels normalize to a common `Message` type internally
- **Channel testing harness:** Mock channel implementations for integration testing

### Channel Health Dashboard
- Per-channel status: `connected`, `disconnected`, `degraded`, `rate_limited`
- Metrics: message throughput (sent/received per minute), error rates, latency percentiles
- **Reconnection logic:** Exponential backoff with jitter, configurable max retry, manual reconnect button
- **Alerts:** Notify operator (via another channel) when a channel is degraded for > N minutes
- **Message queue:** Buffer messages during disconnection, deliver on reconnect

**Milestone:** 15+ channels. Migration tool covers all channels that OpenClaw migrants are likely using.

---

## Phase 4: Memory & Intelligence

**Goal:** Make Moltis "feel smarter" than every alternative. Memory is the loop that makes agents feel alive.

### 4.1 Built-in Vector Memory
- **SQLite + sqlite-vec** for zero-dependency vector search (no Docker, no external service)
- Automatic embedding of: conversations, task results, operator documents
- **Pluggable backends:** SQLite (default, zero-config), pgvector (PostgreSQL), ChromaDB (optional Docker), custom via trait
- **Transparent context augmentation:** Relevant memories injected into agent context without manual RAG configuration
- Configurable retrieval: top-K results, similarity threshold, recency weighting
- Memory deduplication: avoid storing near-duplicate content
- Embedding model: configurable (default to a small local model for privacy; optional cloud embeddings)

### 4.2 Cross-Session Recall (Enhancement)
- `sessions_search` already ships — enhance with **semantic search** (vector similarity, not just keyword)
- **Automatic summarization** of long sessions: generate condensed summaries when sessions exceed N messages
- **Project-scoped recall:** "What did I work on in project X?" retrieves only sessions tagged to that project
- **Global recall:** "What did I work on last week?" across all projects
- **Temporal queries:** Natural language date ranges ("last month", "since March", "this week")
- **Entity extraction:** Automatically tag sessions with detected entities (project names, people, technologies)

### 4.3 Proactive Intelligence
- **Scheduled context consolidation:** Daily or weekly job that summarizes recent activity into MEMORY.md updates
- **Skill suggestion:** After complex workflows, agent proposes creating a reusable skill from the pattern
- **Stale-memory detection:** Flag memories that reference outdated information (based on file modification dates, config changes)
- **Learning loops:** Agent identifies repeated patterns in operator behavior, suggests automations or shortcuts
- **Proactive notifications:** Agent surfaces relevant past context unprompted when it detects a related task starting

### 4.4 Memory Viewer & Management
- **Browse:** UI for browsing all stored memories, grouped by source (conversation, document, manual)
- **Search:** Full-text and semantic search across all memories
- **Delete:** Remove individual memories or bulk-delete by age/source/project
- **Manual add:** Operator can directly inject context ("Remember: project X uses PostgreSQL 16, not 15")
- **Statistics:** Memory usage breakdown by source, age distribution, embedding coverage
- **Export:** Download all memories as structured JSON or Markdown

### 4.5 Project Context Intelligence
- **Automatic project awareness:** Detect project type (Rust, Python, Node, etc.) from directory structure
- **Load relevant context automatically:** `Cargo.toml`, `package.json`, `pyproject.toml` → inject key metadata
- **Dependency awareness:** Track project dependencies and flag when agent suggestions conflict with installed versions
- **Build/test integration:** Agent understands project build system, can run tests in context

**Milestone:** Users report that "Moltis remembers things I forgot" without being prompted. Memory feels like a natural extension of the operator's own knowledge.

---

## Phase 5: Ecosystem & Growth

**Goal:** Build community gravity. Transform Moltis from a tool into a platform.

### 5.1 Portable Skill Format
- **Import/export** for personal skills including all sidecar files (references/, templates/, assets/, scripts/)
- **Standardized skill manifest** (`SKILL.yaml` or equivalent):
  ```yaml
  name: my-skill
  version: 1.2.0
  author: operator-name
  description: Does X, Y, Z
  permissions: [exec, read, write]
  tools: [web_fetch, memory_search]
  moltis-version: ">=2026.4.0"
  ```
- **Skill archive format:** `.mskill` tarball containing manifest + SKILL.md + sidecars
- **Install from URL:** `moltis skill install https://example.com/skills/my-skill.mskill`
- **Versioning:** Skills declare compatible Moltis versions; warn or block incompatible installs

### 5.2 Curated Skill Registry
- **Hosted registry** (not open — curated for supply chain safety)
- Submission process: skill review, security audit, quality check
- **Trust indicators per skill:** verified author, review status, install count, last-updated date
- **Skill categories:** coding, devops, research, communication, productivity, monitoring
- **Search:** By name, category, permission requirements, Moltis version compatibility
- **CLI integration:** `moltis skill search <query>`, `moltis skill install <name>`, `moltis skill update <name>`
- Start with 20-50 high-quality built-in skills before opening to community submissions

### 5.3 Plugin SDK
- **Typed interfaces** for tool plugins and channel plugins (Rust traits)
- **Install-time validation:** Check plugin requirements (Rust version, Moltis version, system deps) before activation
- **Semantic versioning:** Plugins declare supported Moltis versions
- **Built-in test runner:** Plugin authors write tests against a sandboxed Moltis instance
- **Permission scoping:** Plugins declare required permissions; operator approves/denies per permission
- **Plugin isolation:** Plugins run in WASM sandbox or separate process; compromised plugin cannot access host

### 5.4 Web Dashboard Enhancements
- **Agent management:** Create, configure, start, stop, monitor agents from browser
- **Skill browser:** Browse, search, install, configure skills from the registry
- **Channel management:** Add/remove/configure channels, view health status
- **Scheduling:** Visual calendar for cron jobs, create/edit/delete scheduled tasks
- **Conversation history:** Browse and search past conversations across all channels
- **Memory viewer:** Full memory management UI (as described in Phase 4.4)
- **Real-time log streaming:** Live agent activity log with filtering by agent/session/channel
- **Health metrics:** System resource usage, agent performance, error rates
- **Configuration:** Make 80% of configuration possible without touching config files

### 5.5 Documentation Overhaul
- **Quickstart:** Working agent in under 5 minutes (install → configure → first message)
- **Tutorials** for top 5 use cases:
  1. Personal assistant (Telegram + memory + scheduling)
  2. Coding agent (multi-agent: coder + reviewer + tester)
  3. Team bot (Slack/Discord + multi-agent + shared memory)
  4. DevOps agent (SSH + webhooks + monitoring)
  5. Migration from OpenClaw (step-by-step guide)
- **API reference:** Auto-generated from Rust doc comments
- **Architecture guide:** How multi-agent, channels, memory, and sandbox fit together
- **Video walkthroughs:** 2-5 minute demos for each major feature
- **Multi-language docs:** Start with Chinese (zh), Japanese (ja), Korean (ko) — highest Claw-ecosystem demand

**Milestone:** Third-party skills published for Moltis. Community contributions accelerating. Documentation is a competitive advantage, not a liability.

---

## Phase 6: Enterprise Preparation

**Goal:** Make Moltis credible for team deployments without becoming IronClaw. Capture the segment that wants security without Kubernetes.

### 6.1 SSO / OIDC
- OIDC provider support (Google Workspace, Azure AD, Okta, Keycloak)
- Per-organization identity mapping
- Session delegation: team members can share agent sessions with scoped access
- Role-based access within the organization: admin, operator, viewer

### 6.2 Structured Audit Logging
- **Tamper-evident log** for every agent action: tool calls, file edits, channel messages, permission changes
- Log format: JSON with cryptographic chain (each entry references the previous entry's hash)
- **Compliance-ready export:** JSON, CSV, SIEM-compatible (CEF/Syslog format)
- **Filtering:** By agent, session, channel, time range, action type
- **Retention policies:** Configurable log retention with automatic pruning
- **Log viewer:** Built-in UI for browsing and searching audit logs

### 6.3 Multi-Tenancy
- **Organization-level isolation:** Separate config, memory, skills, and sessions per org
- **Per-org namespaces:** No cross-org data leakage
- **Usage quotas:** Token limits, cost caps, and rate limits per organization
- **Resource isolation:** Optional per-org processes or containers
- **Admin dashboard:** Organization management, user provisioning, usage analytics

### 6.4 Deployment Options
- **Official Docker image** with production-ready `docker-compose.yml` (PostgreSQL, Redis optional, volume mounts)
- **Helm chart** for Kubernetes deployments (for orgs that want K8s despite Moltis not being K8s-native)
- **Systemd service file** with proper lifecycle management
- **One-click install script** (from Phase 0.4, hardened for production)
- **Configuration validation:** `moltis config validate` checks config before startup, reports issues with actionable fixes
- **Health endpoints:** `/health`, `/ready`, `/metrics` for load balancer integration

### 6.5 Backup & Recovery
- **Automated backup:** Scheduled backups of config, memory, sessions, and skill data
- **Backup formats:** SQLite dump, tarball, or S3-compatible upload
- **Restore:** `moltis restore <backup-file>` with validation before applying
- **Disaster recovery:** Documented recovery procedures for common failure modes

**Milestone:** First enterprise deployment with compliance requirements (audit logging, SSO, multi-tenancy) fully met.

---

## Success Metrics

| Metric | Current | Next Target | Stretched Target |
|--------|---------|-------------|------------------|
| GitHub stars | 2.6K | 10K | 25K |
| Channels | 8 | 12 | 15+ |
| Contributing devs | ~45 | 70 | 100+ |
| OpenClaw migrations | 0 | 500+ | 2,000+ |
| Community skills (registry) | 0 | 20 | 100+ |
| Public roadmap | No | Yes | Updated regularly |
| Security audit | None | Commissioned | Passed + published |
| Install time | Manual | One command | One command |
| Documentation | Good | Excellent | Best-in-class |

---

## Non-Goals (Explicitly Out of Scope)

- **Mobile app** — Use channels (Telegram, Signal, Discord mobile) instead. A mobile app is a distraction.
- **Cloud-hosted managed service** — Self-host only. Managed hosting creates supply-chain and trust concerns that conflict with security-first positioning.
- **Python SDK or runtime** — Rust-first. Expose APIs (HTTP, gRPC) for integration; don't maintain a second language runtime.
- **RL / trajectory generation** — Research, not product. Let the ML research community handle this.
- **Chasing every channel** — Prioritize channels that cover the most migration demand. Don't build LINE, WeChat, or Twitch unless demand signals justify it.
- **Copying OpenClaw's security-optional defaults** — Security-by-default is the brand. Never compromise this.
- **Desktop app (near-term)** — Web UI is sufficient. Revisit if demand signals warrant it.

---

## Risks & Mitigations

| Risk | Impact | Probability | Mitigation |
|------|--------|-------------|------------|
| OpenClaw v4.0 multi-agent solidifies before Moltis deepens moat | High | Medium | Phase 1 is top priority after Phase 0. Ship patterns library. |
| ZeroClaw captures majority of migration demand | High | High | Phase 0.3 migration tool is table stakes. Lead with security differentiation. |
| IronClaw (NEAR-backed) accelerates with financial runway | Medium | Medium | Different positioning (blockchain-adjacent). Focus on general-purpose security. |
| Microsoft M365 Copilot reduces self-hosted enterprise demand | Medium | Medium | Don't compete with Microsoft. Target orgs that want self-hosted control. |
| Resource constraints (small team, AI-accelerated but finite) | High | High | Prioritize ruthlessly. Phase 0-2 first. Cut scope before cutting quality. |
| Breaking changes during migration wave | Medium | Low | Semantic versioning. Migration tool handles version translation. |
| Security vulnerability discovered in Moltis | Critical | Low | Phase 2 hardening. External audit. Bug bounty program. Incident response plan. |
| Market fragmentation dilutes attention | Medium | High | Lead with a clear narrative. "Secure Rust multi-agent" is distinct from "fast Rust" (ZeroClaw) or "private Rust" (IronClaw). |

---

## Execution Principles

1. **Publish or perish.** Internal plans that users can't see don't exist. This roadmap must be public.
2. **Ship the differentiator first.** Multi-agent is the moat. Security is the brand. Deepen both before expanding surface area.
3. **Security is non-negotiable.** Every feature ships with a security review. `#![forbid(unsafe_code)]` is a constraint, not a suggestion.
4. **Migration is marketing.** Every OpenClaw refugee who successfully migrates is a potential advocate. Make the path frictionless.
5. **Measure what matters.** Stars, migrations, community skills, audit results — not just commits and PRs.
6. **AI-accelerated execution.** Time estimates are meaningless in this era. Ship when ready. Iterate fast.
7. **Ruthless prioritization.** If a phase blocks on another, reorder. If a feature doesn't serve a strategic pillar, cut it.
8. **Community before features.** A feature with no users is noise. A community with no features is potential. Build both, but community compounds.
