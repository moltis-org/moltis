# Generic Webhooks Plan

## Summary

Add a first-class **generic webhook** feature to Moltis so users can create
named inbound HTTP endpoints from the web UI, bind them to an agent persona,
apply execution restrictions, and turn each delivery into a persistent chat
session that can be inspected and continued later.

The key use case: **run AI agents in response to external events** — GitHub PRs,
GitLab merge requests, Stripe payments, alerting systems, CI/CD pipelines, or
any service that can POST JSON to a URL.

This should be a new feature area, not a hack on top of channel webhooks.
Existing channel webhook middleware is for platform adapters like Slack and
Teams, not for arbitrary user-defined webhook ingress.

## Goals

- Let users create, edit, view, and delete generic webhooks from the web UI.
- Give each webhook a stable public endpoint with a high-entropy identifier.
- Bind a webhook to an existing agent persona.
- Allow webhook-specific execution overrides:
  - model override
  - extra system prompt suffix
  - tool allow/deny policy
  - optional sandbox override
- Support common webhook authentication patterns used by major senders.
- Persist every accepted delivery with metadata, status, and session linkage.
- Create a normal persistent chat session per delivery by default.
- Make resulting sessions easy to inspect and continue in the standard chat UI.
- Provide **source profiles** with event filtering, payload normalization, and
  response actions for GitHub, GitLab, Stripe, and other major providers.
- Let the agent **act back** on the source system (post PR comments, update
  issues, trigger deployments) via configured API credentials and tools.

## Non-Goals

- Do not make generic webhooks part of the existing `channels` feature.
- Do not require users to define a full new agent per webhook.
- Do not block webhook HTTP responses on full LLM execution.
- Do not attempt a full workflow engine in v1.
- Do not build a visual DAG/pipeline editor.
- Do not include webhooks in the onboarding flow. Webhooks are a power-user
  feature that lives exclusively in **Settings → Webhooks**. No onboarding
  cards, no setup wizard on first run, no "get started with webhooks" prompts
  in the main UI. Users discover it when they need it.

## Product Shape

### Conceptual Model

A generic webhook is:

- a public endpoint
- a source profile (defines auth, event parsing, response actions)
- a verification policy
- an event filter
- a delivery log
- an execution target (agent + overrides)

The execution target should be:

- an existing `agent_id`
- plus optional webhook-local overrides

This keeps Moltis aligned with existing agent personas and workspace files:

- `IDENTITY.md`
- `SOUL.md`
- `AGENTS.md`
- `TOOLS.md`

The webhook should not define a completely separate identity stack unless we
later decide generic webhooks are their own long-lived agent type.

### Default Execution Behavior

Default behavior for each accepted delivery:

1. Verify auth and source policy.
2. Parse event type from source-specific headers.
3. Check event filter — skip if event type is not in the allow list.
4. Deduplicate replayed deliveries.
5. Persist the delivery record.
6. Return `202 Accepted` quickly.
7. Spawn async processing.
8. Create a new persistent named session.
9. Assign the configured `agent_id` to that session.
10. Inject a normalized inbound message into chat.
11. Run `chat.send_sync`.
12. Update delivery status and link to the resulting session.

This mirrors the cron execution pattern: cron creates a session key like
`cron:{name}` or `cron:{uuid}`, calls `chat.send_sync(params)` with optional
model override, and records run status. Webhooks follow the same shape but with
richer inbound metadata.

## Why Not Reuse Channels

Channels are human-facing messaging surfaces with reply routing, streaming,
presence semantics, and platform-specific behavior.

Generic webhooks are infrastructure ingress. They do not map well to:

- DM/group policies
- outbound reply targets
- channel account setup flows
- channel capability flags

The clean boundary is:

- `channels`: human messaging transports
- `webhooks`: machine event ingress

## Source Profiles

Source profiles are the bridge between raw HTTP delivery and structured agent
context. Each profile defines how to authenticate, parse, filter, normalize,
and respond to events from a specific provider.

### Profile Architecture

A source profile provides:

| Concern | What the profile defines |
|---------|--------------------------|
| **Auth** | Verification method (HMAC, token, etc.) and config schema |
| **Event parsing** | How to extract event type from headers/body |
| **Idempotency** | Which header or field is the delivery ID |
| **Event catalog** | Known event types with descriptions for the UI filter |
| **Payload normalization** | How to produce a structured summary for the agent prompt |
| **Response actions** | What tools/credentials the agent needs to act back on the source |
| **Setup guidance** | UI copy, links to docs, example configurations |

Profiles are implemented as a Rust trait so new providers are just new
implementations, not special cases scattered through the codebase.

### `SourceProfile` Trait (Sketch)

```rust
pub trait SourceProfile: Send + Sync {
    /// Profile identifier used in config and UI.
    fn id(&self) -> &str;

    /// Human-readable name for the UI.
    fn display_name(&self) -> &str;

    /// Auth mode this profile recommends.
    fn default_auth_mode(&self) -> AuthMode;

    /// Known event types with descriptions (for UI filter checkboxes).
    fn event_catalog(&self) -> &[EventCatalogEntry];

    /// Extract event type from request headers and/or body.
    fn parse_event_type(&self, headers: &HeaderMap, body: &[u8]) -> Option<String>;

    /// Extract idempotency key from request.
    fn parse_delivery_key(&self, headers: &HeaderMap, body: &[u8]) -> Option<String>;

    /// Verify request authenticity. Called before any processing.
    fn verify(&self, config: &AuthConfig, headers: &HeaderMap, body: &[u8]) -> Result<()>;

    /// Produce a structured, human-readable summary of the payload for the
    /// agent prompt. This is NOT the raw JSON dump — it's a curated extraction
    /// of the fields that matter for the event type.
    fn normalize_payload(
        &self,
        event_type: &str,
        body: &serde_json::Value,
    ) -> NormalizedPayload;

    /// Additional tools this profile injects into the agent session.
    /// For example, GitHub profile adds `github_post_comment`,
    /// `github_create_review`, etc.
    fn response_tools(&self) -> Vec<ToolDefinition>;

    /// UI setup guidance: markdown with setup steps, links, screenshots.
    fn setup_guide(&self) -> &str;
}
```

### Built-in Profiles

#### Generic

Bare-bones profile. No event parsing magic, no response tools. Raw JSON
delivered to agent as-is.

- Auth: `static_header` or `bearer`
- Event type: extracted from configurable header name, or `unknown`
- Idempotency: configurable header name, or hash of body
- Normalization: pretty-printed JSON with truncation
- Response tools: none

#### GitHub

Full integration with GitHub webhooks and API.

**Auth:**
- `github_hmac_sha256` — verify `X-Hub-Signature-256` against configured secret
- Optionally layer CIDR allowlist from [GitHub meta API](https://api.github.com/meta)

**Event Parsing:**
- Event type: `X-GitHub-Event` header + `.action` field → `pull_request.opened`
- Delivery ID: `X-GitHub-Delivery` header

**Event Catalog (subset — expand as needed):**

| Event | Description | Common use case |
|-------|-------------|-----------------|
| `pull_request.opened` | New PR | Code review, labeling |
| `pull_request.synchronize` | PR updated with new commits | Re-review |
| `pull_request.closed` | PR closed/merged | Cleanup, changelog |
| `push` | Commits pushed | CI trigger, deploy check |
| `issues.opened` | New issue | Triage, auto-respond |
| `issues.labeled` | Issue labeled | Route to agent |
| `issue_comment.created` | New comment on issue/PR | Respond to questions |
| `pull_request_review.submitted` | PR review submitted | Respond to review |
| `release.published` | New release | Announce, post-release tasks |
| `check_suite.completed` | CI finished | Report results |
| `workflow_run.completed` | GitHub Actions workflow done | Post-CI analysis |

**Payload Normalization:**

Instead of dumping the entire GitHub webhook payload (which can be 50+ KB for a
PR event), extract the fields that matter:

```text
GitHub event: pull_request.opened

Repository: moltis-org/moltis
PR #567: "Add webhook support"
  Author: @penso
  Branch: feature/webhooks → main
  URL: https://github.com/moltis-org/moltis/pull/567
  Labels: enhancement, needs-review
  Draft: false

Description:
  This PR adds generic webhook support to Moltis...

Changed files: 42 (+1,203 / -156)

Full payload available as delivery attachment.
```

This gives the agent actionable context without wasting tokens on nested API
objects it doesn't need.

**Response Actions / Tools:**

When a GitHub source profile is configured with an API token (PAT or GitHub App
installation token), the webhook session gets additional tools:

| Tool | What it does |
|------|--------------|
| `github_post_comment` | Post a comment on a PR or issue |
| `github_create_review` | Submit a PR review (approve, request changes, comment) |
| `github_add_labels` | Add labels to an issue or PR |
| `github_remove_labels` | Remove labels from an issue or PR |
| `github_create_issue` | Create a new issue |
| `github_close_issue` | Close an issue |
| `github_merge_pr` | Merge a pull request |
| `github_request_reviewers` | Request reviewers on a PR |
| `github_create_check_run` | Create/update a check run with status |
| `github_get_diff` | Fetch the PR diff (for code review agents) |
| `github_get_file` | Fetch file contents at a specific ref |
| `github_list_files` | List changed files in a PR |

These tools are implemented using the GitHub REST API (or GraphQL where needed)
and configured with the stored API token. They are **only available in webhook
sessions for this profile**, not globally.

**Setup Guide:**

The UI shows step-by-step instructions:
1. Go to your GitHub repo → Settings → Webhooks → Add webhook
2. Payload URL: `{your-moltis-url}/api/webhooks/{public_id}`
3. Content type: `application/json`
4. Secret: `{generated_secret}` (with copy button)
5. Select events: (checkboxes matching event filter)
6. For response actions: create a GitHub PAT or App with required permissions

**GitHub App Support (Phase 2):**

For organizations, support GitHub App installation as an alternative to PATs:
- App receives webhooks natively
- Installation token auto-refreshes
- Fine-grained permissions per repository
- Avoids personal token lifetime/scope issues

#### GitLab

**Auth:**
- `gitlab_token` — verify `X-Gitlab-Token` header
- Support `X-Gitlab-Instance` for self-hosted GitLab

**Event Parsing:**
- Event type: `X-Gitlab-Event` header → `Merge Request Hook`, normalized to
  `merge_request.open`, `merge_request.merge`, etc.
- Delivery ID: `Idempotency-Key` when present, else derive from body hash

**Event Catalog:**

| Event | Description | Common use case |
|-------|-------------|-----------------|
| `merge_request.open` | New MR | Code review |
| `merge_request.update` | MR updated | Re-review |
| `merge_request.merge` | MR merged | Post-merge actions |
| `push` | Commits pushed | CI trigger |
| `note` (on MR/issue) | New comment | Respond to questions |
| `issue.open` | New issue | Triage |
| `pipeline` | Pipeline status change | Report failures |
| `release` | New release created | Changelog, announce |

**Payload Normalization:**

Similar to GitHub — extract repo, MR/issue number, author, branch, URL,
description. GitLab payloads have a different structure but the normalized
output should be comparable.

**Response Tools:**

| Tool | What it does |
|------|--------------|
| `gitlab_post_comment` | Comment on MR or issue |
| `gitlab_create_review` | Submit MR review notes |
| `gitlab_add_labels` | Add labels |
| `gitlab_approve_mr` | Approve a merge request |
| `gitlab_merge_mr` | Merge a merge request |
| `gitlab_create_issue` | Create a new issue |
| `gitlab_get_diff` | Fetch MR diff |
| `gitlab_get_file` | Fetch file at ref |

Uses GitLab REST API v4 with stored personal/project access token.

#### Stripe

**Auth:**
- `stripe_webhook_signature` — verify `Stripe-Signature` header using Stripe's
  [signature verification](https://docs.stripe.com/webhooks/signatures)
  (timestamp + HMAC-SHA256 with `whsec_` signing secret)
- Timestamp tolerance check (default 300s) to reject replayed old events

**Event Parsing:**
- Event type: `$.type` field in body (e.g., `checkout.session.completed`)
- Delivery ID: `$.id` field (event ID like `evt_1234...`)
- API version: `$.api_version` — log for debugging version mismatches

**Event Catalog:**

| Event | Description | Common use case |
|-------|-------------|-----------------|
| `checkout.session.completed` | Successful checkout | Fulfill order, send confirmation |
| `payment_intent.succeeded` | Payment captured | Update order status |
| `payment_intent.payment_failed` | Payment failed | Notify customer, retry logic |
| `invoice.paid` | Invoice paid | Activate subscription |
| `invoice.payment_failed` | Invoice payment failed | Dunning, notify |
| `customer.subscription.created` | New subscription | Provision access |
| `customer.subscription.updated` | Subscription changed | Adjust access/billing |
| `customer.subscription.deleted` | Subscription canceled | Revoke access |
| `charge.dispute.created` | Chargeback opened | Alert, gather evidence |
| `account.updated` | Connected account updated | Verify status |

**Payload Normalization:**

```text
Stripe event: checkout.session.completed

Event ID: evt_1OxBvY2eZvKYlo2C
API version: 2024-12-18
Livemode: true

Checkout Session:
  ID: cs_live_a1b2c3...
  Customer: cus_MNO456
  Email: customer@example.com
  Amount: $49.99 USD
  Payment status: paid
  Mode: subscription
  Subscription: sub_XYZ789

Metadata:
  plan: pro
  referral: campaign_spring

Full payload available as delivery attachment.
```

**Response Tools:**

| Tool | What it does |
|------|--------------|
| `stripe_retrieve_customer` | Fetch customer details |
| `stripe_update_customer` | Update customer metadata |
| `stripe_retrieve_subscription` | Fetch subscription details |
| `stripe_create_invoice` | Create a new invoice |
| `stripe_issue_refund` | Issue a refund |
| `stripe_send_invoice` | Finalize and send an invoice |

Uses Stripe API with stored restricted API key (recommend `rk_` keys with
minimal required permissions).

**Setup Guide:**

1. Go to Stripe Dashboard → Developers → Webhooks → Add endpoint
2. Endpoint URL: `{your-moltis-url}/api/webhooks/{public_id}`
3. Select events to listen to
4. Copy the signing secret (`whsec_...`) into Moltis
5. For response actions: create a restricted API key with only needed permissions

#### Linear

**Auth:**
- `linear_webhook_signature` — verify using Linear's
  [webhook signature](https://linear.app/docs/graphql/webhooks) (HMAC-SHA256
  with signing secret in `Linear-Signature` header)

**Event Parsing:**
- Event type: `$.type` + `$.action` → `Issue.create`, `Comment.create`
- Delivery ID: `$.webhookId` + `$.webhookTimestamp`

**Event Catalog:**

| Event | Description | Common use case |
|-------|-------------|-----------------|
| `Issue.create` | New issue | Auto-triage, assign |
| `Issue.update` | Issue updated | Track progress |
| `Comment.create` | New comment | Respond, summarize |
| `Project.update` | Project changed | Status reports |
| `Cycle.update` | Cycle updated | Sprint review |

**Response Tools:**

| Tool | What it does |
|------|--------------|
| `linear_create_issue` | Create issue |
| `linear_update_issue` | Update issue fields |
| `linear_post_comment` | Comment on issue |
| `linear_assign_issue` | Assign to user |

#### Jira (via Atlassian Connect)

**Auth:**
- `atlassian_connect` — verify JWT in `Authorization` header using shared
  secret from app installation
- Alternatively: `static_header` with a user-configured secret for simple
  webhook setups

**Event Parsing:**
- Event type: `$.webhookEvent` (e.g., `jira:issue_created`)
- Delivery ID: `$.timestamp` + `$.webhookEvent` + `$.issue.key`

**Event Catalog:**

| Event | Description |
|-------|-------------|
| `jira:issue_created` | New issue |
| `jira:issue_updated` | Issue updated |
| `comment_created` | New comment |
| `sprint_started` | Sprint started |
| `sprint_closed` | Sprint ended |

#### PagerDuty

**Auth:**
- `pagerduty_v2_signature` — verify `X-PagerDuty-Signature` (HMAC-SHA256)

**Event Parsing:**
- Event type: `$.event.event_type` (e.g., `incident.triggered`)
- Delivery ID: `$.event.id`

**Event Catalog:**

| Event | Description | Common use case |
|-------|-------------|-----------------|
| `incident.triggered` | New incident | Auto-investigate |
| `incident.acknowledged` | Incident ack'd | Log |
| `incident.resolved` | Incident resolved | Post-mortem |
| `incident.escalated` | Escalation | Alert |

**Response Tools:**

| Tool | What it does |
|------|--------------|
| `pagerduty_acknowledge` | Acknowledge incident |
| `pagerduty_resolve` | Resolve incident |
| `pagerduty_add_note` | Add note to incident |
| `pagerduty_reassign` | Reassign incident |

#### Sentry

**Auth:**
- `sentry_webhook_signature` — verify `Sentry-Hook-Signature` (HMAC-SHA256)

**Event Catalog:**

| Event | Description |
|-------|-------------|
| `issue.created` | New error group |
| `issue.resolved` | Issue resolved |
| `issue.assigned` | Issue assigned |
| `event_alert.triggered` | Alert rule fired |
| `metric_alert.critical` | Metric alert critical |

**Response Tools:**

| Tool | What it does |
|------|--------------|
| `sentry_resolve_issue` | Resolve issue |
| `sentry_assign_issue` | Assign to user |
| `sentry_post_comment` | Comment on issue |

#### Generic Alerting (Prometheus/Alertmanager, Grafana, Datadog)

These share a common pattern: POST a JSON body with alert details. Use the
`generic` profile with a recommended system prompt suffix for alert triage.

Provide example recipes in the UI rather than full profiles.

### Profile Extensibility

Users should be able to:

1. Use a built-in profile as-is.
2. Use a built-in profile with event filter customization.
3. Use `generic` with a custom system prompt suffix for any unsupported provider.
4. (Phase 3) Register custom profiles via config.

### Source Profile Storage

Profiles are registered in code, not in the database. The webhook table stores
`source_profile` as a string key (e.g., `"github"`, `"stripe"`, `"generic"`)
and `source_config_json` for profile-specific settings (API tokens, base URLs
for self-hosted instances, etc.).

```rust
pub struct SourceConfig {
    /// Profile identifier.
    pub profile: String,
    /// Profile-specific settings (API token, base URL, etc.)
    /// Stored encrypted at rest.
    pub settings: serde_json::Value,
}
```

## Event Filtering

Not every event from a source matters. GitHub sends dozens of event types but a
code review webhook only cares about `pull_request.opened` and
`pull_request.synchronize`.

### Filter Configuration

Each webhook has an optional event filter:

```rust
pub struct EventFilter {
    /// If non-empty, only process events matching these types.
    /// Empty means accept all events.
    pub allow: Vec<String>,
    /// Events to always skip, applied after allow.
    pub deny: Vec<String>,
}
```

**Behavior:**

1. If `allow` is empty and `deny` is empty → accept all events.
2. If `allow` is non-empty → only accept events in the allow list.
3. `deny` is always applied last → explicitly skip specific events.
4. Filtered events get status `filtered` and are logged but not processed.
5. Filtered events do NOT count against rate limits.

### UI for Event Filtering

The UI shows the event catalog from the source profile as a checkbox list.
Users check the events they want. This is much better than asking users to
type `pull_request.opened` from memory.

For `generic` profiles, the filter is a free-form text list.

## Session Model

### Default

Use **one new persistent session per delivery**.

Reasons:

- Easier debugging
- Cleaner history
- Easy to resume manually from the UI
- Avoids one giant mixed thread from unrelated events

### Session Strategies

- `per_delivery` (default) — one session per accepted event
- `per_entity` — group by entity key (e.g., all events for PR #567 in one
  session). Entity key extracted by the source profile.
- `named_session` — all deliveries go to one named session

`per_entity` is the most useful advanced mode: it lets an agent maintain context
across the lifecycle of a PR, issue, or subscription. The source profile defines
how to extract the entity key:

```rust
fn entity_key(&self, event_type: &str, body: &serde_json::Value) -> Option<String>;
```

Examples:
- GitHub `pull_request.*` → `github:moltis-org/moltis:pr:567`
- Stripe `customer.subscription.*` → `stripe:sub_XYZ789`
- Linear `Issue.*` → `linear:ENG-123`

`named_session` is useful for intentionally accumulative workflows, but should
not be the default because it will become a garbage heap almost immediately.

### Session Labels

Suggested default label format:

- `{webhook name} #{sequence}` for `per_delivery`
- `{webhook name}: {entity_display}` for `per_entity`

Examples:

- `GitHub PR Hook #42`
- `GitHub PR Hook: #567 Add webhook support`
- `Stripe Payments #7 (checkout.session.completed)`

## Delivery Message Format

The inbound message injected into chat should be normalized and stable, not raw
verbatim headers pasted into the prompt in an ad hoc way.

### Layered Format

The delivery message has three layers:

1. **Envelope** — always present, same structure for all profiles.
2. **Normalized summary** — produced by the source profile's `normalize_payload`.
3. **Instructions** — webhook-specific system prompt suffix, if configured.

```text
Webhook delivery received.

Webhook: GitHub PR Hook (wh_xxxxx)
Source: github
Event: pull_request.opened
Delivery: 12345678-abcd-...
Received: 2026-04-06T12:34:56Z

---

GitHub event: pull_request.opened

Repository: moltis-org/moltis
PR #567: "Add webhook support"
  Author: @penso
  Branch: feature/webhooks → main
  URL: https://github.com/moltis-org/moltis/pull/567
  Labels: enhancement
  Draft: false

Description:
  This PR adds generic webhook support...

Changed files: 42 (+1,203 / -156)

---

Full JSON payload is available. Use it if you need details not in the summary
above.
```

The full payload is stored on the delivery record and available to the agent
via a tool (`webhook_get_full_payload`) rather than stuffed into the prompt.
This keeps token usage reasonable for large payloads.

### Rules

- Normalized summary produced by source profile, not raw JSON.
- For `generic` profile, fall back to pretty-printed JSON with truncation.
- Redact known secrets (tokens, passwords in URLs).
- Full payload stored separately, accessible via tool.
- Max prompt payload size: configurable, default 8 KB. Truncate with note.

## Response Actions

The key differentiator from a dumb webhook receiver: the agent can **act back**
on the source system.

### How It Works

1. Source profile defines response tools (e.g., `github_post_comment`).
2. When a webhook session starts, these tools are injected alongside the agent's
   regular tools.
3. The tools use credentials stored in the webhook's `source_config_json`.
4. Tool implementations call the source's API (GitHub REST, Stripe API, etc.).
5. The agent decides what actions to take based on the event and its instructions.

### Credential Storage

Source API credentials are stored in `source_config_json` on the webhook record,
encrypted at rest using the existing `secrecy::Secret<String>` pattern.

Example for GitHub:

```json
{
  "api_token": "ghp_xxxxxxxxxxxxxxxxxxxx",
  "api_url": "https://api.github.com"
}
```

For self-hosted instances (GitLab, GitHub Enterprise):

```json
{
  "api_token": "glpat-xxxxxxxxxxxx",
  "api_url": "https://gitlab.company.com/api/v4"
}
```

### Tool Policy Integration

Response tools respect the webhook's tool policy:

- If the webhook has a `tool_deny` list that includes `github_merge_pr`, the
  agent cannot merge PRs even though the profile provides that tool.
- This gives users fine-grained control: "review PRs and comment, but never
  merge."

## Auth and Source Verification

### Supported Modes for V1

- `none`
  - Only acceptable for local/testing. UI should warn loudly.
- `static_header`
  - User configures header name and expected value.
- `bearer`
  - Expect `Authorization: Bearer <token>`.
- `github_hmac_sha256`
  - Verify `X-Hub-Signature-256` against configured secret.
- `gitlab_token`
  - Verify `X-Gitlab-Token` against configured secret.
- `stripe_webhook_signature`
  - Verify `Stripe-Signature` header with timestamp + HMAC-SHA256.
- `linear_webhook_signature`
  - Verify `Linear-Signature` header.
- `pagerduty_v2_signature`
  - Verify `X-PagerDuty-Signature` header.
- `sentry_webhook_signature`
  - Verify `Sentry-Hook-Signature` header.
- `cidr_allowlist`
  - Optional additional policy layered on top of auth mode.

### Verification Pipeline

```
Request arrives
  → Parse public_id from URL
  → Load webhook config (404 if not found or disabled)
  → Check CIDR allowlist (403 if blocked)
  → Check rate limit (429 if exceeded)
  → Read body bytes (413 if too large)
  → Profile.verify(auth_config, headers, body) → 401/403 on failure
  → Profile.parse_event_type(headers, body)
  → Check event filter → log as `filtered` if not allowed
  → Profile.parse_delivery_key(headers, body)
  → Dedup check → log as `deduplicated` if seen before
  → Persist delivery record as `received`
  → Queue for async processing
  → Return 202 Accepted
```

## Security Requirements

- Public webhook ID must be random and high entropy, not sequential.
- ID is not authentication, only routing.
- Authentication should use headers by default, not query params.
- Support query-based auth only as an advanced escape hatch if added later.
- Enforce request body size limits (default 1 MB, configurable per webhook).
- Enforce request timeout and fast acknowledgment behavior.
- Persist rejection reasons for observability where safe.
- Redact secrets in logs and UI.
- Deduplicate deliveries.
- Add rate limiting:
  - per webhook (default: 60/minute)
  - global (default: 300/minute)
- Support CIDR allowlists as an additional guardrail.
- Validate content types but do not reject valid JSON just because the sender
  forgot the exact perfect header.
- Source API credentials encrypted at rest.
- Response tool credentials scoped per-webhook, never shared globally.
- Log all response tool invocations for audit trail.

## Data Model

Add a dedicated webhook store, not a config-file-only feature.

New crate: `moltis-webhooks` with its own `migrations/` directory.

### `webhooks`

| Column | Type | Notes |
|--------|------|-------|
| `id` | INTEGER PK | |
| `name` | TEXT NOT NULL | |
| `description` | TEXT | |
| `enabled` | BOOLEAN DEFAULT TRUE | |
| `public_id` | TEXT NOT NULL UNIQUE | High-entropy, e.g. `wh_` + 24 random chars |
| `agent_id` | TEXT | References agent preset |
| `model` | TEXT | Override |
| `system_prompt_suffix` | TEXT | |
| `tool_policy_json` | TEXT | `{ "allow": [...], "deny": [...] }` |
| `sandbox_policy_json` | TEXT | |
| `auth_mode` | TEXT NOT NULL | Enum string |
| `auth_config_json` | TEXT | Secrets, encrypted at rest |
| `source_profile` | TEXT NOT NULL DEFAULT 'generic' | |
| `source_config_json` | TEXT | API tokens, base URLs; encrypted at rest |
| `event_filter_json` | TEXT | `{ "allow": [...], "deny": [...] }` |
| `session_mode` | TEXT NOT NULL DEFAULT 'per_delivery' | |
| `named_session_key` | TEXT | For `named_session` mode |
| `entity_key_template` | TEXT | For `per_entity` mode (profile may override) |
| `allowed_cidrs_json` | TEXT | |
| `max_body_bytes` | INTEGER DEFAULT 1048576 | 1 MB |
| `rate_limit_per_minute` | INTEGER DEFAULT 60 | |
| `delivery_count` | INTEGER DEFAULT 0 | Denormalized for UI |
| `last_delivery_at` | TEXT | Denormalized for UI |
| `created_at` | TEXT NOT NULL | |
| `updated_at` | TEXT NOT NULL | |

### `webhook_deliveries`

| Column | Type | Notes |
|--------|------|-------|
| `id` | INTEGER PK | |
| `webhook_id` | INTEGER NOT NULL | FK → webhooks.id |
| `received_at` | TEXT NOT NULL | |
| `status` | TEXT NOT NULL | |
| `event_type` | TEXT | |
| `entity_key` | TEXT | For session grouping |
| `delivery_key` | TEXT | Idempotency key |
| `http_method` | TEXT | |
| `content_type` | TEXT | |
| `remote_ip` | TEXT | |
| `headers_json` | TEXT | Selected headers only |
| `body_size` | INTEGER | |
| `body_blob` | BLOB | For payloads ≤ 256 KB |
| `body_storage_path` | TEXT | For payloads > 256 KB |
| `session_key` | TEXT | Links to sessions table |
| `rejection_reason` | TEXT | |
| `run_error` | TEXT | |
| `started_at` | TEXT | |
| `finished_at` | TEXT | |
| `duration_ms` | INTEGER | |
| `input_tokens` | INTEGER | |
| `output_tokens` | INTEGER | |

### `webhook_response_actions`

| Column | Type | Notes |
|--------|------|-------|
| `id` | INTEGER PK | |
| `delivery_id` | INTEGER NOT NULL | FK → webhook_deliveries.id |
| `tool_name` | TEXT NOT NULL | e.g., `github_post_comment` |
| `input_json` | TEXT | Tool input parameters |
| `output_json` | TEXT | Tool output |
| `status` | TEXT NOT NULL | `success`, `error` |
| `error_message` | TEXT | |
| `created_at` | TEXT NOT NULL | |

This audit table is important for users to understand what the agent did in
response to each delivery. Visible in the delivery inspector.

### Status Values

- `received` — persisted, not yet queued
- `filtered` — event type not in allow list
- `deduplicated` — duplicate delivery key
- `rejected` — auth failure or policy violation
- `queued` — waiting for worker
- `processing` — agent running
- `completed` — agent finished successfully
- `failed` — agent errored

### Indexes

```sql
CREATE INDEX idx_webhook_deliveries_webhook_id ON webhook_deliveries(webhook_id);
CREATE INDEX idx_webhook_deliveries_status ON webhook_deliveries(status);
CREATE INDEX idx_webhook_deliveries_delivery_key ON webhook_deliveries(delivery_key);
CREATE INDEX idx_webhook_deliveries_entity_key ON webhook_deliveries(entity_key);
CREATE INDEX idx_webhook_deliveries_received_at ON webhook_deliveries(received_at);
CREATE INDEX idx_webhook_response_actions_delivery ON webhook_response_actions(delivery_id);
```

## Execution Config

The webhook should point at an existing `agent_id`, then layer overrides.

### Why

Moltis already has:

- persona workspaces
- per-session agent binding
- prompt assembly from agent files

Using `agent_id` preserves:

- identity
- soul
- workspace instructions
- memory boundaries

Webhook-local overrides should be intentionally narrow:

- `model`
- `system_prompt_suffix`
- `tool allow/deny`
- `sandbox policy`

### Important Implementation Detail

Today, prompt persona and runtime tool filtering are loaded from the effective
agent config during chat execution. We should make webhook overrides flow into
that same runtime path instead of inventing a separate ad hoc executor.

This likely requires adding a notion of **session-scoped execution overrides**
or **webhook execution profile** so webhook sessions can:

- keep agent persona files
- apply tool restrictions
- apply model override
- append extra prompt text
- inject source-profile response tools

without mutating the base agent permanently.

### Tool Injection

The webhook session's tool list is assembled as:

1. Start with agent preset's tool policy (allow/deny).
2. Apply webhook-level tool policy overrides.
3. Add source profile response tools.
4. Add `webhook_get_full_payload` tool (always available in webhook sessions).

## HTTP API

### Public Ingress

- `POST /api/webhooks/ingest/{public_id}`

Separate from admin namespace (`/api/webhooks/...`) to avoid confusion and allow
different auth middleware. The `ingest` prefix makes it clear this is the
public-facing endpoint, not an admin CRUD route.

Behavior:

- Accept raw body bytes and headers
- Verify according to webhook config
- Persist delivery record
- Queue async execution
- Return fast acknowledgment

Response suggestions:

- `202 Accepted` on success (body: `{ "delivery_id": "...", "status": "queued" }`)
- `401` or `403` on auth failure
- `404` for unknown or disabled webhook
- `200` for duplicates with `{ "delivery_id": "...", "status": "deduplicated" }`
- `413` for payload too large
- `429` for rate limiting

### Admin / UI RPC

Add CRUD and listing methods similar to existing settings surfaces:

- `webhooks.list` — list all webhooks with summary stats
- `webhooks.get` — full webhook config (secrets redacted)
- `webhooks.create` — create new webhook, returns config with `public_id`
- `webhooks.update` — update webhook config
- `webhooks.delete` — delete webhook and all deliveries
- `webhooks.test` — send a test delivery (source profile provides sample payload)
- `webhooks.deliveries` — list deliveries for a webhook (paginated, filterable)
- `webhooks.delivery.get` — single delivery with full metadata
- `webhooks.delivery.payload` — raw payload download
- `webhooks.delivery.actions` — response actions for a delivery
- `webhooks.delivery.retry` — re-process a delivery
- `webhooks.profiles` — list available source profiles with metadata

## Background Processing

Processing must not happen inline in the HTTP request path.

Reason:

- GitHub recommends quick responses
- GitLab retries if receivers are slow
- Stripe has a 20-second timeout
- LLM work is variable and can be long-running

### Worker Architecture

```
HTTP Handler                     Worker Pool
    │                                │
    ├── validate + persist ──────►   │
    │   delivery record              │
    ├── send delivery_id ──────────► │
    │   via tokio::mpsc              │
    ├── return 202 ──►               │
    │                                ├── load webhook config
                                     ├── load delivery body
                                     ├── resolve session key
                                     ├── create/load session
                                     ├── assign agent
                                     ├── inject normalized message
                                     ├── inject response tools
                                     ├── chat.send_sync(params)
                                     ├── record token usage
                                     ├── update delivery status
                                     └── update delivery_count
```

Worker pool is a fixed-size `tokio::mpsc` channel with configurable concurrency
(default: 4 concurrent webhook executions). This prevents a burst of webhook
deliveries from overwhelming the LLM provider.

### Crash Recovery

On startup, scan for deliveries with status `received` or `queued` and re-queue
them. This makes the system durable enough that accepted deliveries are not
silently dropped on restart.

### Concurrency Control

For `per_entity` session mode, deliveries for the same entity key should be
serialized (process in order). Use a per-entity lock (keyed semaphore) to
prevent concurrent writes to the same session.

## UI Plan

Webhooks live exclusively in **Settings → Webhooks**. They are not part of the
onboarding flow, not surfaced in the main chat sidebar, and not promoted in
first-run experience. Users navigate to settings when they need this feature.

No webhook-related UI should appear outside the settings page unless the user
has at least one webhook configured (at which point a small badge/count in the
settings nav is fine).

### Main View (`Settings → Webhooks`)

- Webhook cards (not a table — cards show more context at a glance)
- Source profile icon/badge (GitHub octocat, GitLab fox, Stripe, etc.)
- Status badge (active / paused / error)
- Target agent name
- Last delivery time (relative, e.g., "3 minutes ago")
- Recent delivery sparkline or status dots (last 10 deliveries: green/red/gray)
- Delivery count
- Enabled toggle (inline, immediate effect)
- Quick actions per card: **Edit**, **Delete**, copy endpoint URL, view deliveries

Empty state: simple "No webhooks configured" message with a "Create webhook"
button. No tutorial, no onboarding wizard. A link to docs is enough.

### Create Flow

Guided wizard instead of a single modal (too many fields):

**Step 1: Source**
- Pick source profile from cards (GitHub, GitLab, Stripe, Linear, Generic, ...)
- Each card shows icon, name, one-line description
- Selecting a profile prefills auth mode and shows setup guide

**Step 2: Authentication**
- Auth mode (prefilled from profile, editable)
- Secret/token input with generate button
- Optional CIDR allowlist
- Setup instructions specific to the selected profile (with links, screenshots)

**Step 3: Events**
- Event filter checkboxes from profile's event catalog
- "Select all" / "Select none"
- Event descriptions visible
- For generic profile: free-form event name list

**Step 4: Agent & Execution**
- Target agent (selection cards, like model selection)
- Model override (optional)
- System prompt suffix (textarea, with profile-specific suggestions)
- Tool policy overrides
- Session mode (per_delivery / per_entity / named_session)
- Sandbox override

**Step 5: Response Actions** (only for profiles that have response tools)
- API credential input (PAT, token, etc.)
- Which response tools to enable (checkboxes from profile's tool list)
- Permission scope guidance

**Step 6: Review & Create**
- Summary of all settings
- Generated endpoint URL with copy button
- "Create & Test" button that creates the webhook and sends a sample payload

### Edit View

Clicking **Edit** on a webhook card opens a tabbed editor (not the wizard —
the wizard is for initial creation only):

- **General** — name, description, enabled, source profile (read-only after
  creation, changing profile is a delete-and-recreate)
- **Authentication** — auth mode, secret rotation (generate new secret without
  losing the webhook), CIDR allowlist
- **Events** — event filter checkboxes, same as create step 3
- **Execution** — agent, model, prompt suffix, tool policy, session mode, sandbox
- **Response Actions** — API credentials, enabled tools

Each tab saves independently. Show the public endpoint URL at the top of the
edit view with a copy button, always visible.

Secret rotation UX: "Rotate secret" button generates a new secret and shows it
once. Old secret continues working for a configurable grace period (default: 1
hour) to allow the sender to be updated without downtime.

### Delete Flow

Clicking **Delete** on a webhook card shows a confirmation dialog:

- Webhook name prominently displayed
- Warning: "This will permanently delete the webhook and all N delivery records."
- Deliveries that created chat sessions are NOT deleted — sessions persist
  independently. The dialog should say this clearly.
- Require typing the webhook name to confirm (prevents accidental deletion of
  the wrong webhook when managing many).
- Delete calls `webhooks.delete` RPC which cascades to `webhook_deliveries` and
  `webhook_response_actions`.

### Disable vs Delete

The enabled toggle on the card is the "soft" off switch — the webhook endpoint
returns `404` but config and history are preserved. Delete is permanent.

Make this distinction clear: the toggle tooltip should say "Pause webhook
(keeps configuration and history)" and the delete button should say "Delete
permanently."

### Delivery Inspector

For each webhook:

- Recent deliveries list with filters (status, event type, date range)
- Each row: status badge, event type, received time, duration, session link
- Pagination

Per delivery:

- Normalized metadata (envelope)
- Normalized summary (from source profile)
- Selected headers
- Body preview (collapsible, syntax-highlighted JSON)
- Response actions table (tool name, status, timestamp)
- Session link (click to open chat session)
- Raw body download button
- Retry button

### Dashboard (Phase 2)

Add a webhook health dashboard:
- Delivery volume chart (last 24h / 7d / 30d)
- Success/failure rate
- Average processing duration
- Error breakdown

### Example Recipes

The create flow should include starter recipes:

- **GitHub PR Reviewer** — review PRs, post comments with feedback
- **GitHub Issue Triager** — label and assign new issues
- **GitLab MR Reviewer** — review merge requests
- **Stripe Payment Handler** — process successful payments, handle failures
- **PagerDuty Investigator** — auto-investigate incidents, gather context
- **Sentry Error Analyzer** — analyze new error groups, suggest fixes
- **Generic Alert Handler** — triage and summarize alerts from any source

Each recipe prefills: source profile, event filter, system prompt suffix, and
suggested agent instructions.

## Recommended UX Defaults

- default source profile: `Generic`
- default auth mode: `static_header` (or profile-specific if selected)
- default session mode: `per_delivery`
- default target agent: current default agent
- default body limit: 1 MB
- default response mode: async `202`
- default rate limit: 60/minute
- default tool policy: inherit agent
- default sandbox policy: inherit agent/session defaults

## Observability

### Metrics

All gated behind `#[cfg(feature = "metrics")]`:

- `webhooks_deliveries_total` (labels: webhook_id, status, event_type)
- `webhooks_deliveries_rejected_total` (labels: webhook_id, reason)
- `webhooks_deliveries_filtered_total` (labels: webhook_id, event_type)
- `webhooks_deliveries_deduplicated_total` (labels: webhook_id)
- `webhooks_processing_duration_seconds` (histogram, labels: webhook_id)
- `webhooks_response_actions_total` (labels: webhook_id, tool_name, status)
- `webhooks_rate_limited_total` (labels: webhook_id)
- `webhooks_worker_queue_depth` (gauge)
- `webhooks_active_executions` (gauge)

### Tracing

All webhook processing spans instrumented with `#[tracing::instrument]`:
- `webhook_ingest` — HTTP handler span
- `webhook_verify` — auth verification
- `webhook_process` — background worker execution
- `webhook_response_action` — individual response tool calls

### Delivery Logs

Delivery logs stored in `webhook_deliveries` and `webhook_response_actions`
tables, not in text log files. This makes them queryable and displayable in
the UI.

Never log secrets (auth tokens, signing secrets, API keys). Log rejection
reasons for observability.

## Testing Plan

### Unit Tests

- Auth verification per mode (HMAC, token, bearer, Stripe sig, etc.)
- Dedup behavior
- Event filter logic (allow/deny combinations)
- Payload normalization per source profile
- Entity key extraction per source profile
- Session label generation
- Config validation
- CIDR matching
- Rate limit logic

### Integration Tests

- POST accepted delivery creates session
- Accepted delivery runs `chat.send_sync`
- Rejected auth does not create session
- Duplicate delivery does not double-run
- Filtered event logged but not processed
- Model override is applied
- Agent binding is applied
- Tool restrictions are applied
- Response tools injected and callable
- Response actions logged to audit table
- `per_entity` mode groups deliveries into same session
- Crash recovery re-queues unprocessed deliveries
- Rate limiting returns 429

### Source Profile Tests

Per profile:
- `verify()` with valid and invalid signatures
- `parse_event_type()` with real payload samples
- `parse_delivery_key()` extracts correct key
- `normalize_payload()` produces expected summary
- `entity_key()` extracts correct key
- Response tools make correct API calls (mocked HTTP)

### Web UI / E2E

- Create webhook via wizard (each profile)
- Edit webhook settings
- Delete webhook
- Send test delivery
- Inspect deliveries list
- View delivery detail with response actions
- Follow delivery to chat session
- Event filter checkboxes work
- Endpoint URL copy works

## Migration / Rollout

### Phase 1: Foundation

- `moltis-webhooks` crate with store, types, migrations
- `SourceProfile` trait and `generic` profile implementation
- Public ingress endpoint with verification pipeline
- Background worker with `tokio::mpsc` queue
- Session creation following cron pattern
- Delivery persistence and status tracking
- `webhook_get_full_payload` tool
- Basic admin RPC (CRUD + deliveries)
- UI: create/edit/delete/list webhooks, delivery list
- Auth modes: `none`, `static_header`, `bearer`, `github_hmac_sha256`
- Tests for all of the above

### Phase 2: Major Source Profiles

- GitHub source profile with full event catalog + normalization + response tools
- GitLab source profile
- Stripe source profile
- Event filtering UI with profile-aware checkboxes
- `per_entity` session mode
- Entity key extraction per profile
- Response actions audit table
- Delivery inspector with response action detail
- CIDR allowlists
- Rate limiting (per-webhook and global)
- Crash recovery on startup
- Concurrency control for `per_entity` mode
- UI wizard with guided setup per profile
- Starter recipes
- Test delivery from UI

### Phase 3: More Providers & Polish

- Linear, Jira, PagerDuty, Sentry source profiles
- GitHub App installation support (alternative to PAT)
- Delivery retry from UI
- Webhook health dashboard
- Delivery volume charts
- Custom source profile registration (config-driven)
- Webhook-to-webhook chaining (output of one triggers another)
- Conditional execution (only run agent if payload matches JSONPath expression)
- Delivery payload transformation templates

## Crate Structure

```
crates/webhooks/
├── Cargo.toml
├── migrations/
│   └── 20260407000000_initial.sql
└── src/
    ├── lib.rs              # re-exports, run_migrations()
    ├── types.rs            # Webhook, WebhookDelivery, EventFilter, etc.
    ├── store.rs            # WebhookStore trait + SQLite impl
    ├── profile.rs          # SourceProfile trait
    ├── profiles/
    │   ├── mod.rs          # profile registry
    │   ├── generic.rs
    │   ├── github.rs
    │   ├── gitlab.rs
    │   ├── stripe.rs
    │   ├── linear.rs
    │   ├── pagerduty.rs
    │   └── sentry.rs
    ├── auth.rs             # AuthMode enum, verify implementations
    ├── normalize.rs        # NormalizedPayload, shared normalization helpers
    ├── filter.rs           # EventFilter logic
    ├── dedup.rs            # Deduplication
    ├── rate_limit.rs       # Per-webhook + global rate limiting
    ├── worker.rs           # Background processing worker pool
    └── tools/
        ├── mod.rs          # Response tool trait + registry
        ├── payload.rs      # webhook_get_full_payload tool
        ├── github.rs       # GitHub response tools
        ├── gitlab.rs       # GitLab response tools
        ├── stripe.rs       # Stripe response tools
        ├── linear.rs       # Linear response tools
        └── pagerduty.rs    # PagerDuty response tools
```

Gateway integration:
```
crates/gateway/src/
├── webhooks.rs             # LiveWebhooksService, RPC handlers
├── webhook_routes.rs       # Public ingress route, admin routes
```

Web UI:
```
crates/web/src/assets/js/
├── page-webhooks.js        # Webhooks settings page
├── components/
│   └── webhook-wizard.js   # Create/edit wizard component
```

## Open Questions

- Should webhook sessions be tagged as a new session kind, for example
  `webhook`, in runtime context? **Leaning yes** — helps filter in UI and
  prevents webhook sessions from cluttering normal chat history.
- Should we store full payloads in SQLite blobs or on disk with references?
  **Recommendation:** blobs for payloads ≤ 256 KB (vast majority), disk
  files for larger ones. Keeps queries fast for common case.
- Do we want delivery replay from the UI in v1 or wait?
  **Recommendation:** Phase 2. Need stable delivery model first.
- Should response tools be hard-coded per profile or user-configurable?
  **Recommendation:** hard-coded per profile in v1, user-configurable in v3.
- How do we handle GitHub App installations vs PATs?
  **Recommendation:** PATs in Phase 2, App installations in Phase 3.
- Should `per_entity` sessions have a TTL or max size?
  **Recommendation:** yes — default 7 days or 100 messages, whichever first.
  Prevents unbounded session growth for long-lived entities.

## Recommendation

Build this as a **new top-level feature** using the existing agent/session/chat
primitives. The key design constraint is to avoid creating a second independent
agent configuration system.

The clean path is:

- webhook targets an existing `agent_id`
- webhook adds narrow execution overrides
- source profiles provide auth + normalization + response tools
- each accepted delivery becomes a normal persistent chat session
- response tools let the agent act back on the source system

Source profiles are the key abstraction. They transform raw HTTP deliveries into
structured agent context and give the agent typed tools to respond. Without
profiles, users have to write custom system prompts to parse raw JSON and use
generic HTTP tools to respond — that works but it's fragile and hard to set up.

Profiles make the common case easy (GitHub PR review, Stripe payment handling)
while the generic profile keeps the escape hatch open for anything unsupported.
