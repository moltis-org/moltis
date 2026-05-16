# Remote Access

Moltis can expose the gateway beyond `localhost` through private mesh networks
or managed HTTPS tunnels. Configure these providers from **Settings -> Remote
Access** in the web UI, or through `moltis.toml` and the CLI.

Use authentication before exposing Moltis outside your machine. Public tunnels
with `auth.disabled = true` are blocked by the setup-required page until
authentication is configured.

| Provider | Access model | Best for |
|----------|--------------|----------|
| Tailscale Serve | Private tailnet HTTPS | Personal devices and team tailnets |
| Tailscale Funnel | Public HTTPS | Public callbacks through Tailscale |
| NetBird | Private mesh access | Open source mesh VPN users |
| ngrok | Public HTTPS | Demos, tests, and webhook callbacks |
| Cloudflare Tunnel | Public HTTPS | Stable public hostnames managed by Cloudflare |

## Tailscale

Tailscale support shells out to the `tailscale` CLI and supports two modes:

- `serve`: exposes Moltis over HTTPS inside your tailnet.
- `funnel`: exposes Moltis over public HTTPS through Tailscale Funnel.

Configuration:

```toml
[tailscale]
mode = "serve" # "off" | "serve" | "funnel"
reset_on_exit = true
```

CLI:

```bash
moltis tailscale status
moltis tailscale serve --port 18789
moltis tailscale funnel --port 18789
moltis tailscale disable
```

Notes:

- Install and log in to Tailscale before enabling Serve or Funnel.
- Moltis must bind to a loopback address for Serve/Funnel.
- Funnel exposes Moltis to the public internet. Keep password/passkey
  authentication enabled.
- Passkeys are hostname-bound; changing the Tailscale hostname or public URL can
  require logging in again on the new origin.

## NetBird

NetBird support shells out to the `netbird` CLI and provides private mesh access
similar to Tailscale Serve. NetBird does not provide a Moltis-managed public
Funnel equivalent.

Configuration:

```toml
[netbird]
mode = "serve" # "off" | "serve"
```

CLI:

```bash
moltis netbird status
moltis netbird serve
moltis netbird disable
```

Notes:

- Install NetBird and connect the peer before enabling serve mode.
- Moltis keeps its main listener bound to loopback and starts a NetBird peer-IP
  forwarder to the local gateway port.
- Use Cloudflare Tunnel, ngrok, or Tailscale Funnel when you need a public HTTPS
  endpoint.

## ngrok

ngrok support uses the Rust ngrok SDK and does not require a separate `ngrok`
binary. Moltis starts an internal loopback forwarding listener and exposes it
through ngrok.

Configuration:

```toml
[ngrok]
enabled = true
authtoken = "${NGROK_AUTHTOKEN}" # or set NGROK_AUTHTOKEN
# domain = "team-gateway.ngrok.app" # optional reserved/static domain
```

Notes:

- Use a reserved domain for stable passkey behavior across restarts.
- If `authtoken` is omitted, Moltis reads `NGROK_AUTHTOKEN` from the
  environment.
- ngrok is useful for webhook callbacks, demos, and temporary public access.

## Cloudflare Tunnel

Cloudflare Tunnel support starts `cloudflared` and forwards a Cloudflare-managed
public hostname to the local Moltis gateway.

Configuration:

```toml
[cloudflare_tunnel]
enabled = true
token = "${CLOUDFLARE_TUNNEL_TOKEN}" # or set CLOUDFLARE_TUNNEL_TOKEN
hostname = "moltis.example.com"      # optional but recommended for passkeys
```

CLI:

```bash
moltis cloudflare-tunnel status
moltis cloudflare-tunnel enable --token <token> --hostname moltis.example.com
moltis cloudflare-tunnel disable
```

Notes:

- Install `cloudflared` before enabling the connector.
- If `token` is omitted, Moltis reads `CLOUDFLARE_TUNNEL_TOKEN` from the
  environment.
- Set `hostname` when you know the public route so Moltis can update WebAuthn
  passkey origins consistently.

## Choosing a Provider

Prefer private mesh access when possible:

- Use Tailscale Serve if your devices already use Tailscale.
- Use NetBird if you prefer a fully open source mesh VPN.
- Use Tailscale Funnel, ngrok, or Cloudflare Tunnel only when external services
  need a public callback URL or users outside your mesh need access.

For cloud-hosted deployments, see [Cloud Deploy](cloud-deploy.md). For all
config keys and defaults, see [Configuration Reference](configuration-reference.md).
