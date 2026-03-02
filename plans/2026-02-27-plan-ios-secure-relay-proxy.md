# iOS Secure Relay Proxy Plan

**Status:** Planned  
**Priority:** High  
**Date:** 2026-02-27  
**Scope:** Provide secure remote access for iOS companion clients to Moltis instances behind NAT/firewalls, using Moltis-operated relay servers.

## Goal

Enable users to connect from iOS to their self-hosted Moltis securely, even when the home server is not directly reachable due to NAT/firewall restrictions, without requiring users to expose inbound ports.

## Non-Goals (v1)

- Full mesh VPN replacement.
- Metadata privacy from relay operators (v1 hides payload content, not all metadata).
- Direct peer-to-peer hole punching (can be added later).

## Current Gaps in Codebase

1. Pairing state and device token lifecycle are currently in-memory.
2. `node.pair.verify` is a placeholder.
3. WebSocket `connect.auth` supports token/password/api key, but no first-class device token auth path.
4. `hello.auth.deviceToken` is currently not populated with meaningful device identity.

## Security Objectives

1. End-to-end confidentiality of user payloads between iOS client and home Moltis server.
2. Strong mutual authentication between paired iOS device and target Moltis instance.
3. Replay resistance, short-lived session credentials, and fast revocation.
4. Relay compromise should not expose decrypted application data.
5. TLS everywhere, with sane certificate rotation and pinning strategy.

## Threat Model

### In-Scope Adversaries

1. Internet attacker probing relay endpoints.
2. Attacker with stolen device token or stale credential.
3. Malicious/compromised relay edge node.
4. Replay and downgrade attempts on handshake messages.
5. Abuse traffic against free relay infrastructure.

### Out-of-Scope (Explicit)

1. Fully compromised paired iOS device with active app credentials.
2. Physical compromise of the home server OS with root access.

## Architecture Overview

### Components

1. **Relay Control Plane (`relay-control`)**
   - Handles registration/bootstrap.
   - Issues short-lived connect grants.
   - Publishes revocations and key metadata (JWKS-like endpoint).

2. **Relay Data Plane (`relay-edge`)**
   - Public `wss://` endpoint on 443.
   - Maintains long-lived outbound tunnel from home Moltis connector.
   - Accepts iOS client tunnels and routes to target instance tunnel.
   - Does authenticated rendezvous and byte forwarding only.

3. **Home Connector (inside `moltis-gateway`)**
   - Maintains outbound relay session.
   - Reconnects with backoff and jitter.
   - Exposes tunnel stream to gateway RPC layer.

4. **iOS Companion Relay Client**
   - Connects to relay edge over TLS.
   - Uses paired device identity to request/connect to target instance.
   - Runs end-to-end encrypted application channel over relay stream.

### Transport Layers

1. **Outer transport:** TLS 1.3 + WebSocket (`wss`) between each endpoint and relay.
2. **Inner transport:** End-to-end encrypted Moltis RPC stream between iOS and home gateway.

Inference: this preserves compatibility with current WebSocket-first architecture while minimizing trust in relay operators.

## Identity, Pairing, and Auth Model

### Device Pairing

1. iOS generates device keypair (stored in Keychain/Secure Enclave where possible).
2. Pairing request includes device public key and metadata.
3. Operator approves pairing from authenticated Moltis UI/API.
4. Gateway stores device identity and scoped permissions in persistent storage.

### Persistent Records

Store these in gateway DB:

1. `paired_devices` (device ID, public key, status, created/rotated/revoked timestamps).
2. `pair_requests` (nonce/challenge, expiry, status).
3. `device_tokens` (hashed token, scopes, expiry, revocation state).

### Session Auth

1. iOS requests a short-lived connect grant (minutes, not days).
2. Grant binds:
   - instance ID
   - device ID
   - nonce
   - expiry
   - required role/scopes
3. Home gateway re-validates device identity during end-to-end handshake before accepting RPC traffic.

## End-to-End Encryption Design

### Handshake

1. Use a proven pattern (Noise-based or equivalent) with ephemeral key exchange and identity binding.
2. Include relay-routed session context (instance ID, device ID, nonce, expiry) in handshake transcript.
3. Reject any replayed or stale transcript.

### Data Channel

1. Derive per-session symmetric keys from handshake.
2. Encrypt each frame with AEAD and monotonic counters.
3. Enforce strict nonce/counter checks; close channel on desync.

### Why This Matters

Even if a relay edge is compromised, it sees encrypted frames and metadata, not plaintext RPC payloads.

## TLS / Certificates / PKI

1. Relay public endpoints use CA-issued certs (ACME automation).
2. TLS 1.3 only, strong cipher suites, HSTS on control plane HTTPS.
3. Home connector authenticates to control/edge using mTLS or equivalent signed client credential.
4. iOS uses ATS-compliant networking and certificate pinning policy for relay endpoints.
5. Define key/cert rotation cadence and emergency rollover playbook.

## Abuse and Platform Hardening (Free Relay Reality)

1. Per-IP and per-device rate limits on connect/bootstrap paths.
2. Concurrent tunnel caps per instance and per account.
3. Bandwidth/egress quotas and automatic throttling.
4. Basic bot/abuse mitigation at edge/WAF.
5. Structured audit logs without payload logging.

## Multi-Relay and Availability

1. Start with one region, design for multiple edges from day one.
2. Add region-aware bootstrap and nearest-edge selection.
3. Keep home connector reconnect logic robust (jittered exponential backoff).
4. Add standby path to second edge in v2 for failover.
5. Use distributed in-memory/state store (e.g., Redis) only for rendezvous metadata.

## Implementation Phases

## Phase 0: Pairing/Auth Hardening (Prerequisite)

1. Persist pairing/device data in DB.
2. Replace placeholder verify with real signature challenge flow.
3. Add first-class device token verification path in WS connect/auth.
4. Add revoke/rotate endpoints with immediate enforcement.

## Phase 1: Relay MVP (Single Region)

1. Build `relay-control` bootstrap/grant APIs.
2. Build `relay-edge` tunnel multiplexer.
3. Add home connector task in gateway.
4. Add iOS relay connection mode and instance targeting.

## Phase 2: E2EE Channel

1. Implement handshake and frame encryption.
2. Bind grants to handshake transcript.
3. Enforce replay prevention and strict clock/expiry checks.

## Phase 3: Operations and Hardening

1. Metrics, tracing, alerts, dashboards.
2. Abuse protections and quotas.
3. Regional expansion and failover drills.
4. Incident response and secret rotation runbooks.

## Testing Strategy

1. Unit tests:
   - handshake transcript validation
   - replay rejection
   - token expiry/revocation logic
2. Integration tests:
   - iOS client -> relay -> gateway roundtrip
   - relay restart during active session
   - revoked device denied mid-session
3. Security tests:
   - MITM simulation between client and relay
   - forged grant/device identity attempts
   - protocol downgrade attempts
4. Load tests:
   - many idle tunnels
   - burst reconnect storms

## Rollout Plan

1. Feature flag relay path (default off).
2. Internal canary users first.
3. Gradual opt-in rollout.
4. Emergency kill switch and rollback path to direct/Tailscale modes.

## Open Questions

1. Should v1 include direct-path ICE/STUN attempts, or relay-only first?
2. What per-user free-tier limits are acceptable (bandwidth, concurrent sessions)?
3. Do we require account-level identity for relay use, or allow anonymous paired-instance mode?
4. Is iOS background reconnect requirement strict for v1, or foreground-only acceptable initially?

## References

1. TLS 1.3 (RFC 8446): https://www.rfc-editor.org/rfc/rfc8446
2. QUIC transport (RFC 9000): https://www.rfc-editor.org/rfc/rfc9000
3. QUIC-TLS (RFC 9001): https://www.rfc-editor.org/rfc/rfc9001.txt
4. ICE (RFC 8445): https://www.rfc-editor.org/rfc/rfc8445
5. STUN (RFC 5389): https://www.rfc-editor.org/rfc/rfc5389
6. TURN (RFC 8656): https://www.rfc-editor.org/rfc/rfc8656
7. Noise Protocol Framework: https://noiseprotocol.org/noise_rev34.html
8. Tailscale NAT traversal explainer: https://tailscale.com/blog/how-nat-traversal-works
9. Tailscale DERP overview: https://tailscale.com/kb/1232/derp-servers
10. OWASP MASVS platform controls: https://mas.owasp.org/MASVS/
11. Apple ATS reference: https://developer.apple.com/library/archive/documentation/General/Reference/InfoPlistKeyReference/Articles/CocoaKeys.html

