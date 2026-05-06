# Plan: Publish Moltis to AUR

**Context:** GitHub discussion [#679](https://github.com/moltis-org/moltis/discussions/679) — a user built a working PKGBUILD because the AUR package was outdated/missing. We already build `.pkg.tar.zst` in CI (`release.yml` build-arch job) for x86_64 and aarch64.

**Goal:** Make `yay -S moltis-bin` work for Arch users, and improve the in-repo PKGBUILD.

---

## Phase 1: AUR account setup (manual, needs Arch box)

The AUR registration CAPTCHA requires running a `pacman` command, so you need an Arch system (VM, container, or bare metal).

1. Register at https://aur.archlinux.org/register
2. Generate a dedicated SSH keypair for AUR:
   ```bash
   ssh-keygen -t ed25519 -f ~/.ssh/aur -C "aur@moltis.org"
   ```
3. Upload the public key to AUR profile under "My Account > SSH Public Key"
4. Add to `~/.ssh/config`:
   ```
   Host aur.archlinux.org
     IdentityFile ~/.ssh/aur
     User aur
   ```

## Phase 2: Create `moltis-bin` AUR package

This is a binary package that downloads the pre-built `.pkg.tar.zst` from GitHub Releases. Lowest maintenance.

1. Clone the empty AUR namespace:
   ```bash
   git -c init.defaultBranch=master clone ssh://aur@aur.archlinux.org/moltis-bin.git
   cd moltis-bin
   ```

2. Create a PKGBUILD like this (adapt version/checksums from latest release):
   ```bash
   # Maintainer: Fabien Penso <fabien@penso.info>
   pkgname=moltis-bin
   pkgver=20260413.06
   pkgrel=1
   pkgdesc="Personal AI gateway inspired by OpenClaw"
   arch=('x86_64' 'aarch64')
   url="https://www.moltis.org/"
   license=('MIT')
   depends=('gcc-libs')
   optdepends=(
     'docker: sandboxed command execution'
     'podman: sandboxed command execution (alternative to docker)'
     'nodejs: stdio-based MCP servers'
     'chromium: browser automation feature'
     'tmux: terminal sessions in web UI'
   )
   provides=('moltis')
   conflicts=('moltis' 'moltis-git')

   _gh="https://github.com/moltis-org/moltis/releases/download"
   source_x86_64=("${_gh}/${pkgver}/moltis-${pkgver}-1-x86_64.pkg.tar.zst")
   source_aarch64=("${_gh}/${pkgver}/moltis-${pkgver}-1-aarch64.pkg.tar.zst")
   sha256sums_x86_64=('FILL_FROM_RELEASE')
   sha256sums_aarch64=('FILL_FROM_RELEASE')
   noextract=("${source_x86_64[@]##*/}" "${source_aarch64[@]##*/}")

   package() {
     local _pkg="moltis-${pkgver}-1-${CARCH}.pkg.tar.zst"
     bsdtar -xf "$srcdir/$_pkg" -C "$pkgdir" usr/

     install -Dm644 /dev/null "$pkgdir/usr/share/licenses/$pkgname/LICENSE.md"
     # TODO: extract LICENSE from the archive or download separately
   }
   ```

   > **Note:** The exact archive layout and naming must match what `release.yml` produces.
   > Check a real release to confirm paths. The CI-built `.pkg.tar.zst` already contains
   > `usr/bin/moltis`, `usr/share/moltis/web/`, `usr/share/moltis/wasm/` — so extracting
   > `usr/` into `$pkgdir` should be sufficient.

3. Generate .SRCINFO and push:
   ```bash
   makepkg --printsrcinfo > .SRCINFO
   git add PKGBUILD .SRCINFO
   git commit -m "Initial upload: moltis-bin ${pkgver}"
   git push
   ```

## Phase 3: Automate AUR updates in CI

Add a job to `release.yml` that runs after the GitHub Release is published, updating the AUR package automatically on each release.

1. Generate a **dedicated** SSH keypair for CI (not your personal one):
   ```bash
   ssh-keygen -t ed25519 -f aur-ci-key -C "ci@moltis.org" -N ""
   ```
2. Upload the public key to the AUR account
3. Add GitHub Actions secrets:
   - `AUR_SSH_PRIVATE_KEY` — the private key
   - `AUR_USERNAME` — AUR username
   - `AUR_EMAIL` — email for git commits

4. Add a workflow job (after `publish-release`):
   ```yaml
   update-aur:
     needs: [publish-release]
     runs-on: ubuntu-latest
     if: startsWith(github.ref, 'refs/tags/')
     steps:
       - uses: actions/checkout@v4

       - name: Get release checksums
         id: checksums
         run: |
           VERSION="${GITHUB_REF_NAME}"
           for ARCH in x86_64 aarch64; do
             SHA=$(curl -sL "https://github.com/moltis-org/moltis/releases/download/${VERSION}/moltis-${VERSION}-1-${ARCH}.pkg.tar.zst.sha256" | awk '{print $1}')
             echo "sha256_${ARCH}=${SHA}" >> "$GITHUB_OUTPUT"
           done
           echo "version=${VERSION}" >> "$GITHUB_OUTPUT"

       - name: Generate PKGBUILD
         run: |
           # Use a template script or sed to fill in version + checksums
           ./scripts/generate-aur-pkgbuild.sh \
             "${{ steps.checksums.outputs.version }}" \
             "${{ steps.checksums.outputs.sha256_x86_64 }}" \
             "${{ steps.checksums.outputs.sha256_aarch64 }}"

       - name: Publish to AUR
         uses: KSXGitHub/github-actions-deploy-aur@v8
         with:
           pkgname: moltis-bin
           pkgbuild: ./pkg/arch/aur/PKGBUILD
           commit_username: ${{ secrets.AUR_USERNAME }}
           commit_email: ${{ secrets.AUR_EMAIL }}
           ssh_private_key: ${{ secrets.AUR_SSH_PRIVATE_KEY }}
           commit_message: "Update to ${{ steps.checksums.outputs.version }}"
   ```

5. Create `scripts/generate-aur-pkgbuild.sh` that templates the PKGBUILD with version and checksums.

## Phase 4: Improve in-repo `pkg/arch/PKGBUILD`

Update `pkg/arch/PKGBUILD` to be a proper source-build PKGBUILD (useful for contributors, and as the basis for a future `moltis-git` AUR package). Incorporate improvements from discussion #679:

- Add `optdepends` (docker, podman, nodejs, chromium, tmux)
- Add `pkgver()` function for date-based tags
- Add Tailwind CSS build step
- Add WASM component build step
- Pin nightly toolchain version (must match `rust-toolchain.toml`)
- Install web assets and WASM to `/usr/share/moltis/`

## Phase 5 (optional): Publish `moltis-git` to AUR

If there's demand, publish a `-git` variant based on the improved in-repo PKGBUILD. This builds from latest `main` and is for power users who want bleeding-edge. Lower priority since `-bin` covers most users.

---

## Checklist

- [ ] Register AUR account (needs Arch box)
- [ ] Create and push `moltis-bin` PKGBUILD to AUR
- [ ] Verify `yay -S moltis-bin` works
- [ ] Add CI automation for AUR updates (`update-aur` job + `generate-aur-pkgbuild.sh`)
- [ ] Store AUR SSH key + credentials as GitHub Actions secrets
- [ ] Update `pkg/arch/PKGBUILD` with improvements from discussion #679
- [ ] Reply to discussion #679 thanking contributor, linking to AUR package
- [ ] (Optional) Publish `moltis-git` AUR package
- [ ] Update `docs/src/installation.md` to mention AUR

## References

- Discussion #679: https://github.com/moltis-org/moltis/discussions/679
- AUR submission guidelines: https://wiki.archlinux.org/title/AUR_submission_guidelines
- CI deploy action: https://github.com/KSXGitHub/github-actions-deploy-aur
- Existing CI arch job: `.github/workflows/release.yml` lines 454-561
- Existing in-repo PKGBUILD: `pkg/arch/PKGBUILD`
