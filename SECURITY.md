# Security Policy

## Reporting a Vulnerability

If you discover a security vulnerability in Moltis, please report it responsibly.

### Preferred Methods

1. **GitHub Security Advisories** (recommended): Use [GitHub's private vulnerability reporting](https://github.com/moltis-org/moltis/security/advisories/new) to report issues confidentially.

2. **Email**: Send details to [moltis AT pen DOT so](mailto:moltis AT pen DOT so)

### What to Include

- Description of the vulnerability
- Steps to reproduce
- Potential impact
- Any suggested fixes (optional)

### Response Timeline

- **Acknowledgment**: Within 48 hours
- **Initial assessment**: Within 7 days
- **Fix timeline**: Depends on severity, but we aim to address critical issues as quickly as possible

### Scope

This policy applies to the Moltis codebase. Third-party dependencies should be reported to their respective maintainers.

## Supported Versions

Security updates are provided for the latest release only.

## Verifying Release Signatures

All release artifacts are signed using [Sigstore](https://sigstore.dev) keyless
signing. This provides cryptographic proof that artifacts were built by our
GitHub Actions workflow, not tampered with after the fact.

### Install cosign

```bash
# macOS
brew install cosign

# Linux (or download from https://github.com/sigstore/cosign/releases)
go install github.com/sigstore/cosign/v2/cmd/cosign@latest
```

### Verify a binary/package

Each release artifact has three companion files:
- `.sha256` / `.sha512` — checksums
- `.sig` — Sigstore signature
- `.crt` — Signing certificate

```bash
# Download the artifact and its signature files
curl -LO https://github.com/penso/moltis/releases/download/v0.1.0/moltis-0.1.0-x86_64-unknown-linux-gnu.tar.gz
curl -LO https://github.com/penso/moltis/releases/download/v0.1.0/moltis-0.1.0-x86_64-unknown-linux-gnu.tar.gz.sig
curl -LO https://github.com/penso/moltis/releases/download/v0.1.0/moltis-0.1.0-x86_64-unknown-linux-gnu.tar.gz.crt

# Verify the signature
cosign verify-blob \
  --signature moltis-0.1.0-x86_64-unknown-linux-gnu.tar.gz.sig \
  --certificate moltis-0.1.0-x86_64-unknown-linux-gnu.tar.gz.crt \
  --certificate-identity-regexp="https://github.com/penso/moltis/*" \
  --certificate-oidc-issuer="https://token.actions.githubusercontent.com" \
  moltis-0.1.0-x86_64-unknown-linux-gnu.tar.gz

# Expected output: Verified OK
```

### Verify a Docker image

```bash
# Verify the image signature
cosign verify \
  --certificate-identity-regexp="https://github.com/penso/moltis/*" \
  --certificate-oidc-issuer="https://token.actions.githubusercontent.com" \
  ghcr.io/penso/moltis:latest

# View the SBOM attached to the image
cosign download sbom ghcr.io/penso/moltis:latest

# View provenance attestation
cosign verify-attestation \
  --type slsaprovenance \
  --certificate-identity-regexp="https://github.com/penso/moltis/*" \
  --certificate-oidc-issuer="https://token.actions.githubusercontent.com" \
  ghcr.io/penso/moltis:latest
```

### What the signature proves

- The artifact was built by GitHub Actions in the `penso/moltis` repository
- The build was triggered by a specific commit (visible in the certificate)
- The artifact has not been modified since signing
- No private keys are involved — signatures use GitHub's OIDC identity

### Transparency log

All signatures are recorded in Sigstore's public transparency log (Rekor).
You can search for moltis signatures at: https://search.sigstore.dev

## Signed Commits

All commits to this repository must be cryptographically signed. This ensures
that commits actually come from the claimed author and haven't been tampered
with.

### Setting up commit signing

**Option 1: SSH signing (recommended)**

If you already have an SSH key, this is the easiest option:

```bash
# Use your existing SSH key for signing
git config --global gpg.format ssh
git config --global user.signingkey ~/.ssh/id_ed25519.pub
git config --global commit.gpgsign true

# Add your SSH signing key to GitHub:
# Settings → SSH and GPG keys → New SSH key → Key type: Signing Key
```

**Option 2: GPG signing**

```bash
# Generate a GPG key if you don't have one
gpg --full-generate-key

# Get your key ID
gpg --list-secret-keys --keyid-format=long
# Look for: sec rsa4096/XXXXXXXXXXXXXXXX

# Configure git
git config --global user.signingkey XXXXXXXXXXXXXXXX
git config --global commit.gpgsign true

# Add your GPG key to GitHub:
# gpg --armor --export XXXXXXXXXXXXXXXX
# Settings → SSH and GPG keys → New GPG key
```

**Option 3: GPG with YubiKey**

If you have a YubiKey with GPG keys:

```bash
# Your key is already on the YubiKey, just configure git
git config --global user.signingkey XXXXXXXXXXXXXXXX
git config --global commit.gpgsign true
```

### Verifying your setup

```bash
# Make a test commit
echo "test" >> test.txt && git add test.txt && git commit -m "test signed commit"

# Verify it's signed
git log --show-signature -1

# Clean up
git reset --hard HEAD~1
```

### Troubleshooting

If commits show as "Unverified" on GitHub:
1. Ensure your signing key is added to your GitHub account
2. Your commit email must match a verified email on your GitHub account
3. For GPG: the key's email must match your commit email
