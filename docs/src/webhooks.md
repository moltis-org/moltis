# Webhooks

Moltis can receive inbound HTTP webhooks from external services and run AI
agents in response. Each webhook delivery becomes a persistent chat session
that can be inspected and continued from the web UI.

Use webhooks to trigger agents from GitHub PRs, GitLab merge requests, Stripe
payments, PagerDuty incidents, or any service that can POST JSON to a URL.

## How It Works

```
External Service (GitHub, Stripe, …)
        │
        ▼  POST /api/webhooks/ingest/{public_id}
┌──────────────────────────────────┐
│         Ingress Handler          │
│  verify → filter → dedup → 202  │
└──────────────┬───────────────────┘
               │  delivery_id
               ▼
┌──────────────────────────────────┐
│       Background Worker          │
│  normalize → create session      │
│  → inject message → chat.send   │
└──────────────┬───────────────────┘
               │
               ▼
┌──────────────────────────────────┐
│       Persistent Session         │
│  Agent processes event,          │
│  optionally acts back via tools  │
└──────────────────────────────────┘
```

1. The external service POSTs to the webhook's public endpoint.
2. Moltis verifies authentication, checks the event filter, and deduplicates.
3. The request is acknowledged with `202 Accepted` immediately.
4. A background worker normalizes the payload, creates a chat session, and runs
   the bound agent.
5. The resulting session is visible in the web UI like any other conversation.

## Setup

Webhooks can be created from the **web UI**, the **CLI**, or by the **agent** itself
using the `webhook` tool. They are not part of the onboarding flow.

### Creating a Webhook (Web UI)

1. Navigate to **Settings → Webhooks** and click **Create webhook**.
2. Choose a **source profile** (GitHub, GitLab, Stripe, or Generic).
3. Configure **authentication** — the profile pre-selects a recommended mode.
4. Optionally filter which **event types** to process.
5. Select a **target agent** and optional model override.
6. Click **Create** — the endpoint URL is displayed with a copy button.
7. Register this URL in the external service's webhook settings.

### Creating a Webhook (CLI)

```bash
moltis webhooks create \
  --name github-pr-review \
  --source-profile github \
  --auth-mode github_hmac_sha256 \
  --events "pull_request.opened,pull_request.synchronize" \
  --system-prompt "Review this PR for security issues"
```

For a zero-cost event forwarder (no LLM tokens):

```bash
moltis webhooks create \
  --name stripe-payments \
  --source-profile stripe \
  --auth-mode stripe_webhook_signature \
  --deliver-only \
  --prompt-template "Payment: {data.object.amount} {data.object.currency} from {data.object.customer_email}" \
  --deliver-to telegram \
  --events "payment_intent.succeeded"
```

### Creating a Webhook (Agent Tool)

The agent can create webhooks programmatically using the `webhook` tool:

> "Set up a webhook for GitHub issues on my repo and forward them to my Slack channel"

The agent will use the `webhook` tool with `action: "create"` to set up the endpoint,
then tell you the URL to register in GitHub's settings.

### Endpoint URL

Each webhook gets a stable, high-entropy public URL:

```
https://your-moltis-host/api/webhooks/ingest/wh_a1b2c3d4e5f6...
```

The `wh_` prefix followed by 36 random hex characters serves as a routing
identifier — it is **not** authentication. Authentication is handled by the
configured auth mode.

## Source Profiles

Source profiles define how to authenticate, parse, and normalize events from a
specific provider. Selecting a profile pre-fills the recommended auth mode and
provides an event catalog for filtering.

| Profile | Auth Mode | Event Parsing | Entity Grouping |
|---------|-----------|---------------|-----------------|
| Generic | Static header | Configurable header | None |
| GitHub | HMAC-SHA256 (`X-Hub-Signature-256`) | `X-GitHub-Event` + action | PR number, issue number |
| GitLab | Token (`X-Gitlab-Token`) | `X-Gitlab-Event` + action | MR iid, issue iid |
| Stripe | Webhook signature (`Stripe-Signature`) | `$.type` in body | Subscription ID |

### GitHub

GitHub webhooks use HMAC-SHA256 signature verification. When you create a
webhook with the GitHub profile:

1. Moltis generates a random secret (or you provide one).
2. In your GitHub repo, go to **Settings → Webhooks → Add webhook**.
3. Set the payload URL to your Moltis webhook endpoint.
4. Set content type to `application/json`.
5. Paste the secret.
6. Select the events you want to trigger (or choose "Send me everything" and
   filter in Moltis).

**Event types:**

| Event | Description | Use case |
|-------|-------------|----------|
| `pull_request.opened` | New PR | Code review, labeling |
| `pull_request.synchronize` | PR updated | Re-review |
| `pull_request.closed` | PR closed/merged | Cleanup, changelog |
| `push` | Commits pushed | CI trigger, deploy check |
| `issues.opened` | New issue | Triage, auto-respond |
| `issue_comment.created` | Comment on issue/PR | Answer questions |
| `pull_request_review.submitted` | PR review posted | Respond to feedback |
| `release.published` | New release | Announce, post-release tasks |
| `workflow_run.completed` | Actions workflow done | Post-CI analysis |

**Payload normalization** extracts key fields (repo, PR number, author, branch,
description, changed files) instead of dumping the full payload into the agent
prompt — keeping token usage reasonable.

### GitLab

GitLab webhooks use a static token in the `X-Gitlab-Token` header.

1. In your GitLab project, go to **Settings → Webhooks**.
2. Set the URL to your Moltis webhook endpoint.
3. Paste the secret token.
4. Select trigger events.

**Event types:**

| Event | Description |
|-------|-------------|
| `merge_request.open` | New merge request |
| `merge_request.update` | MR updated |
| `merge_request.merge` | MR merged |
| `push` | Commits pushed |
| `note` | Comment on MR or issue |
| `issue.open` | New issue |
| `pipeline` | Pipeline status change |

### Stripe

Stripe webhooks use a composite signature in the `Stripe-Signature` header
with timestamp validation (5-minute tolerance).

1. In the Stripe Dashboard, go to **Developers → Webhooks → Add endpoint**.
2. Set the endpoint URL to your Moltis webhook endpoint.
3. Select events to listen to.
4. Copy the signing secret (`whsec_...`) into Moltis.

**Event types:**

| Event | Description |
|-------|-------------|
| `checkout.session.completed` | Successful checkout |
| `payment_intent.succeeded` | Payment captured |
| `payment_intent.payment_failed` | Payment failed |
| `invoice.paid` | Invoice paid |
| `customer.subscription.created` | New subscription |
| `customer.subscription.deleted` | Subscription canceled |
| `charge.dispute.created` | Chargeback opened |

### Generic

The generic profile works with any service that can POST JSON. Configure a
static header or bearer token for authentication. Event type is extracted from
common headers (`X-Event-Type`, `X-Webhook-Event`) if present.

## Authentication

Each webhook is configured with an auth mode that verifies inbound requests.

| Mode | Header | Verification |
|------|--------|-------------|
| `none` | — | No verification (testing only) |
| `static_header` | Configurable | Constant-time comparison of header value |
| `bearer` | `Authorization` | `Bearer <token>` comparison |
| `github_hmac_sha256` | `X-Hub-Signature-256` | HMAC-SHA256 of body against shared secret |
| `gitlab_token` | `X-Gitlab-Token` | Constant-time token comparison |
| `stripe_webhook_signature` | `Stripe-Signature` | HMAC-SHA256 with timestamp tolerance |
| `linear_webhook_signature` | `Linear-Signature` | HMAC-SHA256 |
| `pagerduty_v2_signature` | `X-PagerDuty-Signature` | HMAC-SHA256 |
| `sentry_webhook_signature` | `Sentry-Hook-Signature` | HMAC-SHA256 |

```admonish warning title="Auth Mode: None"
The `none` auth mode accepts all requests without verification. Use it only for
local testing. The UI displays a warning when this mode is selected.
```

All secret comparisons use constant-time operations to prevent timing attacks.

## Event Filtering

Each webhook can filter which event types to process using allow and deny lists.

- **Allow list empty, deny list empty** — accept all events.
- **Allow list non-empty** — only accept events in the list.
- **Deny list** — always applied last, explicitly skips matching events.

Filtered events are logged with status `filtered` but not processed. They do
not count against rate limits.

When using a source profile, the UI shows the event catalog as checkboxes
instead of requiring free-form text.

## Session Modes

Each delivery creates a chat session. The session mode controls how sessions
are organized.

| Mode | Behaviour |
|------|-----------|
| `per_delivery` (default) | One new session per delivery. Best for debugging and clean history. |
| `per_entity` | Group deliveries by entity key (e.g., all events for PR #567 in one session). Useful for maintaining context across an entity's lifecycle. |
| `named_session` | All deliveries go to one named session. Use sparingly — can become noisy. |

### Entity Keys

In `per_entity` mode, the source profile extracts a grouping key from the
payload:

| Profile | Entity Key Format |
|---------|-------------------|
| GitHub | `github:{repo}:pr:{number}` or `github:{repo}:issue:{number}` |
| GitLab | `gitlab:{project}:mr:{iid}` or `gitlab:{project}:issue:{iid}` |
| Stripe | `stripe:{subscription_id}` or `stripe:dispute:{charge_id}` |
| Generic | None (falls back to `per_delivery`) |

### Session Labels

Sessions are labeled for easy identification in the sidebar:

- **per_delivery**: `webhook:{public_id}:{delivery_id}`
- **per_entity**: `webhook:{public_id}:{entity_key}`
- **named_session**: configured key or `webhook:{public_id}`

## Agent Execution

Each webhook is bound to an agent preset. When a delivery is processed:

1. The worker creates a session with the webhook's session key.
2. The configured agent is assigned to the session.
3. A normalized message describing the event is injected.
4. `chat.send_sync` runs the agent turn.
5. The delivery record is updated with status, duration, and token counts.

### Execution Overrides

Webhooks can override specific agent settings without changing the base preset:

- **Model** — use a different LLM for webhook processing.
- **System prompt suffix** — append extra instructions (e.g., "Focus on security
  issues" for a code review webhook).
- **Tool policy** — restrict which tools the agent can use.

### Delivery Message Format

The agent receives a structured message with three layers:

```
Webhook delivery received.

Webhook: GitHub PR Hook (wh_xxxxx)
Source: github
Event: pull_request.opened
Delivery: abc-123-def
Received: 2026-04-07T12:34:56Z

---

GitHub event: pull_request.opened

Repository: moltis-org/moltis
PR #567: "Add webhook support"
  Author: @penso
  Branch: feature/webhooks → main
  URL: https://github.com/moltis-org/moltis/pull/567
  Draft: false

Description:
  This PR adds generic webhook support...

Changed files: 42 (+1,203 / -156)

---

Full payload available via webhook_get_full_payload tool.
```

The full raw payload is stored on the delivery record and available to the
agent via the `webhook_get_full_payload` tool, keeping prompt token usage
manageable for large payloads.

## Delivery Lifecycle

Each delivery goes through a status progression:

| Status | Description |
|--------|-------------|
| `received` | Persisted, not yet queued |
| `filtered` | Event type not in allow list |
| `deduplicated` | Duplicate delivery key |
| `rejected` | Auth failure or policy violation |
| `queued` | Waiting for worker |
| `processing` | Agent running |
| `completed` | Agent finished successfully |
| `failed` | Agent errored |

### Deduplication

Deliveries are deduplicated by a provider-specific key:

- **GitHub**: `X-GitHub-Delivery` header
- **GitLab**: `Idempotency-Key` header (falls back to body hash)
- **Stripe**: `$.id` field in body
- **Generic**: `X-Delivery-Id` or `X-Request-Id` header (falls back to body
  SHA-256 hash)

Duplicate deliveries are logged with status `deduplicated` and return `200 OK`.

## Rate Limiting

Two levels of rate limiting protect against abuse:

| Level | Default | Description |
|-------|---------|-------------|
| Per-webhook | 60/minute | Configurable per webhook |
| Global | 300/minute | Across all webhooks |

Rate-limited requests receive `429 Too Many Requests`. Filtered and
deduplicated events do not count against rate limits.

## Security

- **Public IDs are routing identifiers, not secrets.** Authentication is
  handled by the configured auth mode.
- **Secrets use constant-time comparison** to prevent timing attacks.
- **Request body size is limited** (default: 1 MB, configurable per webhook).
- **Auth headers are never logged.** Only safe headers (event type, delivery
  ID, content type) are persisted.
- **Webhook secrets and source API credentials** are encrypted at rest when
  Vault is enabled.

```admonish warning title="Secret Management"
Without Vault, webhook secrets and API tokens remain plaintext in the SQLite
database. Enable Moltis [Vault](vault.md) if these secrets are going to live on
disk. Rotate secrets periodically.
```

## Delivery Inspector

The web UI provides a delivery inspector for each webhook:

- **Deliveries list** with status, event type, timestamp, and duration.
- **Per-delivery detail** with normalized metadata, headers, body preview, and
  session link.
- **Response actions** (when using profiles with response tools) showing what
  the agent did.
- Click a delivery's session link to open the full chat conversation.

## Editing and Deleting

### Editing

Click **Edit** on a webhook card to modify its settings. Changes take effect
immediately for new deliveries. In-progress deliveries use the configuration
that was active when they were received.

### Disabling

The toggle on each webhook card pauses it — the endpoint returns `404` but
configuration and delivery history are preserved.

### Deleting

Deleting a webhook permanently removes it and all delivery records. Chat
sessions created by deliveries are **not** deleted — they persist independently
as normal sessions.

## Crash Recovery

On startup, Moltis scans for deliveries with status `received` or `queued` and
re-queues them for processing. Accepted deliveries are not silently dropped on
restart.

## Testing Webhooks

Use [Hoppscotch](https://hoppscotch.io) (free, open source, no signup) to test
your webhooks. Set the method to POST, paste your webhook endpoint URL, add a
JSON body, and set any required auth headers.

Alternatively, use the included test script:

```bash
./scripts/test-webhook.sh <webhook-url> --profile github --secret <your-secret>
```

Available profiles: `generic`, `github`, `gitlab`, `stripe`. Each sends a
realistic sample payload with the correct headers and signature.

## Example: GitHub PR Reviewer

A complete example of setting up a webhook that reviews pull requests:

1. **Create webhook** in Settings → Webhooks:
   - Source: **GitHub**
   - Auth: **GitHub HMAC-SHA256** (auto-selected)
   - Events: check `pull_request.opened` and `pull_request.synchronize`
   - Agent: `code-reviewer` (or your default agent)
   - Session mode: **Per entity** (groups all events for the same PR)
   - System prompt suffix:

     ```
     You are reviewing a GitHub pull request. Analyze the PR description and
     changed files. Focus on correctness, security, and maintainability.
     Provide specific, actionable feedback.
     ```

2. **Register in GitHub**:
   - Repo → Settings → Webhooks → Add webhook
   - Payload URL: copy from Moltis
   - Content type: `application/json`
   - Secret: copy from Moltis
   - Events: "Pull requests"

3. **Test it**: open a PR — a new session appears in Moltis with the agent's
   review.

## Example: Stripe Payment Handler

1. **Create webhook** in Settings → Webhooks:
   - Source: **Stripe**
   - Auth: **Stripe Signature** (auto-selected)
   - Events: check `checkout.session.completed`, `payment_intent.payment_failed`
   - Session mode: **Per delivery**
   - System prompt suffix:

     ```
     Process this Stripe payment event. For successful payments, log the
     details and confirm fulfillment. For failures, summarize the issue
     and suggest next steps.
     ```

2. **Register in Stripe**:
   - Dashboard → Developers → Webhooks → Add endpoint
   - Endpoint URL: copy from Moltis
   - Events: select the matching events
   - Copy signing secret (`whsec_...`) into Moltis

## Metrics

| Metric | Type | Description |
|--------|------|-------------|
| `webhooks_deliveries_total` | Counter | Total deliveries by webhook, status, event type |
| `webhooks_deliveries_rejected_total` | Counter | Rejected deliveries by reason |
| `webhooks_deliveries_filtered_total` | Counter | Filtered deliveries |
| `webhooks_processing_duration_seconds` | Histogram | Agent execution time |
| `webhooks_response_actions_total` | Counter | Response actions by tool and status |
| `webhooks_rate_limited_total` | Counter | Rate-limited requests |
| `webhooks_worker_queue_depth` | Gauge | Pending deliveries in worker queue |

## Deliver-Only Mode (Webhook Proxy)

By default, each webhook delivery triggers an agent run — the LLM processes the
event, reasons about it, and optionally acts using tools. This costs tokens and
takes seconds.

**Deliver-only mode** skips the agent entirely. The webhook payload is rendered
through a template and forwarded directly to a channel. Zero LLM tokens,
sub-second delivery.

This turns Moltis into a **webhook proxy**: external services POST events, and
formatted messages appear in your Telegram, Discord, Slack, or any other
configured channel.

### When to Use Deliver-Only

- **Monitoring alerts**: Datadog/Grafana/Sentry → Discord
- **Payment notifications**: Stripe → Telegram
- **CI/CD status**: GitHub Actions → Slack
- **Inter-service notifications**: any HTTP POST → any channel
- **High-volume events**: where per-event LLM calls would be wasteful

### Configuration

Set `deliver_only: true` and provide a `prompt_template` with `{dot.notation}`
variables, plus a `deliver_to` channel target.

**Web UI**: Toggle "Deliver only" in the webhook settings, fill in the template
and target channel.

**CLI**:
```bash
moltis webhooks create \
  --name deploy-status \
  --source-profile generic \
  --auth-mode static_header \
  --deliver-only \
  --prompt-template "Deploy {status}: {environment} ({commit_sha})" \
  --deliver-to slack
```

**Agent**: The agent can also create deliver-only webhooks using the `webhook` tool.

### How It Works

```
External Service POSTs event
        │
        ▼
┌──────────────────────────────────┐
│         Ingress Handler          │
│  verify → filter → dedup → 202  │
└──────────────┬───────────────────┘
               │
               ▼
┌──────────────────────────────────┐
│       Background Worker          │
│  render template → deliver to    │
│  channel → mark completed        │
│  (no agent, no LLM tokens)       │
└──────────────────────────────────┘
```

## Template Rendering

Templates use `{dot.notation}` to interpolate values from the webhook JSON payload.

| Syntax | Resolves to |
|--------|-------------|
| `{action}` | Top-level field: `payload["action"]` |
| `{pull_request.title}` | Nested: `payload["pull_request"]["title"]` |
| `{pull_request.user.login}` | Deep nested: `payload["pull_request"]["user"]["login"]` |
| `{__raw__}` | Full payload as indented JSON (truncated at 4000 chars) |

- **Missing keys** are left as literal `{key}` in the output (no error).
- **Objects and arrays** are JSON-serialized (truncated at 2000 chars).
- **Strings and numbers** are inserted directly.

Templates work in both `prompt_template` (the message body) and `deliver_extra`
values (channel-specific metadata like chat IDs).

### Example: GitHub Issue → Telegram

```bash
moltis webhooks create \
  --name github-issues \
  --source-profile github \
  --auth-mode github_hmac_sha256 \
  --deliver-only \
  --prompt-template "#{issue.number} {issue.title} ({action} by {sender.login})" \
  --deliver-to telegram \
  --events "issues.opened,issues.closed"
```

When someone opens issue #42 "Fix auth bug", Telegram receives:
```
#42 Fix auth bug (opened by alice)
```

### Example: Stripe Payment → Discord

```bash
moltis webhooks create \
  --name stripe-notify \
  --source-profile stripe \
  --auth-mode stripe_webhook_signature \
  --deliver-only \
  --prompt-template "Payment {data.object.status}: {data.object.amount} cents ({data.object.currency})" \
  --deliver-to discord \
  --events "payment_intent.succeeded,payment_intent.payment_failed"
```

## Comparison with Channels and Cron

| | Channels | Webhooks | Cron |
|---|---------|----------|------|
| **Purpose** | Human messaging | Machine event ingress | Scheduled tasks |
| **Trigger** | User sends message | External HTTP POST | Time-based schedule |
| **Reply** | Back to the channel | Via response tools (optional) | Optional channel delivery |
| **Session** | Per conversation | Per delivery / entity | Per job run |
| **Auth** | Platform account | Per-webhook (HMAC, token, etc.) | Internal only |

Webhooks are **not channels**. They do not support reply routing, streaming, or
platform presence semantics. Use channels for human messaging and webhooks for
machine event ingress.
