# Plan: Rustls Migration and OpenSSL Reduction

**Status:** Proposed  
**Date:** 2026-02-14  
**Scope:** Workspace-wide dependency and release pipeline changes

## Goal

Make release builds (especially Windows) reliable and reduce OpenSSL coupling by:

1. Moving network TLS paths to `rustls` where practical.
2. Isolating OpenSSL-only features behind explicit feature gates.
3. Defining a path to remove OpenSSL from default shipping builds.

## Why This Exists

The `v0.8.24` release failed on Windows (`Build .exe (x86_64)`) because vendored OpenSSL invoked a Perl runtime missing modules (`Locale::Maketext::Simple`).

This is a symptom of a broader issue: TLS/crypto dependencies are currently mixed across crates, and OpenSSL is still required by key features.

## Current Dependency Reality (As Observed)

`openssl-sys` is pulled by:

1. `webauthn-rs-core` (via `webauthn-rs`) for passkeys/WebAuthn.
2. `web-push` (via `isahc`/`curl` and `ece` backend-openssl) for push notifications.
3. Explicit workspace dependency on `openssl` (vendored).

Relevant local evidence:

- `cargo tree -p moltis -i openssl-sys --edges normal,build`
- `Cargo.toml` workspace deps (`openssl = { features = ["vendored"], ... }`)
- `crates/gateway/Cargo.toml` (`openssl`, `webauthn-rs`, `web-push` path via feature)

## Non-Goals

1. No immediate rewrite of all auth/push logic in one PR.
2. No silent behavior changes for existing users.
3. No regressions in Linux/macOS release packaging.

## Migration Strategy

Use two tracks:

1. **Track A (stability now):** keep releases green while migration is in progress.
2. **Track B (architecture):** move default networking/TLS to rustls and minimize OpenSSL blast radius.

---

## Track A: Immediate Build Stability

### A1. Keep Windows release unblocked

1. Force `OPENSSL_SRC_PERL` and `PERL` to Strawberry Perl in the Windows release job.
2. Log `perl -v` before build.
3. Fail early with a clear message if Strawberry Perl path is missing.

### A2. Add a dependency visibility check in CI (release/debug aid)

Add a diagnostic step (non-blocking initially) that outputs:

1. `cargo tree -p moltis -i openssl-sys`
2. `cargo tree -p moltis -i native-tls`

This prevents guessing when a new dependency reintroduces OpenSSL.

---

## Track B: Rustls-First Migration

### Phase 1: Move HTTP/WebSocket client stack to rustls defaults

1. Update workspace `reqwest` dependency to rustls-first:
   - `default-features = false`
   - Keep only needed features (`json`, `stream`, and crate-specific extras such as `multipart` where required).
   - Add rustls TLS feature set explicitly.
2. Audit all crates using `reqwest` to ensure required features are still enabled.
3. Audit `tokio-tungstenite` TLS feature usage and pin to rustls-compatible features where used.
4. Validate with:
   - `cargo tree -p moltis -i native-tls`
   - `cargo tree -p moltis -i openssl-sys`

**Acceptance criteria**

1. `native-tls` is gone from default build graph.
2. `reqwest` traffic works in unit/e2e paths.
3. No TLS behavior regressions in gateway and providers.

### Phase 2: Isolate OpenSSL-only features in `gateway`

Introduce explicit feature boundaries (names illustrative):

1. `passkeys-openssl` for WebAuthn/passkey stack.
2. `push-openssl` for web-push stack.
3. Keep `tls` (server TLS) independent and rustls-based.

Then:

1. Wire feature forwarding in `crates/cli/Cargo.toml` (default behavior explicit).
2. Ensure compile guards in gateway modules (`#[cfg(feature = "...")]`) are complete.
3. Ensure startup logs indicate when passkeys/push features are disabled at compile time.

**Acceptance criteria**

1. `cargo build --release` can succeed without OpenSSL-dependent features.
2. Feature matrix is explicit and documented.
3. Runtime behavior is predictable when features are disabled.

### Phase 3: Decide long-term replacements for OpenSSL-bound capabilities

#### 3A. Passkeys / WebAuthn decision

Run a short architecture spike:

1. Evaluate staying on `webauthn-rs` (OpenSSL) vs moving to a RustCrypto-first stack (for example, passkey-rs ecosystem).
2. Compare:
   - API maturity and maintenance
   - Server-side ceremony support
   - Attestation support and compatibility
   - Migration effort and risk
3. Produce a decision doc with recommendation and rollback plan.

#### 3B. Web Push decision

1. Re-check latest `web-push` behavior/features and OpenSSL requirements.
2. If still OpenSSL-bound, choose one:
   - Keep as optional feature (disabled on targets where OpenSSL toolchain pain is unacceptable).
   - Replace implementation path with rustls-based HTTP + compatible ECE/VAPID stack.

**Acceptance criteria**

1. Written decision for passkeys.
2. Written decision for push notifications.
3. Explicit target policy (what ships on Windows/Linux/macOS).

### Phase 4: Remove workspace OpenSSL dependency from default profile

Once Phase 3 decisions are implemented:

1. Remove direct `openssl` workspace dependency if no longer needed in default path.
2. Ensure OpenSSL appears only in explicitly opted-in features (or not at all).
3. Enforce with CI guard:
   - default build must not include `openssl-sys`.

**Acceptance criteria**

1. `cargo tree -p moltis -i openssl-sys` is empty for default release profile.
2. Release workflows pass on Windows without Perl/OpenSSL toolchain workarounds.

---

## Release/CI Changes Needed During Migration

1. Keep Windows workaround active until OpenSSL is no longer required by default.
2. Add feature-aware build matrix checks:
   - Full-feature Linux/macOS build.
   - Default-feature Windows build (target baseline).
   - Optional OpenSSL-feature build job to prevent bitrot if those features remain supported.
3. Keep local validation aligned with CI toolchain (`nightly-2025-11-30` for fmt/clippy steps that use nightly).

## Risks and Mitigations

1. **Risk:** Feature-gating passkeys/push creates user-visible differences by platform.
   - **Mitigation:** Log clearly at startup and show UI hints when capability is compile-disabled.

2. **Risk:** Rustls migration changes certificate behavior.
   - **Mitigation:** Add targeted TLS integration tests against representative endpoints.

3. **Risk:** Replacing WebAuthn/push crates may introduce protocol regressions.
   - **Mitigation:** Preserve current behavior behind feature flag while new path is validated.

## Suggested Execution Order

1. Land Track A and CI diagnostics.
2. Land Phase 1 (`reqwest`/WS rustls-first).
3. Land Phase 2 (feature isolation).
4. Run Phase 3 decision spikes.
5. Execute Phase 4 cleanup.

## References

1. `web-push` crate notes OpenSSL requirement: https://lib.rs/crates/web-push
2. `webauthn-rs` docs: https://docs.rs/webauthn-rs/latest/webauthn_rs/
3. `passkey-rs` project: https://github.com/1Password/passkey-rs
4. 1Password background on WebAuthn/OpenSSL constraints: https://blog.1password.com/passkey-crates/
