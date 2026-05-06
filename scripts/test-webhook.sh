#!/usr/bin/env bash
# Test a Moltis webhook endpoint with realistic payloads.
#
# Usage:
#   ./scripts/test-webhook.sh <URL> [--profile github|gitlab|stripe|generic] [--secret SECRET]
#
# Examples:
#   ./scripts/test-webhook.sh http://localhost:18789/api/webhooks/ingest/wh_abc123
#   ./scripts/test-webhook.sh http://localhost:18789/api/webhooks/ingest/wh_abc123 --profile github --secret mysecret
#   ./scripts/test-webhook.sh http://localhost:18789/api/webhooks/ingest/wh_abc123 --profile stripe --secret whsec_test123
#   ./scripts/test-webhook.sh http://localhost:18789/api/webhooks/ingest/wh_abc123 --profile generic --secret mytoken

set -euo pipefail

URL="${1:?Usage: $0 <webhook-url> [--profile github|gitlab|stripe|generic] [--secret SECRET]}"
shift

PROFILE="generic"
SECRET=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --profile) PROFILE="$2"; shift 2 ;;
    --secret)  SECRET="$2";  shift 2 ;;
    *) echo "Unknown arg: $1"; exit 1 ;;
  esac
done

# ── Payloads ────────────────────────────────────────────────────────────

GITHUB_PR_PAYLOAD='{
  "action": "opened",
  "number": 42,
  "pull_request": {
    "number": 42,
    "title": "Add webhook support",
    "user": { "login": "testuser" },
    "head": { "ref": "feature/webhooks" },
    "base": { "ref": "main" },
    "html_url": "https://github.com/example/repo/pull/42",
    "body": "This PR adds generic webhook support to the project.\n\nChanges:\n- New webhook ingress endpoint\n- Source profiles for GitHub, GitLab, Stripe\n- Event filtering and deduplication",
    "draft": false,
    "additions": 1203,
    "deletions": 156,
    "changed_files": 42,
    "mergeable": true
  },
  "repository": {
    "full_name": "example/repo",
    "html_url": "https://github.com/example/repo"
  },
  "sender": { "login": "testuser" }
}'

GITHUB_ISSUE_PAYLOAD='{
  "action": "opened",
  "issue": {
    "number": 99,
    "title": "Bug: webhook delivery fails silently",
    "user": { "login": "reporter" },
    "html_url": "https://github.com/example/repo/issues/99",
    "body": "When sending a webhook with an invalid JSON body, the delivery is accepted but no error is shown in the UI.\n\nSteps to reproduce:\n1. Send POST with Content-Type: application/json but body is plain text\n2. Check deliveries panel\n3. Status shows \"failed\" but no error detail",
    "labels": [{"name": "bug"}, {"name": "webhooks"}]
  },
  "repository": {
    "full_name": "example/repo",
    "html_url": "https://github.com/example/repo"
  },
  "sender": { "login": "reporter" }
}'

GITLAB_MR_PAYLOAD='{
  "object_kind": "merge_request",
  "event_type": "merge_request",
  "user": { "username": "testuser", "name": "Test User" },
  "project": {
    "path_with_namespace": "group/project",
    "web_url": "https://gitlab.com/group/project"
  },
  "object_attributes": {
    "iid": 15,
    "title": "Implement webhook handler",
    "action": "open",
    "state": "opened",
    "url": "https://gitlab.com/group/project/-/merge_requests/15",
    "source_branch": "feature/webhooks",
    "target_branch": "main",
    "description": "This MR implements the webhook HTTP handler with auth verification."
  }
}'

STRIPE_CHECKOUT_PAYLOAD='{
  "id": "evt_test_1234567890",
  "type": "checkout.session.completed",
  "api_version": "2024-12-18",
  "livemode": false,
  "data": {
    "object": {
      "id": "cs_test_abc123",
      "object": "checkout.session",
      "customer": "cus_TestCustomer",
      "customer_email": "buyer@example.com",
      "amount_total": 4999,
      "currency": "usd",
      "payment_status": "paid",
      "mode": "subscription",
      "subscription": "sub_TestSub789",
      "status": "complete",
      "metadata": {
        "plan": "pro",
        "referral": "test_campaign"
      }
    }
  }
}'

GENERIC_PAYLOAD='{
  "event": "deploy.completed",
  "service": "api-server",
  "environment": "staging",
  "version": "2.4.1",
  "commit": "abc123def",
  "status": "success",
  "duration_seconds": 142,
  "deployed_by": "ci-bot",
  "url": "https://staging.example.com",
  "timestamp": "2026-04-07T15:30:00Z"
}'

# ── Auth helpers ────────────────────────────────────────────────────────

compute_github_signature() {
  local secret="$1" body="$2"
  echo -n "$body" | openssl dgst -sha256 -hmac "$secret" | sed 's/^.* //'
}

compute_stripe_signature() {
  local secret="$1" body="$2"
  local timestamp
  timestamp=$(date +%s)
  local signed_payload="${timestamp}.${body}"
  local sig
  sig=$(echo -n "$signed_payload" | openssl dgst -sha256 -hmac "$secret" | sed 's/^.* //')
  echo "t=${timestamp},v1=${sig}"
}

# ── Send ────────────────────────────────────────────────────────────────

send_webhook() {
  local profile="$1" payload="$2" extra_headers=()

  echo "────────────────────────────────────────────"
  echo "Profile: $profile"
  echo "URL:     $URL"

  case "$profile" in
    github)
      local event_type="pull_request"
      # Pick event type from action
      if echo "$payload" | grep -q '"object_kind"'; then
        event_type="merge_request"
      elif echo "$payload" | grep -q '"issue"'; then
        event_type="issues"
      fi
      extra_headers+=(-H "X-GitHub-Event: $event_type")
      extra_headers+=(-H "X-GitHub-Delivery: $(uuidgen 2>/dev/null || echo test-$(date +%s))")
      if [[ -n "$SECRET" ]]; then
        local sig
        sig=$(compute_github_signature "$SECRET" "$payload")
        extra_headers+=(-H "X-Hub-Signature-256: sha256=$sig")
        echo "Auth:    HMAC-SHA256 (signed)"
      fi
      ;;
    gitlab)
      extra_headers+=(-H "X-Gitlab-Event: Merge Request Hook")
      if [[ -n "$SECRET" ]]; then
        extra_headers+=(-H "X-Gitlab-Token: $SECRET")
        echo "Auth:    GitLab token"
      fi
      ;;
    stripe)
      if [[ -n "$SECRET" ]]; then
        local stripe_sig
        stripe_sig=$(compute_stripe_signature "$SECRET" "$payload")
        extra_headers+=(-H "Stripe-Signature: $stripe_sig")
        echo "Auth:    Stripe signature"
      fi
      ;;
    generic)
      extra_headers+=(-H "X-Event-Type: deploy.completed")
      extra_headers+=(-H "X-Delivery-Id: test-$(date +%s)")
      if [[ -n "$SECRET" ]]; then
        extra_headers+=(-H "X-Webhook-Secret: $SECRET")
        echo "Auth:    Static header"
      fi
      ;;
  esac

  if [[ -z "$SECRET" ]]; then
    echo "Auth:    none (no --secret provided)"
  fi
  echo ""

  echo "Request headers:"
  for h in "${extra_headers[@]}"; do
    echo "  $h"
  done
  echo ""
  echo "Sending..."
  echo ""

  local response http_code
  response=$(curl -sk -w "\n__HTTP_CODE__%{http_code}" \
    -X POST "$URL" \
    -H "Content-Type: application/json" \
    "${extra_headers[@]}" \
    -d "$payload" 2>&1)

  http_code="${response##*__HTTP_CODE__}"
  local body="${response%__HTTP_CODE__*}"

  echo "Response body:"
  echo "$body" | python3 -m json.tool 2>/dev/null || echo "$body"
  echo ""
  echo "HTTP Status: $http_code"

  if [[ "$http_code" == "401" ]]; then
    echo ""
    echo "⚠  Auth failed. If your webhook uses 'static_header' auth mode,"
    echo "   pass --secret <value> matching what you configured."
    echo "   If your webhook uses 'none' auth, edit it in Settings → Webhooks."
  elif [[ "$http_code" == "000" ]]; then
    echo ""
    echo "⚠  Connection failed. Check that:"
    echo "   - The Moltis server is running"
    echo "   - The URL is correct"
    echo "   - For HTTPS, the certificate is valid (script uses -k to skip verification)"
  fi
  echo "────────────────────────────────────────────"
  echo ""
}

# ── Main ────────────────────────────────────────────────────────────────

echo ""
echo "Moltis Webhook Test"
echo "==================="
echo ""

case "$PROFILE" in
  github)
    echo "Sending GitHub pull_request.opened event..."
    send_webhook github "$GITHUB_PR_PAYLOAD"

    read -rp "Send another event? (GitHub issue.opened) [y/N] " yn
    if [[ "$yn" =~ ^[Yy] ]]; then
      echo ""
      echo "Sending GitHub issues.opened event..."
      send_webhook github "$GITHUB_ISSUE_PAYLOAD"
    fi
    ;;
  gitlab)
    echo "Sending GitLab merge_request.open event..."
    send_webhook gitlab "$GITLAB_MR_PAYLOAD"
    ;;
  stripe)
    echo "Sending Stripe checkout.session.completed event..."
    send_webhook stripe "$STRIPE_CHECKOUT_PAYLOAD"
    ;;
  generic)
    echo "Sending generic deploy.completed event..."
    send_webhook generic "$GENERIC_PAYLOAD"
    ;;
  *)
    echo "Unknown profile: $PROFILE"
    echo "Available: github, gitlab, stripe, generic"
    exit 1
    ;;
esac

echo "Done. Check Settings → Webhooks → Deliveries in the Moltis UI."
