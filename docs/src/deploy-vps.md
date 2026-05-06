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

Docker is the fastest path. It handles TLS certificates, sandbox isolation,
and upgrades via image pulls.

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

Open `https://<your-server-ip>:13131` in your browser. You'll see a TLS
warning because Moltis generates a self-signed certificate. Accept it or
download the CA from `http://<your-server-ip>:13132`.

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

Open ports 13131 (HTTPS gateway) and 13132 (HTTP CA download) in your
firewall. If you put Moltis behind a reverse proxy (nginx, Caddy), you only
need to expose the proxy port and can use `--no-tls` on Moltis.

```bash
# UFW example
sudo ufw allow 13131/tcp
sudo ufw allow 13132/tcp
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
