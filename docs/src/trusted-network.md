# Trusted Network Mode

Trusted network mode gives sandbox containers filtered internet access
through a local HTTP proxy, so LLM-generated code can reach approved
domains (package registries, APIs) while everything else is blocked.

## Why

By default, sandbox containers run with `no_network = true` — full network
isolation. This is the safest option but breaks any task that needs to
install packages, fetch data, or call an API.

Setting `no_network = false` opens the network entirely, which defeats much
of the sandbox's purpose: a malicious command could exfiltrate data or
download additional payloads.

Trusted network mode sits between these extremes. It routes all outbound
traffic through a filtering proxy that only allows connections to domains
you explicitly trust.

```
┌──────────────┐      CONNECT        ┌────────────┐       ┌──────────────┐
│   Sandbox    │ ──────────────────▶  │   Proxy    │ ────▶ │  github.com  │ ✓
│  Container   │                      │ :18791     │       └──────────────┘
│              │      CONNECT         │            │       ┌──────────────┐
│  HTTP_PROXY  │ ──────────────────▶  │  Domain    │ ──✗─▶ │  evil.com    │ ✗
│  = proxy:18791                      │  Filter    │       └──────────────┘
└──────────────┘                      └────────────┘
```

## Configuration

Enable trusted network mode in `moltis.toml`:

```toml
[tools.exec.sandbox]
network = "trusted"
trusted_domains = [
  # Package registries
  "registry.npmjs.org",
  "pypi.org",
  "files.pythonhosted.org",
  "crates.io",
  "static.crates.io",

  # Git hosting
  "github.com",
  "gitlab.com",

  # Common APIs
  "api.openai.com",
  "httpbin.org",
]
```

### Network policies

| Policy | Behavior | Use case |
|--------|----------|----------|
| *(empty / default)* | Uses legacy `no_network` flag | Backward compatible |
| `blocked` | No network at all | Maximum isolation |
| `trusted` | Proxy-filtered by domain allowlist | Development tasks |
| `open` | Unrestricted network | Fully trusted workloads |

```admonish warning title="open mode"
`network = "open"` disables all network filtering. Only use this when you
fully trust the LLM output or are running on a throw-away machine.
```

### Domain patterns

The `trusted_domains` list supports three pattern types:

| Pattern | Example | Matches |
|---------|---------|---------|
| Exact | `github.com` | Only `github.com` |
| Wildcard subdomain | `*.npmjs.org` | `registry.npmjs.org`, `www.npmjs.org`, etc. |
| Full wildcard | `*` | Everything (equivalent to `open` mode) |

## How the proxy works

When `network = "trusted"`, the gateway starts an HTTP CONNECT proxy on
`127.0.0.1:18791` at startup. Sandbox containers are configured to route
traffic through this proxy via `HTTP_PROXY` / `HTTPS_PROXY` environment
variables.

For each connection the proxy:

1. Extracts the target domain from the `CONNECT` request (or `Host` header
   for plain HTTP).
2. Checks the domain against the allowlist in `DomainApprovalManager`.
3. If **allowed** — opens a TCP tunnel to the target and relays data
   bidirectionally.
4. If **denied** — returns `403 Forbidden` and logs the attempt.

Both allowed and denied requests are recorded in the network audit log.

## Network Audit

Every proxied request is logged to an in-memory ring buffer (2048 entries)
and persisted to `~/.moltis/network-audit.jsonl`. The audit log is
accessible from:

- **Settings > Network Audit** in the web UI
- **RPC methods**: `network.audit.list`, `network.audit.tail`,
  `network.audit.stats`
- **WebSocket events**: `network.audit.entry` streams entries in real time

### Audit entry fields

| Field | Description |
|-------|-------------|
| `timestamp` | ISO 8601 timestamp (RFC 3339) |
| `domain` | Target domain name |
| `port` | Target port number |
| `protocol` | `https`, `http`, or `connect` |
| `action` | `allowed` or `denied` |
| `source` | Why the decision was made (`config_allowlist`, `session`, etc.) |

### Filtering the audit log

The web UI provides real-time filtering by:

- **Domain** — free-text search across domain names
- **Protocol** — filter by `https`, `http`, or `connect`
- **Action** — show only `allowed` or `denied` entries

The RPC methods accept the same filter parameters:

```json
{
  "method": "network.audit.list",
  "params": {
    "domain": "github",
    "protocol": "https",
    "action": "denied",
    "limit": 100
  }
}
```

### Audit stats

`network.audit.stats` returns aggregate counts:

```json
{
  "total": 847,
  "allowed": 812,
  "denied": 35,
  "by_domain": [
    { "domain": "registry.npmjs.org", "count": 423 },
    { "domain": "github.com", "count": 312 }
  ]
}
```

## Recommended domain lists

### Node.js / npm

```toml
trusted_domains = [
  "registry.npmjs.org",
  "*.npmjs.org",
]
```

### Python / pip

```toml
trusted_domains = [
  "pypi.org",
  "files.pythonhosted.org",
]
```

### Rust / cargo

```toml
trusted_domains = [
  "crates.io",
  "static.crates.io",
  "index.crates.io",
]
```

### Git operations

```toml
trusted_domains = [
  "github.com",
  "gitlab.com",
  "bitbucket.org",
]
```

```admonish tip title="Start narrow, widen as needed"
Begin with only the registries your project uses. The audit log will show
denied domains — add them to the allowlist only if they are legitimate.
```

## Relationship to other settings

- `no_network = true` (legacy) is equivalent to `network = "blocked"`.
  When `network` is set, it takes precedence over `no_network`.
- `mode = "off"` disables sandboxing entirely — network policy has no
  effect because commands run directly on the host.
- Resource limits (`memory_limit`, `cpu_quota`, `pids_max`) apply
  independently of the network policy.

## Troubleshooting

**Proxy not starting**: Check the gateway startup log for
`trusted-network proxy started on port 18791`. If missing, verify
`network = "trusted"` is set in `moltis.toml`.

**Connections timing out**: Some tools don't respect `HTTP_PROXY`. Verify
the tool uses the proxy by checking the audit log — if no entries appear
for the domain, the tool is bypassing the proxy.

**Too many denied entries**: Review the audit log to identify legitimate
domains being blocked, then add them to `trusted_domains`.
