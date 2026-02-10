# Security Architecture

Moltis is designed with a defense-in-depth security model. This document
explains the key security features and provides guidance for production
deployments.

## Overview

Moltis runs AI agents that can execute code and interact with external systems.
This power requires multiple layers of protection:

1. **Human-in-the-loop approval** for dangerous commands
2. **Sandbox isolation** for command execution
3. **Channel authorization** for external integrations
4. **Rate limiting** to prevent resource abuse
5. **Scope-based access control** for API authorization

For marketplace-style skill/plugin hardening (trust gating, provenance pinning,
drift re-trust, dependency install guards, kill switch, audit log), see
[Third-Party Skills Security](skills-security.md).

## Command Execution Approval

By default, Moltis requires explicit user approval before executing potentially
dangerous commands. This "human-in-the-loop" design ensures the AI cannot take
destructive actions without consent.

### How It Works

When the agent wants to run a command:

1. The command is analyzed against approval policies
2. If approval is required, the user sees a prompt in the UI
3. The user can approve, deny, or modify the command
4. Only approved commands execute

### Approval Policies

Configure approval behavior in `moltis.toml`:

```toml
[tools.exec]
approval_mode = "always"  # always require approval
# approval_mode = "smart" # auto-approve safe commands (default)
# approval_mode = "never" # dangerous: never require approval
```

**Recommendation**: Keep `approval_mode = "smart"` (the default) for most use
cases. Only use `"never"` in fully automated, sandboxed environments.

## Sandbox Isolation

Commands execute inside isolated containers (Docker or Apple Container) by
default. This protects your host system from:

- Accidental file deletion or modification
- Malicious code execution
- Resource exhaustion (memory, CPU, disk)

See [sandbox.md](sandbox.md) for backend configuration.

### Resource Limits

```toml
[tools.exec.sandbox.resource_limits]
memory_limit = "512M"
cpu_quota = 1.0
pids_max = 256
```

### Network Isolation

Sandbox containers have limited network access by default. Outbound connections
are allowed but the sandbox cannot bind to host ports.

## Channel Authorization

Channels (Telegram, Slack, etc.) allow external parties to interact with your
Moltis agent. This requires careful access control.

### Sender Allowlisting

When a new sender contacts the agent through a channel, they are placed in a
pending queue. You must explicitly approve or deny each sender before they can
interact with the agent.

```
UI: Settings > Channels > Pending Senders
```

### Per-Channel Permissions

Each channel can have different permission levels:

- **Read-only**: Sender can ask questions, agent responds
- **Execute**: Sender can trigger actions (with approval still required)
- **Admin**: Full access including configuration changes

### Channel Isolation

Channels run in isolated sessions by default. A malicious message from one
channel cannot affect another channel's session or the main UI session.

## Cron Job Security

Scheduled tasks (cron jobs) can run agent turns automatically. Security
considerations:

### Rate Limiting

To prevent prompt injection attacks from rapidly creating many cron jobs:

```toml
[cron]
rate_limit_max = 10           # max jobs per window
rate_limit_window_secs = 60   # window duration (1 minute)
```

This limits job creation to 10 per minute by default. System jobs (like
heartbeat) bypass this limit.

### Job Notifications

When cron jobs are created, updated, or removed, Moltis broadcasts events:

- `cron.job.created` - A new job was created
- `cron.job.updated` - An existing job was modified
- `cron.job.removed` - A job was deleted

Monitor these events to detect suspicious automated job creation.

### Sandbox for Cron Jobs

Cron job execution uses sandbox isolation by default:

```toml
# Per-job configuration
[cron.job.sandbox]
enabled = true              # run in sandbox (default)
# image = "custom:latest"   # optional custom image
```

## Identity Protection

The agent's identity fields (name, emoji, creature, vibe) are stored in `IDENTITY.md`
YAML frontmatter at the workspace root (`data_dir`).
User profile fields are stored in `USER.md` YAML frontmatter at the same location.
The personality text is stored separately in `SOUL.md` at the workspace root (`data_dir`).
Tool guidance is stored in `TOOLS.md` at the workspace root (`data_dir`) and is injected
as workspace context in the system prompt.
Modifying identity requires the `operator.write` scope, not just `operator.read`.

This prevents prompt injection attacks from subtly modifying the agent's
personality to make it more compliant with malicious requests.

## API Authorization

The gateway API uses role-based access control with scopes:

| Scope | Permissions |
|-------|-------------|
| `operator.read` | View status, list jobs, read history |
| `operator.write` | Send messages, create jobs, modify configuration |
| `operator.admin` | All permissions (includes all other scopes) |
| `operator.approvals` | Handle command approval requests |
| `operator.pairing` | Manage device/node pairing |

### API Keys

API keys authenticate external tools and scripts connecting to Moltis. Keys
**must specify at least one scope** — keys without scopes are denied access
(least-privilege by default).

#### Creating API Keys

**Web UI**: Settings > Security > API Keys

1. Enter a label describing the key's purpose
2. Select the required scopes
3. Click "Generate key"
4. **Copy the key immediately** — it's only shown once

**CLI**:

```bash
# Scoped key (comma-separated scopes)
moltis auth create-api-key --label "Monitor" --scopes "operator.read"
moltis auth create-api-key --label "Automation" --scopes "operator.read,operator.write"
moltis auth create-api-key --label "CI pipeline" --scopes "operator.admin"
```

#### Using API Keys

Pass the key in the `connect` handshake over WebSocket:

```json
{
  "method": "connect",
  "params": {
    "client": { "id": "my-tool", "version": "1.0.0" },
    "auth": { "api_key": "mk_abc123..." }
  }
}
```

Or use Bearer authentication for REST API calls:

```
Authorization: Bearer mk_abc123...
```

#### Scope Recommendations

| Use Case | Recommended Scopes |
|----------|-------------------|
| Read-only monitoring | `operator.read` |
| Automated workflows | `operator.read`, `operator.write` |
| Approval handling | `operator.read`, `operator.approvals` |
| Full automation | `operator.admin` |

**Best practice**: Use the minimum necessary scopes. If a key only needs to
read status and logs, don't grant `operator.write`.

#### Backward Compatibility

Existing API keys created without scopes will be **denied access** until
scopes are added. Re-create keys with explicit scopes to restore access.

## Network Security

### TLS Encryption

HTTPS is enabled by default with auto-generated certificates:

```toml
[tls]
enabled = true
auto_generate = true
```

For production, use certificates from a trusted CA or configure custom
certificates.

### Origin Validation

WebSocket connections validate the `Origin` header to prevent cross-site
WebSocket hijacking (CSWSH). Connections from untrusted origins are rejected.

### SSRF Protection

The `web_fetch` tool resolves DNS and blocks requests to private IP ranges
(loopback, RFC 1918, link-local, CGNAT). This prevents server-side request
forgery attacks.

## Three-Tier Authentication Model

Moltis uses a per-request three-tier authentication model that balances
local development convenience with production security:

| Tier | Condition | Behaviour |
|------|-----------|-----------|
| **1** | Password/passkey is set | Auth **always** required (any IP) |
| **2** | No password + direct local connection | Full access (dev convenience) |
| **3** | No password + remote/proxied connection | Onboarding only (setup code required) |

### How "local" is determined

Each incoming request is classified as **local** or **remote** using
four checks that must **all** pass:

1. `MOLTIS_BEHIND_PROXY` env var is **not** set (hard override)
2. No proxy headers present (`X-Forwarded-For`, `X-Real-IP`,
   `CF-Connecting-IP`, `Forwarded`)
3. The `Host` header resolves to a loopback address (or is absent)
4. The TCP source IP is loopback (`127.0.0.1`, `::1`)

If **any** check fails, the connection is treated as remote.

### Practical implications

| Scenario | No password | Password set |
|----------|-------------|-------------|
| Local browser → `localhost:18789` | Full access | Auth required |
| Local CLI/wscat → `localhost:18789` | Full access | Auth required |
| Internet → Caddy (with XFF) → `127.0.0.1:18789` | Onboarding only | Auth required |
| Internet → nginx (with `proxy_set_header`) → `127.0.0.1:18789` | Onboarding only | Auth required |
| Internet → bare nginx (`proxy_pass` only) → `127.0.0.1:18789` | **See below** | Auth required |
| Server bound to `0.0.0.0`, remote client | Onboarding only | Auth required |

## Reverse Proxy Deployments

Running Moltis behind a reverse proxy (Caddy, nginx, Traefik, etc.)
requires understanding how authentication interacts with loopback
connections.

### The problem

When Moltis binds to `127.0.0.1` and a proxy on the same machine
forwards traffic to it, **every** incoming TCP connection appears to
originate from `127.0.0.1` — including requests from the public
internet.  A naive "trust all loopback connections" check would bypass
authentication for all proxied traffic.

This is the same class of vulnerability as
[CVE-2026-25253](https://github.com/openclaw/openclaw/security/advisories/GHSA-g8p2-7wf7-98mq),
which allowed one-click remote code execution on OpenClaw through
authentication token exfiltration and cross-site WebSocket hijacking.

### How Moltis handles it

Moltis uses the per-request `is_local_connection()` check described
above.  Most reverse proxies add forwarding headers or change the
`Host` header, which automatically triggers the "remote" classification.

For proxies that **strip all signals** (e.g. a bare nginx `proxy_pass`
that rewrites `Host` to the upstream address and adds no `X-Forwarded-For`),
use the `MOLTIS_BEHIND_PROXY` environment variable as a hard override:

```bash
MOLTIS_BEHIND_PROXY=true moltis
```

When this variable is set, **all** connections are treated as remote —
no loopback bypass, no exceptions.

### Deploying behind a proxy

1. **Set `MOLTIS_BEHIND_PROXY=true`** if your proxy does not add
   forwarding headers (safest option — eliminates any ambiguity).

2. **Set a password or register a passkey** during initial setup.
   Once a password is configured (Tier 1), authentication is required
   for all traffic regardless of `is_local_connection()`.

3. **WebSocket proxying** must forward the `Origin` and `Host` headers
   correctly.  Moltis validates same-origin on WebSocket upgrades to
   prevent cross-site WebSocket hijacking (CSWSH).

4. **TLS termination** should happen at the proxy.  Moltis can also
   serve TLS directly (`[tls] enabled = true`), but most proxy setups
   handle certificates at the edge.

## Production Recommendations

### 1. Enable Authentication

By default, Moltis requires a password when accessed from non-localhost:

```toml
[auth]
disabled = false  # keep this false in production
```

### 2. Use Sandbox Isolation

Always run with sandbox enabled in production:

```toml
[tools.exec.sandbox]
enabled = true
backend = "auto"  # uses strongest available
```

### 3. Limit Rate Limits

Tighten rate limits for untrusted environments:

```toml
[cron]
rate_limit_max = 5
rate_limit_window_secs = 300  # 5 per 5 minutes
```

### 4. Review Channel Senders

Regularly audit approved senders and revoke access for unknown parties.

### 5. Monitor Events

Watch for these suspicious patterns:

- Rapid cron job creation
- Identity modification attempts
- Unusual command patterns in approval requests
- New channel senders from unexpected sources

### 6. Network Segmentation

Run Moltis on a private network or behind a reverse proxy with:

- IP allowlisting
- Rate limiting
- Web Application Firewall (WAF) rules

### 7. Keep Software Updated

Subscribe to security advisories and update promptly when vulnerabilities are
disclosed.

## Reporting Security Issues

Report security vulnerabilities privately to the maintainers. Do not open
public issues for security bugs.

See the repository's SECURITY.md for contact information.
