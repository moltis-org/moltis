# Installation

Moltis is distributed as a single self-contained binary. Choose the installation method that works best for your setup.

## Quick Install (Recommended)

The fastest way to get started on macOS or Linux:

```bash
curl -fsSL https://www.moltis.org/install.sh | sh
```

This downloads the latest release for your platform and installs it to `~/.local/bin`.

## Package Managers

### Homebrew (macOS / Linux)

```bash
brew install moltis-org/tap/moltis
```

## Linux Packages

Package filenames are versioned on every release. Use the installer script below instead of hardcoding GitHub release asset names.

### Debian / Ubuntu (.deb)

```bash
# Install the latest .deb package
curl -fsSL https://www.moltis.org/install.sh | sh -s -- --method=deb
```

### Fedora / RHEL (.rpm)

```bash
# Install the latest .rpm package
curl -fsSL https://www.moltis.org/install.sh | sh -s -- --method=rpm
```

### Arch Linux (.pkg.tar.zst)

```bash
# Install the latest package
curl -fsSL https://www.moltis.org/install.sh | sh -s -- --method=arch
```

### Snap

```bash
sudo snap install moltis
```

### AppImage

```bash
# Install the latest AppImage
curl -fsSL https://www.moltis.org/install.sh | sh -s -- --method=appimage
```

## Docker

Multi-architecture images (amd64/arm64) are published to GitHub Container Registry:

```bash
docker pull ghcr.io/moltis-org/moltis:latest
```

See [Docker Deployment](docker.md) for full instructions on running Moltis in a container.

## Build from Source

### Prerequisites

- Rust 1.91 or later
- A C compiler (for some dependencies)
- [just](https://github.com/casey/just) (command runner)
- Node.js (for building Tailwind CSS)

### Clone and Build

```bash
git clone https://github.com/moltis-org/moltis.git
cd moltis
just build-css           # Build Tailwind CSS for the web UI
just build-release       # Build in release mode
```

For a full release build including WASM sandbox tools:

```bash
just build-release-with-wasm
```

The binary will be at `target/release/moltis`.

### Install via Cargo

```bash
cargo install moltis --git https://github.com/moltis-org/moltis
```

## First Run

After installation, start Moltis:

```bash
moltis
```

On first launch:

1. Open `http://localhost:<port>` in your browser (the port is shown in the terminal output)
2. Configure your LLM provider (API key)
3. Start chatting!

```admonish tip
Moltis picks a random available port on first install to avoid conflicts. The port is saved in your config and reused on subsequent runs.
```

```admonish note
Authentication is only required when accessing Moltis from a non-localhost address (e.g., over the network). When this happens, a one-time setup code is printed to the terminal for initial authentication setup.
```

## Verify Installation

```bash
moltis --version
```

## Updating

### Homebrew

```bash
brew upgrade moltis
```

### From Source

```bash
cd moltis
git pull
just build-css
just build-release
```

## Uninstalling

### Homebrew

```bash
brew uninstall moltis
```

### Remove Data

Moltis stores data in two directories:

```bash
# Configuration
rm -rf ~/.config/moltis

# Data (sessions, databases, memory)
rm -rf ~/.moltis
```

```admonish warning
Removing these directories deletes all your conversations, memory, and settings permanently.
```
