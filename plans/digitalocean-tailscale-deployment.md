# DigitalOcean + Tailscale Deployment Plan

**Status:** Planned
**Priority:** High (improves remote access UX)
**Complexity:** Medium
**Platform:** DigitalOcean Droplets (primary), App Platform (fallback path)

## Goal

Provide a reliable way to run Moltis on DigitalOcean with secure remote access
through Tailscale, without requiring SSH access to the deployed instance.

## Decision Summary

- **Primary path (recommended):** use `docker compose` on Droplets with a
  dedicated Tailscale sidecar container.
- **Fallback path:** when only a single image is possible, bundle Tailscale
  into the app image and run in userspace mode.
- **App Platform caveat:** do not assume privileged networking (`/dev/net/tun`,
  `NET_ADMIN`) is available; validate userspace mode viability first.

## Architecture Options

### Option A (Preferred): Compose Sidecar on Droplets

Run two containers:

1. `moltis` app container
2. `tailscale/tailscale` sidecar container

Key properties:

- Persist Tailscale state via named volume (`/var/lib/tailscale`)
- Use `TS_AUTHKEY` from DO secrets/environment (never hardcoded)
- Tag nodes for policy-based ACL (`tag:moltis`)
- Expose Moltis to tailnet (`--advertise-tags` and optional funnel/serve rules)

Why this is preferred:

- Clean separation of concerns
- Easier upgrades and troubleshooting
- Avoids process supervision complexity inside the app container

### Option B: Single Image with Embedded Tailscale

Use only when platform constraints prevent a sidecar.

Key properties:

- Install `tailscale` inside Dockerfile
- Start both `tailscaled` and Moltis (entrypoint supervisor script)
- Prefer `tailscaled --tun=userspace-networking` on restricted hosts
- Add health checks for both app and Tailscale status

Tradeoffs:

- More complex runtime orchestration
- Harder to isolate failures and rotate components independently

## Product/UI Changes

Add a Tailscale section in the web UI for non-SSH environments:

- Connection state (connected/disconnected, tailnet name, device name)
- "Connect" action using pre-provided auth key or auth URL flow
- Reconnect/disconnect actions
- Read-only diagnostics (last error, peer IPs, advertised routes)

API namespace (follow repo convention):

- REST: `/api/tailscale/*`
- RPC: `tailscale.*`

Initial endpoints/methods:

- `tailscale.status`
- `tailscale.connect`
- `tailscale.disconnect`
- `tailscale.auth_url` (if interactive flow is needed)

## Security Requirements

- Store auth keys as secrets (`Secret<String>`), never plain `String`
- Pass credentials via env/secret injection, never in Dockerfile or git
- Persist only required Tailscale state; avoid excessive log retention
- Restrict tags/routes to least privilege
- Document key rotation and revocation workflow

## Implementation Phases

### Phase 1: Deployment Baseline (Droplet)

- Create production `docker-compose.yml` with Moltis + Tailscale sidecar
- Add environment contract (`TS_AUTHKEY`, tags, hostname)
- Add persistence and restart policies
- Validate end-to-end startup and tailnet reachability

### Phase 2: Backend Integration

- Add backend module to query Tailscale status and control lifecycle
- Implement `/api/tailscale/*` and `tailscale.*` methods
- Add telemetry (tracing spans + metrics counters/histograms)

### Phase 3: Web UI

- Add Tailscale settings panel in gateway assets
- Surface status, connect/disconnect, and diagnostics
- Keep UI patterns consistent with existing button classes/components

### Phase 4: Platform Compatibility

- Test App Platform viability with userspace networking
- If unsupported, document unsupported mode and fallback guidance
- Provide clear install guidance for Droplet as recommended path

### Phase 5: Hardening and Docs

- Add integration tests for status/connect/disconnect behavior
- Add failure-mode tests (invalid key, network loss, restart recovery)
- Update docs and changelog (`[Unreleased]`)

## Open Questions

- Is DigitalOcean App Platform a hard requirement, or is Droplet-only acceptable?
- Should connect flow require pre-provided `TS_AUTHKEY`, or allow auth URL flow?
- Do we need subnet routes/exit-node support in v1, or just service access?

## Recommended v1 Scope

Ship Droplet + compose sidecar first with auth-key based connection,
status/diagnostics UI, and explicit App Platform limitations documented.
