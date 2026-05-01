---
name: webhook-subscriptions
description: Create and manage webhook subscriptions for event-driven agent activation. Use when the user wants external services (GitHub, GitLab, Stripe, Linear, PagerDuty, Sentry, or any generic source) to trigger agent runs by POSTing events to a URL.
origin:
  source: hermes-agent
  url: https://github.com/nousresearch/hermes-agent
  version: 9f22977f
---

# Webhook Subscriptions

Create webhook subscriptions so external services can trigger agent runs by POSTing events to Moltis.

Webhooks are available as soon as the Moltis gateway is running — no extra setup needed.

## Ingress Endpoint

Each webhook gets a unique URL:

```
POST https://<moltis-host>/api/webhooks/ingest/{public_id}
```

The `public_id` is a high-entropy identifier like `wh_a1b2c3d4...` assigned at creation.

## Managing Webhooks

Webhooks are managed via RPC or the web UI. The RPC namespace is `webhooks.*`.

### Create a webhook

```json
// RPC: webhooks.create
{
  "name": "github-issues",
  "description": "Triage new GitHub issues",
  "source_profile": "github",
  "auth_mode": "github_hmac_sha256",
  "auth_config": { "secret": "your-github-webhook-secret" },
  "event_filter": { "allow": ["issues.opened", "issues.reopened"] },
  "session_mode": "per_entity",
  "system_prompt_suffix": "Triage this issue: assign a priority label and suggest next steps."
}
```

Returns the webhook with its `public_id` (the URL slug) and all configuration.

### List webhooks

```json
// RPC: webhooks.list
```

### Get webhook details

```json
// RPC: webhooks.get
{ "id": 123 }
```

### Update a webhook

```json
// RPC: webhooks.update
{
  "id": 123,
  "patch": {
    "enabled": false,
    "event_filter": { "allow": ["issues.opened"], "deny": ["issues.closed"] }
  }
}
```

### Delete a webhook

```json
// RPC: webhooks.delete
{ "id": 123 }
```

### View deliveries

```json
// RPC: webhooks.deliveries
{ "webhookId": 123, "limit": 50, "offset": 0 }

// RPC: webhooks.delivery_get
{ "id": 456 }

// RPC: webhooks.delivery_payload
{ "id": 456 }
```

## Source Profiles

Each webhook has a `source_profile` that determines how events are parsed and authenticated. Use the profile that matches the sending service for best results.

| Profile | Auth Mode | Event Parsing | Dedup Header |
|---------|-----------|---------------|-------------|
| `github` | `github_hmac_sha256` | `x-github-event` + action | `x-github-delivery` |
| `gitlab` | `gitlab_token` | Event from headers | GitLab delivery ID |
| `stripe` | `stripe_webhook_signature` | Event type from JSON body | Stripe event ID |
| `linear` | `linear_webhook_signature` | Event from headers | Linear delivery ID |
| `pagerduty` | `pagerduty_v2_signature` | Event from headers | PagerDuty dedup key |
| `sentry` | `sentry_webhook_signature` | Event from headers | Sentry event ID |
| `generic` | `none` (or any) | `x-event-type` header | `x-delivery-id` / `idempotency-key` |

List available profiles via RPC:

```json
// RPC: webhooks.profiles
```

## Authentication Modes

| Mode | Config Keys | What It Verifies |
|------|-----------|-----------------|
| `none` | — | Nothing (open endpoint) |
| `static_header` | `header`, `value` | Exact match on a custom header |
| `bearer` | `token` | `Authorization: Bearer <token>` |
| `github_hmac_sha256` | `secret` | HMAC-SHA256 of body vs `x-hub-signature-256` |
| `gitlab_token` | `token` | Exact match of `x-gitlab-token` |
| `stripe_webhook_signature` | `secret` | Stripe `t=TIMESTAMP,v1=SIG` format with 5min tolerance |
| `linear_webhook_signature` | `secret` | HMAC-SHA256 vs `linear-signature` |
| `pagerduty_v2_signature` | `secret` | HMAC-SHA256 vs `x-pagerduty-signature` |
| `sentry_webhook_signature` | `secret` | HMAC-SHA256 vs `sentry-hook-signature` |

All verifications use constant-time comparison to prevent timing attacks.

## Session Modes

Control how webhook deliveries are grouped into agent sessions:

| Mode | Behavior | Use When |
|------|----------|----------|
| `per_delivery` | Fresh session for each POST | Independent events (alerts, deploys) |
| `per_entity` | Same session for same entity | Related events (all activity on PR #123) |
| `named_session` | Fixed session from `named_session_key` | All events share one conversation |

`per_entity` is powerful for GitHub: all events for the same PR or issue land in the same session, so the agent has full context of the conversation.

## Event Filtering

Filter which event types trigger agent runs:

```json
{
  "event_filter": {
    "allow": ["issues.opened", "pull_request.opened"],
    "deny": ["issues.closed"]
  }
}
```

- If `allow` is non-empty, only listed events pass.
- `deny` always wins over `allow`.
- Filtered events return `200 OK` with `status: filtered` (the sender sees success).

## Tool Policy

Restrict which tools the agent may use for webhook-triggered runs:

```json
{
  "tool_policy": {
    "allow": ["exec", "web_fetch"],
    "deny": ["delete_file"]
  }
}
```

## IP Allowlist

Restrict which IPs can send webhooks:

```json
{
  "allowed_cidrs": ["192.30.252.0/22", "185.199.108.0/22"]
}
```

For GitHub, use their published webhook IP ranges. CIDR check runs before auth verification.

## Common Patterns

### GitHub: triage new issues

```json
{
  "name": "github-issues",
  "source_profile": "github",
  "auth_mode": "github_hmac_sha256",
  "auth_config": { "secret": "whsec_..." },
  "event_filter": { "allow": ["issues.opened"] },
  "session_mode": "per_entity",
  "system_prompt_suffix": "Triage this issue: assign priority, suggest labels, draft a response."
}
```

Then in GitHub repo → Settings → Webhooks → Add webhook:
- **Payload URL:** `https://<moltis-host>/api/webhooks/ingest/<public_id>`
- **Content type:** `application/json`
- **Secret:** same as `auth_config.secret`
- **Events:** Select "Issues"

### GitHub: PR review assistant

```json
{
  "name": "github-prs",
  "source_profile": "github",
  "auth_mode": "github_hmac_sha256",
  "auth_config": { "secret": "whsec_..." },
  "event_filter": { "allow": ["pull_request.opened", "pull_request.synchronize"] },
  "session_mode": "per_entity",
  "system_prompt_suffix": "Review this PR for code quality, potential bugs, and style issues."
}
```

### Stripe: payment monitoring

```json
{
  "name": "stripe-payments",
  "source_profile": "stripe",
  "auth_mode": "stripe_webhook_signature",
  "auth_config": { "secret": "whsec_..." },
  "event_filter": {
    "allow": ["payment_intent.succeeded", "payment_intent.payment_failed", "charge.dispute.created"]
  },
  "session_mode": "per_delivery",
  "system_prompt_suffix": "Summarize this payment event and flag anything unusual."
}
```

### Generic: monitoring alerts

```json
{
  "name": "alerts",
  "source_profile": "generic",
  "auth_mode": "bearer",
  "auth_config": { "token": "my-alert-token" },
  "session_mode": "per_delivery",
  "system_prompt_suffix": "Investigate this alert and suggest remediation steps."
}
```

### GitLab: pipeline notifications

```json
{
  "name": "gitlab-ci",
  "source_profile": "gitlab",
  "auth_mode": "gitlab_token",
  "auth_config": { "token": "my-gitlab-token" },
  "event_filter": { "allow": ["pipeline"] },
  "session_mode": "per_delivery",
  "system_prompt_suffix": "Summarize this CI pipeline result."
}
```

## Deliver-Only Mode (Zero LLM Tokens)

Set `deliver_only: true` to skip the agent and forward a rendered template directly
to a channel. This is a webhook proxy — zero LLM cost, sub-second delivery.

Use `prompt_template` with `{dot.notation}` variables and `deliver_to` for the target channel.

### Template syntax

- `{field}` — top-level field from the payload
- `{object.nested.field}` — nested access
- `{__raw__}` — full payload as JSON (truncated)
- Missing keys are left as `{key}` literals

### Example: GitHub issues → Telegram (no LLM)

```json
{
  "name": "github-issues-notify",
  "source_profile": "github",
  "auth_mode": "github_hmac_sha256",
  "auth_config": { "secret": "whsec_..." },
  "event_filter": { "allow": ["issues.opened", "issues.closed"] },
  "deliver_only": true,
  "prompt_template": "#{issue.number} {issue.title} ({action} by {sender.login})",
  "deliver_to": "telegram"
}
```

### Example: Stripe payments → Discord (no LLM)

```json
{
  "name": "stripe-notify",
  "source_profile": "stripe",
  "auth_mode": "stripe_webhook_signature",
  "auth_config": { "secret": "whsec_..." },
  "event_filter": { "allow": ["payment_intent.succeeded"] },
  "deliver_only": true,
  "prompt_template": "Payment: {data.object.amount} {data.object.currency} — {data.object.status}",
  "deliver_to": "discord"
}
```

## Agent Tool

The `webhook` tool is available to agents. Ask the agent to create webhooks:

> "Set up a GitHub webhook for PR reviews on my repo"
> "Create a deliver-only webhook that forwards Stripe payments to my Telegram"

The agent uses the `webhook` tool with actions: `list`, `create`, `get`, `update`, `delete`, `profiles`, `deliveries`.

## Rate Limiting

Global config in `moltis.toml`:

```toml
[webhooks.rate_limit]
enabled = true
requests_per_minute = 300
burst = 30
```

Per-webhook rate limits are set via `rate_limit_per_minute` (default 60) in the webhook config.

Exceeded limits return `429 Too Many Requests`.

## Built-in Deduplication

If a service retries a delivery, Moltis automatically deduplicates based on the delivery key (e.g. `x-github-delivery` header). Duplicate POSTs return `200 OK` with `status: deduplicated` — no double processing.

## How It Works

1. External service POSTs to `/api/webhooks/ingest/{public_id}`
2. Moltis verifies auth, checks CIDR allowlist, enforces rate limits
3. Source profile parses event type and delivery key from headers/body
4. Event filter decides whether to process or skip
5. Deduplication check prevents duplicate processing
6. Delivery is persisted and queued for async processing
7. The webhook worker normalizes the payload into a human-readable message
8. An agent run is triggered with the normalized message + `system_prompt_suffix`
9. The full delivery history (status, tokens, duration, tool actions) is recorded

## Troubleshooting

1. **Is Moltis running?** `curl http://localhost:<port>/api/gon` should return JSON.
2. **Auth failure (401)?** Verify the secret in your service matches `auth_config`. GitHub sends `X-Hub-Signature-256`, GitLab sends `X-Gitlab-Token`, Stripe uses `Stripe-Signature`.
3. **IP blocked (403)?** Check `allowed_cidrs` matches the sender's IP range.
4. **Rate limited (429)?** Check per-webhook and global rate limits.
5. **Events filtered?** Verify `event_filter.allow` includes the event type the service sends. Check delivery history for `status: filtered`.
6. **Firewall/NAT?** The webhook URL must be reachable from the sending service. For local development, use a tunnel (ngrok, cloudflared, or Moltis's built-in Tailscale/ngrok support).
7. **Check delivery history:** Use `webhooks.deliveries` RPC to inspect status, rejection reasons, and timing.
