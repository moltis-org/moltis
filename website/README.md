<p align="center">
  <a href="https://moltis.org"><img src="favicon.svg" alt="Moltis" width="100"></a>
</p>

# Moltis Website (Cloudflare Worker)

This directory contains the static site and worker for [moltis.org](https://moltis.org).
It was moved into the main `moltis` repository so website, installer, and release manifests can be updated together.

## Deploy

- Cloudflare Worker config: `website/wrangler.jsonc`
- Worker entrypoint: `website/_worker.js`
- Assets root: `website/`
- Served installer script: `website/install.sh`
- Served releases manifest: `website/releases.json`

When updating `install.sh`, keep `website/install.sh` in sync with the repo-root `install.sh`.

## Agent Install Discovery Manifests

The website publishes machine-readable install/discovery files under `/.well-known/`:

- `/.well-known/moltis-install.json` (stable root manifest)
- `/.well-known/moltis-install/channels/stable.json` (moving channel pointer)
- `/.well-known/moltis-install/releases/<version>.json` (immutable per-release manifest)

Update channel/release manifests at release time with:

```bash
cd website
npm ci
npm run install-manifest:update -- --version 0.9.11 --checksums-file /path/to/release-checksums.txt
```
