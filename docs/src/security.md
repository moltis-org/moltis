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

### Built-in Dangerous Command Blocklist

Even with `approval_mode = "never"` or `security_level = "full"`, Moltis
maintains a safety floor: a hardcoded set of regex patterns for the most
critical destructive commands (e.g. `rm -rf /`, `git reset --hard`,
`DROP TABLE`, `mkfs`, `terraform destroy`). Matching commands always require
approval regardless of configuration.

Users can override specific patterns by adding matching entries to their
`allowlist` in `moltis.toml`. The blocklist only applies to host execution;
sandboxed commands are already isolated.

### Destructive Command Guard (dcg)

For broader coverage beyond the built-in blocklist, install the
[Destructive Command Guard](https://github.com/Dicklesworthstone/destructive_command_guard)
(dcg) as a hook. dcg adds 49+ pattern categories including heredoc/inline-script
scanning, database, cloud, and infrastructure patterns.

See [Hooks: Destructive Command Guard](hooks.md#recommended-destructive-command-guard-dcg)
for setup instructions.

dcg complements (does not replace) sandbox isolation and the approval system.

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

Sandbox containers have no network access by default (`no_network = true`).

For tasks that need internet access, [trusted network mode](trusted-network.md)
provides a proxy-filtered allowlist — only connections to explicitly approved
domains are permitted. All requests (allowed and denied) are recorded in the
network audit log for review.

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

## Encryption at Rest

Sensitive data in the SQLite database (environment variables containing
API keys, tokens, etc.) is encrypted at rest using XChaCha20-Poly1305.
The encryption key is derived from the user's password via Argon2id.

The vault initializes when a first password is set (during setup or later
in Settings > Authentication), unseals automatically on login, and
re-seals on server restart. A recovery key is provided at initialization
for emergency access.

When the vault is sealed, a middleware layer blocks API requests with
`423 Locked` to prevent serving stale data.

For full details on the key hierarchy, vault states, API endpoints, and
cryptographic parameters, see [Encryption at Rest (Vault)](vault.md).

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

To allow access to trusted private networks (e.g. Docker sibling containers),
add their CIDR ranges to `ssrf_allowlist`:

```toml
[tools.web.fetch]
ssrf_allowlist = ["172.22.0.0/16"]
```

**Warning:** Only add networks you trust. The allowlist bypasses SSRF protection
for the listed ranges. Never add cloud metadata ranges (`169.254.169.254/32`)
unless you understand the risk.

## Authentication

Moltis uses a unified auth gate that applies a single `check_auth()`
function to every request. This prevents split-brain bugs where different
code paths disagree on auth status.

For full details — including the decision matrix, credential types, API
key scopes, session management endpoints, and WebSocket auth — see the
dedicated [Authentication](authentication.md) page.

### Three-Tier Model (summary)

| Tier | Condition | Behaviour |
|------|-----------|-----------|
| **1** | Password/passkey is configured | Auth **always** required (any IP) |
| **2** | No credentials + direct local connection | Full access (dev convenience) |
| **3** | No credentials + remote/proxied connection | Onboarding only (setup code required) |

## HTTP Endpoint Throttling

Moltis includes built-in per-IP endpoint throttling to reduce brute force
attempts and traffic spikes, but only when auth is required for the current
request.

Throttling is bypassed when a request is already authenticated, when auth is
explicitly disabled, or when setup is incomplete and local Tier-2 access is
allowed.

### Default Limits

| Scope | Default |
|------|---------|
| `POST /api/auth/login` | 5 requests per 60 seconds |
| Other `/api/auth/*` | 120 requests per 60 seconds |
| Other `/api/*` | 180 requests per 60 seconds |
| `/ws/chat` upgrade | 30 requests per 60 seconds |

### When Limits Are Hit

- API endpoints return `429 Too Many Requests`
- Responses include `Retry-After`
- JSON responses include `retry_after_seconds`

### Reverse Proxy Behavior

When `MOLTIS_BEHIND_PROXY=true`, throttling is keyed by forwarded client IP
headers (`X-Forwarded-For`, `X-Real-IP`, `CF-Connecting-IP`) instead of the
direct socket address.

### Production Guidance

Built-in throttling is the first layer. For internet-facing deployments, add
edge rate limits at your reverse proxy or WAF as a second layer (IP reputation,
burst controls, geo rules, bot filtering).

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

3. **WebSocket proxying** must preserve browser origin host info
   (`Host`, or `X-Forwarded-Host` if `Host` is rewritten). Moltis
   validates same-origin on WebSocket upgrades to prevent cross-site
   WebSocket hijacking (CSWSH).

4. **TLS termination** should happen at the proxy. Run Moltis with
   `--no-tls` (or `MOLTIS_NO_TLS=true`) in this mode.

   If your browser is being redirected to `https://<domain>:13131`,
   Moltis TLS is still enabled while your proxy upstream is plain HTTP.

5. **Advanced TLS upstream mode** (optional): if your proxy connects to
   Moltis using HTTPS upstream (or TCP TLS passthrough), you may keep
   Moltis TLS enabled. Set `MOLTIS_ALLOW_TLS_BEHIND_PROXY=true` to
   acknowledge this non-default setup.

### Nginx Proxy Manager (known-good headers)

If WebSockets fail behind NPM while HTTP works, ensure:

- Moltis runs with `MOLTIS_BEHIND_PROXY=true`
- For standard edge TLS termination, Moltis runs with `--no-tls`
- NPM preserves browser host/origin context

Use this in NPM's **Advanced** field:

```nginx
proxy_set_header Host $host;
proxy_set_header X-Forwarded-Host $host;
proxy_set_header X-Forwarded-Proto $scheme;
proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
proxy_set_header Upgrade $http_upgrade;
proxy_set_header Connection "upgrade";
```

Upstream scheme guidance:

- **Edge TLS termination (most setups)**: proxy to `http://<moltis-host>:13131`
  with Moltis started using `--no-tls`
- **HTTPS upstream / TLS passthrough**: proxy to `https://<moltis-host>:13131`
  and set `MOLTIS_ALLOW_TLS_BEHIND_PROXY=true`

### Passkeys Behind Proxies (Host Changes)

WebAuthn passkeys are bound to an RP ID (domain identity), not just the
server process. In practice:

- If users move from one hostname to another, old passkeys for the old host
  will not authenticate on the new host.
- If a proxy rewrites `Host` and does not preserve browser host context,
  passkey routes can fail with "no passkey config for this hostname".

For stable proxy deployments, set explicit WebAuthn identity to your public
domain:

```bash
MOLTIS_BEHIND_PROXY=true
MOLTIS_NO_TLS=true
MOLTIS_WEBAUTHN_RP_ID=chat.example.com
MOLTIS_WEBAUTHN_ORIGIN=https://chat.example.com
```

Migration guidance when changing host/domain:

1. Keep password login enabled during migration.
2. Deploy with the new `MOLTIS_WEBAUTHN_RP_ID`/`MOLTIS_WEBAUTHN_ORIGIN`.
3. Ask users to register a new passkey on the new host.
4. Remove old passkeys after new-host login is confirmed.

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
