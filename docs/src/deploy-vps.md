# Deploy Moltis on a VPS

Run your own AI agent on a $5/month VPS. This guide covers provisioning,
installation, and connecting channels (Telegram, Discord, etc.) so you can
talk to your agent from anywhere.

## Prerequisites

- A VPS with at least 1 GB RAM and 10 GB disk (any provider: Hetzner,
  DigitalOcean, Linode, Vultr, etc.)
- SSH access to the server
- An API key from at least one LLM provider (Anthropic, OpenAI, etc.)

## Option A: Docker (recommended)

Docker is the fastest path. It handles sandbox isolation and upgrades via image
pulls. For browser-trusted TLS on a VPS, put Moltis behind a reverse proxy
such as Caddy, nginx, or Traefik and let the proxy manage public certificates.

### 1. Install Docker

```bash
curl -fsSL https://get.docker.com | sh
sudo usermod -aG docker $USER
# Log out and back in for group membership to take effect
```

### 2. Deploy Moltis

```bash
mkdir -p ~/moltis && cd ~/moltis
curl -fsSL https://raw.githubusercontent.com/moltis-org/moltis/main/deploy/docker-compose.yml -o docker-compose.yml

# Set your password
export MOLTIS_PASSWORD="your-secure-password"

# Start
docker compose up -d
```

### 3. Access the web UI

For a production VPS, use a domain name and terminate TLS at a reverse proxy:

```yaml
services:
  moltis:
    image: ghcr.io/moltis-org/moltis:latest
    command: ["--bind", "0.0.0.0", "--port", "13131", "--no-tls"]
    environment:
      - MOLTIS_BEHIND_PROXY=true
      - MOLTIS_EXTERNAL_URL=https://chat.example.com
```

Point your proxy at `http://<moltis-host>:13131`, then open your public URL,
for example `https://chat.example.com`.

Moltis can also auto-generate a local CA and server certificate, but that mode
is meant for local development or private networks. Trusting the generated CA
only makes the issuer trusted; the browser still requires the certificate's
Subject Alternative Name (SAN) to match the exact hostname or IP you open.
An IP-address URL such as `https://<your-server-ip>:13131` only works if the
certificate contains that IP address as an IP SAN, and normal public TLS setups
are domain-name based. Inside Docker, Moltis usually cannot know your VPS public
IP or provider domain, so direct IP access may fail with a certificate name
mismatch even after importing the CA. To use direct IP access with the
auto-generated certificate, set the public IP explicitly:

```toml
[tls]
public_ip = "203.0.113.10"
```

Then restart Moltis so it regenerates the server certificate and import the
generated CA from `http://<your-server-ip>:13132/certs/ca.pem`. If you want
Moltis to serve HTTPS directly on a public domain, configure `tls.cert_path` and
`tls.key_path` with a certificate issued for that domain.

Log in with the password you set, then configure your LLM provider in
Settings.

## Option B: Binary + systemd

For servers without Docker, install the binary directly.

### 1. Download the binary

```bash
# Replace VERSION with the latest release (e.g. 20260420.01)
VERSION=$(curl -s https://api.github.com/repos/moltis-org/moltis/releases/latest | grep tag_name | cut -d '"' -f 4)
ARCH=$(uname -m | sed 's/x86_64/x86_64/;s/aarch64/aarch64/')

curl -fsSL "https://github.com/moltis-org/moltis/releases/download/${VERSION}/moltis-${VERSION}-linux-${ARCH}.tar.gz" | sudo tar xz -C /usr/local/bin
```

### 2. Create user and directories

```bash
sudo useradd -r -s /usr/sbin/nologin moltis
sudo mkdir -p /var/lib/moltis /etc/moltis
sudo chown moltis:moltis /var/lib/moltis /etc/moltis
```

### 3. Install the systemd service

```bash
sudo curl -fsSL https://raw.githubusercontent.com/moltis-org/moltis/main/deploy/moltis.service -o /etc/systemd/system/moltis.service
sudo systemctl daemon-reload
sudo systemctl enable --now moltis
```

### 4. Set your password

```bash
sudo -u moltis MOLTIS_DATA_DIR=/var/lib/moltis MOLTIS_CONFIG_DIR=/etc/moltis moltis auth reset-password
```

### 5. Check status

```bash
sudo systemctl status moltis
sudo journalctl -u moltis -f
```

## Connecting channels

Once Moltis is running, add messaging channels from Settings > Channels in
the web UI. Each channel has its own setup flow:

| Channel | What you need |
|---------|--------------|
| Telegram | Bot token from [@BotFather](https://t.me/BotFather) |
| Discord | Bot token from the [Developer Portal](https://discord.com/developers) |
| Slack | Bot + App tokens from [api.slack.com](https://api.slack.com/apps) |
| Matrix | Homeserver URL + credentials |
| Nostr | Secret key (nsec) + relay URLs |

See the individual [channel docs](channels.md) for detailed setup
instructions.

## Firewall

If you use the recommended reverse proxy setup, expose only the proxy ports
(`80` and `443`) publicly and keep Moltis' `13131` port private to the host or
container network.

Only expose `13131` directly if you intentionally serve Moltis without a proxy.
Only expose `13132` if you intentionally use Moltis' local CA download endpoint.

```bash
# UFW example for the recommended reverse proxy setup
sudo ufw allow 80/tcp
sudo ufw allow 443/tcp
```

## Upgrades

**Docker:** `docker compose pull && docker compose up -d`

**Binary:** Download the new release binary and restart the service:
```bash
sudo systemctl stop moltis
# Download new binary (same curl as step 1)
sudo systemctl start moltis
```

## Resource requirements

| Workload | RAM | CPU | Disk |
|----------|-----|-----|------|
| Chat only (no sandbox) | 512 MB | 1 vCPU | 5 GB |
| Chat + sandbox | 1 GB | 1 vCPU | 10 GB |
| Chat + sandbox + local LLM | 4+ GB | 2+ vCPU | 20+ GB |

LLM inference happens on the provider's API servers, so even a $5 VPS handles
chat workloads with external providers.
