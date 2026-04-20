# Moltis Deployment Templates

This directory contains templates for deploying Moltis on a VPS or bare-metal server.

## Files

| File | Purpose |
|------|---------|
| `docker-compose.yml` | Docker Compose for VPS deployment |
| `moltis.service` | systemd unit file for bare-metal installs |

## Docker Compose (recommended)

```bash
cd deploy
export MOLTIS_PASSWORD="your-secure-password"
docker compose up -d
```

Open `https://<your-server-ip>:13131` and configure your LLM provider.

## Systemd (bare-metal)

```bash
# Create user and directories
sudo useradd -r -s /usr/sbin/nologin moltis
sudo mkdir -p /var/lib/moltis /etc/moltis
sudo chown moltis:moltis /var/lib/moltis /etc/moltis

# Install the binary
sudo cp moltis /usr/local/bin/moltis

# Install and start the service
sudo cp deploy/moltis.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now moltis
```

See [docs.moltis.org/deploy-vps](https://docs.moltis.org/deploy-vps.html) for
a complete walkthrough.
