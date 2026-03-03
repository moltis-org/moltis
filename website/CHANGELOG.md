# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Machine-readable install discovery files under `/.well-known/` for agent-driven Moltis setup.
- Stable channel pointer at `/.well-known/moltis-install/channels/stable.json` and immutable per-release manifests at `/.well-known/moltis-install/releases/<version>.json`.
- JSON schemas for root, channel, and release install manifests.
- Agent discovery docs: `/.well-known/agent-card.json`, `/.well-known/api-catalog`, and `/llms.txt`.
- Release-manifest generator script at `scripts/generate-install-release-manifest.mjs`.
- NPM helper script `npm run install-manifest:update` for rotating release/channel manifests each release.

### Changed

- `README.md` now documents agent install discovery manifests and the release update command.

### Deprecated

### Removed

### Fixed

### Security
