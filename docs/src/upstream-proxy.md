# Upstream Proxy

Moltis can route all outbound HTTP traffic through an upstream proxy. This is
useful when running behind a corporate firewall, in a restricted network, or
when you need to audit/filter outbound connections.

## Configuration

Add `upstream_proxy` to the top level of your `moltis.toml`:

```toml
upstream_proxy = "http://proxy.corp.example.com:8080"
```

### Supported schemes

| Scheme | Example | Notes |
|--------|---------|-------|
| `http://` | `http://proxy:8080` | HTTP CONNECT proxy (most common) |
| `https://` | `https://proxy:8443` | TLS-encrypted proxy connection |
| `socks5://` | `socks5://proxy:1080` | SOCKS5 proxy (DNS resolved locally) |
| `socks5h://` | `socks5h://proxy:1080` | SOCKS5 proxy (DNS resolved by proxy) |

### Proxy authentication

Include credentials in the URL:

```toml
upstream_proxy = "http://user:password@proxy.corp.example.com:8080"
```

### What is proxied

When `upstream_proxy` is set, the following traffic routes through the proxy:

- **LLM provider API calls** (Anthropic, OpenAI, Gemini, etc.)
- **Tool HTTP requests** (web fetch, web search, Firecrawl)
- **OAuth flows** (device auth, token exchange)
- **MCP server auth** (OAuth for remote MCP servers)
- **Channel outbound** (Slack streaming, MS Teams API calls)

Localhost and loopback addresses (`127.0.0.1`, `::1`, `localhost`) are
automatically excluded from the proxy (`no_proxy`).

### Slack caveat

Slack streaming messages (progressive edits) are proxied via reqwest.
However, regular `chat.postMessage` calls go through the `slack-morphism`
library's built-in hyper connector, which does **not** use the upstream
proxy. If you need full Slack proxy coverage, also set the `HTTPS_PROXY`
environment variable.

### Telegram caveat

Telegram uses [teloxide](https://github.com/teloxide/teloxide) which bundles
its own HTTP client (reqwest 0.11). The `upstream_proxy` config does not apply
to Telegram traffic directly. To proxy Telegram, set the standard
`HTTPS_PROXY` environment variable, which teloxide's reqwest honours:

```bash
export HTTPS_PROXY=http://proxy.corp.example.com:8080
moltis
```

Or use the `env` section in `moltis.toml`:

```toml
[env]
HTTPS_PROXY = "http://proxy.corp.example.com:8080"
```

## Environment variable fallback

When `upstream_proxy` is **not** set in `moltis.toml`, reqwest automatically
honours the standard proxy environment variables:

- `HTTP_PROXY` / `http_proxy`
- `HTTPS_PROXY` / `https_proxy`
- `ALL_PROXY` / `all_proxy`
- `NO_PROXY` / `no_proxy`

Setting `upstream_proxy` in the config takes precedence over these variables
for all traffic except Telegram (see caveat above).

## Interaction with Trusted Network

If you use both `upstream_proxy` and
[trusted network mode](trusted-network.md) (`network = "trusted"`), they
serve different purposes:

- **Trusted network proxy** is a local domain-filtering proxy for sandbox
  tool execution. It controls *which* domains tools can reach.
- **Upstream proxy** routes traffic through your corporate/network proxy to
  *reach* the internet.

When both are active, tool traffic routes through the trusted-network proxy
(which enforces domain allowlists), while provider and channel traffic routes
through the upstream proxy.
