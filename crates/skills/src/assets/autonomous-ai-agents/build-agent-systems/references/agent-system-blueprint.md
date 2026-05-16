# Agent System Blueprint

Use this template when producing a concrete design for an agent product.

## 1. Product Frame

- Target users:
- Tenant model:
- Channels:
- Primary tasks:
- Human approval points:
- Data retention needs:
- Deployment target:

## 2. Agent Roles

| Agent | Purpose | Model Policy | Tool Policy | Memory Scope | Session Access | Limits |
|---|---|---|---|---|---|---|
| coordinator | Decompose and route work | default or strong reasoning | delegate/session tools only | project | cross-session as needed | low tool side effects |
| researcher | Gather evidence | cheap/fast or search-optimized | read/search/fetch only | project | read-only | no writes |
| operator | Execute bounded actions | reliable tool-use model | narrow side-effect tools | user/project | current session | approval required |
| reviewer | Check correctness and safety | strong reasoning | read/test tools | project | read-only | no writes |

Adapt rows to the actual product. Avoid creating roles without distinct policy needs.

## 3. Runtime Loop

```text
channel event / UI message / scheduled job
  -> authenticate and authorize sender
  -> deduplicate by event id
  -> resolve session key and channel binding
  -> load session history, memory, skills, runtime context
  -> build prompt and tool schema list
  -> run streaming model loop
  -> validate and execute tool calls
  -> persist assistant/tool messages
  -> publish stream events
  -> send final answer or error through reply route
```

## 4. Tool Registry

| Tool Class | Examples | Allowed Agents | Side Effects | Approval | Audit |
|---|---|---|---|---|---|
| read-only | search, fetch, history | most agents | none | no | basic |
| local mutation | file edit, skill patch | coder/operator | workspace writes | sometimes | required |
| external action | send message, create ticket | operator | external side effect | often | required |
| admin/config | update channel settings, MCP add | admin/operator | persistent config | yes | required |

Rules:

- hide disallowed tools before prompt assembly
- validate every parameter object before execution
- sanitize tool results before appending to model context
- classify tool errors as retryable or non-retryable

## 5. Skill System

Skill sources:

- bundled skills for product-owned workflows
- project skills for team/domain workflows
- personal skills for user-local customization
- registry/plugin skills for installable extensions

Skill lifecycle:

1. discover metadata
2. show trigger descriptions in prompt
3. load full skill only when needed
4. use references/scripts/assets progressively
5. checkpoint before agent-side mutations
6. watch for `SKILL.md` changes and refresh availability

## 6. Multi-Channel Model

| Channel | Inbound Mode | Streaming | Threads | Voice/File Handling | Access Control |
|---|---|---|---|---|---|
| chat app with webhook | HTTP webhook | optional | platform-specific | upload to media store | signature + allowlist |
| chat app with gateway | WebSocket/sync loop | often | platform-specific | gateway download | token + allowlist |
| polling bot | polling loop | edit-in-place | limited | API download | token + allowlist |
| web UI | direct API/WebSocket | native | session branches | browser upload | authenticated session |

Normalize all channels into one session/event contract instead of giving every channel its own agent runtime.

## 7. Kubernetes/Pod Architecture

```text
Ingress/Webhook Pods
  -> Event Queue
  -> Dispatcher Workers
  -> Agent Runner Pods
  -> Tool/Sandbox Pods
  -> Provider APIs and MCP Servers

Shared: SQL DB, object store/PV, pubsub, secret manager, observability, auth.
```

Design decisions:

- use horizontal pod autoscaling for ingress and runner pools separately
- use per-message idempotency keys
- use per-session leases when order matters
- keep stream fanout outside the runner process when possible
- make tool execution cancellable
- keep channel credentials in a secret manager, not pod environment dumps

## 8. Verification Plan

Minimum tests:

- prompt assembly includes identity, skills, memory, runtime context, and allowed tools
- denied tools are absent from model-visible schemas
- malformed tool calls are rejected without executing
- repeated failing tool calls trigger loop intervention
- channel duplicate events do not duplicate replies
- unauthorized channel senders get visible access feedback when policy allows
- approved sender LLM failures return an error/fallback message
- session history persists across runner pod restart
- skill metadata parses and reference paths are valid
- webhook signatures and SSRF protections are covered

## 9. Output Checklist

Deliver:

- roles and policies
- runtime sequence
- data model
- channel matrix
- tool matrix
- Skill draft
- deployment map
- security invariants
- test plan
- operational risks
