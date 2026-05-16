# Moltis-Derived Agent Patterns

This reference captures reusable agent-system patterns visible in Moltis. Use it as architecture knowledge, not as code to copy.

## Pattern 1: Preset-Driven Agent Roles

Represent an agent role as data:

- identity: name, tone, persona, and user-facing role
- model selection: explicit model, preset model, then default model
- tool policy: allow-list first, deny-list second
- session policy: which sessions can be read or messaged
- memory scope: user, project, or local
- safety limits: max iterations, timeout, delegation restrictions

Why it matters: roles become configurable and auditable instead of hardcoded prompt forks.

## Pattern 2: Tool Registry with Source Metadata

Expose tools through a uniform contract:

- stable name
- short description
- JSON parameter schema
- async execution
- source metadata such as built-in, MCP, or component

Keep the registry cloneable and filterable so each agent receives only the tools it may use. Preserve source metadata for auditing and policy decisions.

## Pattern 3: Lazy Tool Discovery

When the tool surface is large, start the model with a small `tool_search` meta-tool. The model searches by keyword, activates exact tools, and receives full schemas on later turns.

Use lazy discovery to reduce prompt bloat, but increase iteration limits because tool discovery itself consumes turns.

## Pattern 4: Streaming Agent Loop with Tool Turns

A robust loop has these phases:

1. build typed messages from system prompt, history, and current user input
2. list currently available tool schemas
3. call the provider, preferably streaming when the surface can display deltas
4. accumulate text, reasoning, tool-call starts, and argument deltas
5. validate and normalize tool names and arguments
6. execute allowed tool calls
7. append sanitized tool results
8. compact or truncate old tool results before context overflow
9. continue until final answer, max iterations, or a controlled error

Emit runner events for thinking, text deltas, tool start/end, rejections, retries, loop interventions, and sub-agent lifecycle.

## Pattern 5: Tool-Call Recovery and Guardrails

Models produce imperfect tool calls. Handle common failures explicitly:

- trim and normalize tool names
- repair JSON-like argument strings when safe
- reject invalid arguments before execution
- tell the model how to retry malformed tool calls
- detect repeated identical failures
- first nudge the model to explain and answer in text
- then strip tool schemas for one turn if it keeps looping

This keeps bad tool calls visible while preventing runaway loops.

## Pattern 6: Hook Boundaries

Place hook points around high-risk boundaries:

- before LLM call
- after LLM call
- before tool call
- after tool call

Hooks may continue, block, or modify payloads where the type system allows it. Use this for logging, approvals, policy checks, auditing, and integration tests.

## Pattern 7: Skill as Progressive Disclosure

Make Skills metadata-first:

- frontmatter name and description are the trigger surface
- `SKILL.md` contains the minimum workflow
- `references/` contains larger pattern guides and templates
- `scripts/` contains deterministic helper code only when repeated and fragile
- `assets/` contains output resources, not explanatory docs

Dynamic skill creation should checkpoint first, validate paths, reject hidden or escaping sidecar writes, and audit mutations.

## Pattern 8: Session-Centered Multi-Channel Routing

Normalize every channel event into a session:

- compute or look up a session key from channel type, account, chat, and thread
- persist the channel binding on the session
- inject channel metadata into the agent call
- mark inbound activity as seen
- route final text, stream deltas, and errors back through the binding

This lets web UI, channel messages, and proactive tools share one conversation model.

## Pattern 9: Channel Plugin Contract

Represent channels as plugins with a lifecycle:

- descriptor: type, display name, inbound mode, capabilities
- start account
- stop account
- account status and config view
- outbound sender
- stream outbound sender when supported
- event sink for inbound messages, commands, voice, files, interactions, and pairing

Contract-test every channel for lifecycle, duplicate start, unknown stop, outbound behavior, streaming completion, and retryable versus non-retryable errors.

## Pattern 10: Shared Access Control Vocabulary

Use the same policy vocabulary across channels:

- DM policy: open, allowlist, disabled
- group policy: open, allowlist, disabled
- mention mode: mention, always, none
- allowlist matching with exact and wildcard support
- OTP or explicit approval for onboarding when supported

Default private access conservatively. Always send visible fallback or error messages to approved senders.

## Pattern 11: Proactive Outbound Tools

Give agents narrow tools for outbound channel side effects:

- `send_message` style tool for intentional proactive messages
- `update_channel_settings` style tool for non-secret channel settings only
- per-channel model or agent routing overrides

Do not expose arbitrary raw config editing or secret mutation as an agent tool.

## Pattern 12: Distributed Pod Mapping

For Kubernetes-like deployments, map the patterns to pods:

- channel ingress pods: receive webhooks, polling, socket, or gateway events
- dispatcher workers: deduplicate, authorize, bind session, enqueue agent runs
- agent runner pods: execute model loop and stream events
- tool execution pods: run sandboxed shell/browser/filesystem work
- MCP connector pods: maintain external tool-server connections
- scheduler pods: enqueue cron or proactive jobs
- UI/API pods: serve web UI and session streams

Shared infrastructure:

- SQL database for session metadata, auth, channel accounts, and job state
- object storage or persistent volumes for JSONL session logs, media, and artifacts
- queue for inbound events and agent jobs
- pub/sub for stream deltas and UI/channel fanout
- distributed lock or lease keyed by session/message id
- secret manager for tokens and provider keys

## Pattern 13: Idempotency and Crash Recovery

Every inbound message should have a stable idempotency key. Store processing status before executing side effects. On retry, resume or no-op rather than sending duplicate replies.

Use per-session append-only logs for durable history. Keep runner pods stateless enough that a crashed pod can be replaced by another worker using the shared store.

## Pattern 14: Security and Safety Invariants

Require these invariants before shipping:

- validate webhook signatures and WebSocket origins
- block SSRF to private, loopback, link-local, and metadata ranges
- store secrets with redaction and expose them only at consumption points
- keep tool schemas narrow and typed
- reject unsafe path writes for Skills and artifacts
- enforce tool allow/deny policy before the model sees schemas
- audit tool execution and skill mutations
- surface errors to authorized users instead of failing silently
- keep channel credential storage and rotation behavior documented

## Pattern 15: Knowledge Extraction Method

When extracting from an existing agent codebase:

1. list runtime primitives: agents, tools, skills, sessions, memory, channels, hooks
2. identify contracts and invariants, not implementation details
3. map each contract to a product capability
4. separate portable patterns from product-specific names
5. write the Skill that makes another agent reproduce the design
6. verify against source evidence and expected user workflows
