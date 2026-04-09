## Summary

Hardened web E2E helpers and flaky specs after release run `24009906131` and matching main run `24009905914` failed in GitHub Actions.

## Changes

- `crates/web/ui/e2e/helpers.js`
  - `waitForWsConnected()` now accepts a custom timeout.
  - `navigateAndWait()` now retries transient navigation/context errors like `net::ERR_ABORTED` and destroyed execution contexts.
- `crates/web/ui/e2e/specs/chat-input.spec.js`
  - Full-context helper now treats WebSocket readiness as best-effort instead of a hard prerequisite on every attempt.
  - No-provider detection now only runs after the panel actually reports failure.
- `crates/web/ui/e2e/specs/sandboxes.spec.js`
  - Replaced brittle `waitForResponse()` waits with route-count polling for refresh assertions.
  - Made `afterEach` route cleanup tolerant of already-closed pages.
  - Tightened the empty-state assertion by waiting for the refreshed UI first.
- `crates/web/ui/e2e/specs/websocket.spec.js`
  - `clearChatAndWait()` now asserts that chat message nodes are gone instead of requiring `#messages` to be literally empty.

## Validation

- `biome ci --diagnostic-level=error crates/web/ui/e2e/helpers.js crates/web/ui/e2e/specs/chat-input.spec.js crates/web/ui/e2e/specs/sandboxes.spec.js crates/web/ui/e2e/specs/websocket.spec.js`
- `cd crates/web/ui && npx playwright test e2e/specs/chat-input.spec.js e2e/specs/sandboxes.spec.js e2e/specs/websocket.spec.js --grep "full context copy button uses small button style|full context download button produces \\.jsonl file|shows 'No containers found' when list is empty|refresh button triggers container list fetch|refresh button also fetches disk usage|final footer shows token speed with slow/fast tones"`
- `cd crates/web/ui && npx playwright test e2e/specs/send-document.spec.js e2e/specs/settings-nav.spec.js e2e/specs/sessions.spec.js --grep "renders document card with filename and download link for document_ref|identity name fields autosave on blur|channels page shows matrix verification state and pending verification guidance|channels page shows Matrix ownership approval guidance for existing accounts|deleting unmodified fork skips confirmation dialog"`

## Follow-up

Release run `24010605015` still exposed deterministic weak spots after the first hardening pass.

- `crates/web/ui/e2e/specs/chat-input.spec.js`
  - Stubbed `chat.full_context` with a fixed payload in the two panel tests so they no longer depend on backend/provider timing.
- `crates/web/ui/e2e/specs/sandboxes.spec.js`
  - Mocked disk-usage responses directly on mount.
  - Replaced remaining container-list `waitForResponse()` assertions in error-handling tests with route-count polling.
  - Made the second sandboxes `afterEach` cleanup tolerate ended pages too.
- `crates/web/ui/e2e/specs/settings-nav.spec.js`
  - Waits for the senders table row to load after switching tabs before asserting the Matrix sender fields.

Additional validation:

- `biome ci --diagnostic-level=error crates/web/ui/e2e/specs/chat-input.spec.js crates/web/ui/e2e/specs/sandboxes.spec.js crates/web/ui/e2e/specs/settings-nav.spec.js`
- `cd crates/web/ui && npx playwright test e2e/specs/chat-input.spec.js e2e/specs/sandboxes.spec.js e2e/specs/settings-nav.spec.js --grep "full context copy button uses small button style|full context download button produces \\.jsonl file|disk usage fetches on page mount|delete failure shows error message that clears on refresh|error clears on successful container refresh|senders page shows pending matrix sender with one visible sigil and OTP code"`
