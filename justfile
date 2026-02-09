# Default recipe (runs when just is called without arguments)
default:
    @just --list

# Format Rust code
format:
    cargo +nightly fmt --all

# Check if code is formatted
format-check:
    cargo +nightly fmt -- --check

# Lint Rust code using clippy
lint:
    cargo clippy --bins --tests --benches --examples --all-features --all-targets -- -D warnings

# Build the project
build:
    cargo build

# Build in release mode
build-release:
    cargo build --release

# Run local dev server with workspace-local config/data dirs.
dev-server:
    MOLTIS_CONFIG_DIR=.moltis/config MOLTIS_DATA_DIR=.moltis/ cargo run --bin moltis

# Build Debian package for the current architecture
deb: build-release
    cargo deb -p moltis --no-build

# Build Debian package for amd64
deb-amd64:
    cargo build --release --target x86_64-unknown-linux-gnu
    cargo deb -p moltis --no-build --target x86_64-unknown-linux-gnu

# Build Debian package for arm64
deb-arm64:
    cargo build --release --target aarch64-unknown-linux-gnu
    cargo deb -p moltis --no-build --target aarch64-unknown-linux-gnu

# Build Debian packages for all architectures
deb-all: deb-amd64 deb-arm64

# Build Arch package for the current architecture
arch-pkg: build-release
    #!/usr/bin/env bash
    set -euo pipefail
    VERSION=$(grep '^version' Cargo.toml | head -1 | sed 's/.*"\(.*\)"/\1/')
    ARCH=$(uname -m)
    PKG_DIR="target/arch-pkg"
    rm -rf "$PKG_DIR"
    mkdir -p "$PKG_DIR/usr/bin"
    cp target/release/moltis "$PKG_DIR/usr/bin/moltis"
    chmod 755 "$PKG_DIR/usr/bin/moltis"
    cat > "$PKG_DIR/.PKGINFO" <<PKGINFO
    pkgname = moltis
    pkgver = ${VERSION}-1
    pkgdesc = Personal AI gateway inspired by OpenClaw
    url = https://www.moltis.org/
    arch = ${ARCH}
    license = MIT
    PKGINFO
    cd "$PKG_DIR"
    fakeroot -- tar --zstd -cf "../../moltis-${VERSION}-1-${ARCH}.pkg.tar.zst" .PKGINFO usr/
    echo "Built moltis-${VERSION}-1-${ARCH}.pkg.tar.zst"

# Build Arch package for x86_64
arch-pkg-x86_64:
    #!/usr/bin/env bash
    set -euo pipefail
    cargo build --release --target x86_64-unknown-linux-gnu
    VERSION=$(grep '^version' Cargo.toml | head -1 | sed 's/.*"\(.*\)"/\1/')
    PKG_DIR="target/arch-pkg-x86_64"
    rm -rf "$PKG_DIR"
    mkdir -p "$PKG_DIR/usr/bin"
    cp target/x86_64-unknown-linux-gnu/release/moltis "$PKG_DIR/usr/bin/moltis"
    chmod 755 "$PKG_DIR/usr/bin/moltis"
    cat > "$PKG_DIR/.PKGINFO" <<PKGINFO
    pkgname = moltis
    pkgver = ${VERSION}-1
    pkgdesc = Personal AI gateway inspired by OpenClaw
    url = https://www.moltis.org/
    arch = x86_64
    license = MIT
    PKGINFO
    cd "$PKG_DIR"
    fakeroot -- tar --zstd -cf "../../moltis-${VERSION}-1-x86_64.pkg.tar.zst" .PKGINFO usr/
    echo "Built moltis-${VERSION}-1-x86_64.pkg.tar.zst"

# Build Arch package for aarch64
arch-pkg-aarch64:
    #!/usr/bin/env bash
    set -euo pipefail
    cargo build --release --target aarch64-unknown-linux-gnu
    VERSION=$(grep '^version' Cargo.toml | head -1 | sed 's/.*"\(.*\)"/\1/')
    PKG_DIR="target/arch-pkg-aarch64"
    rm -rf "$PKG_DIR"
    mkdir -p "$PKG_DIR/usr/bin"
    cp target/aarch64-unknown-linux-gnu/release/moltis "$PKG_DIR/usr/bin/moltis"
    chmod 755 "$PKG_DIR/usr/bin/moltis"
    cat > "$PKG_DIR/.PKGINFO" <<PKGINFO
    pkgname = moltis
    pkgver = ${VERSION}-1
    pkgdesc = Personal AI gateway inspired by OpenClaw
    url = https://www.moltis.org/
    arch = aarch64
    license = MIT
    PKGINFO
    cd "$PKG_DIR"
    fakeroot -- tar --zstd -cf "../../moltis-${VERSION}-1-aarch64.pkg.tar.zst" .PKGINFO usr/
    echo "Built moltis-${VERSION}-1-aarch64.pkg.tar.zst"

# Build Arch packages for all architectures
arch-pkg-all: arch-pkg-x86_64 arch-pkg-aarch64

# Build RPM package for the current architecture
rpm: build-release
    cargo generate-rpm -p crates/cli

# Build RPM package for x86_64
rpm-x86_64:
    cargo build --release --target x86_64-unknown-linux-gnu
    cargo generate-rpm -p crates/cli --target x86_64-unknown-linux-gnu

# Build RPM package for aarch64
rpm-aarch64:
    cargo build --release --target aarch64-unknown-linux-gnu
    cargo generate-rpm -p crates/cli --target aarch64-unknown-linux-gnu

# Build RPM packages for all architectures
rpm-all: rpm-x86_64 rpm-aarch64

# Build AppImage for the current architecture
appimage: build-release
    #!/usr/bin/env bash
    set -euo pipefail
    VERSION=$(grep '^version' Cargo.toml | head -1 | sed 's/.*"\(.*\)"/\1/')
    ARCH=$(uname -m)
    APP_DIR="target/moltis.AppDir"
    rm -rf "$APP_DIR"
    mkdir -p "$APP_DIR/usr/bin"
    cp target/release/moltis "$APP_DIR/usr/bin/moltis"
    chmod 755 "$APP_DIR/usr/bin/moltis"
    cat > "$APP_DIR/moltis.desktop" <<DESKTOP
    [Desktop Entry]
    Type=Application
    Name=Moltis
    Exec=moltis
    Icon=moltis
    Categories=Network;
    Terminal=true
    DESKTOP
    cat > "$APP_DIR/moltis.svg" <<SVG
    <svg xmlns="http://www.w3.org/2000/svg" width="256" height="256"><rect width="256" height="256" fill="#333"/><text x="128" y="140" font-size="120" text-anchor="middle" fill="white">M</text></svg>
    SVG
    ln -sf moltis.svg "$APP_DIR/.DirIcon"
    cat > "$APP_DIR/AppRun" <<'APPRUN'
    #!/bin/sh
    SELF=$(readlink -f "$0")
    HERE=${SELF%/*}
    exec "$HERE/usr/bin/moltis" "$@"
    APPRUN
    chmod +x "$APP_DIR/AppRun"
    if [ ! -f target/appimagetool ]; then
        wget -q "https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-${ARCH}.AppImage" -O target/appimagetool
        chmod +x target/appimagetool
    fi
    ARCH=${ARCH} target/appimagetool --appimage-extract-and-run "$APP_DIR" "moltis-${VERSION}-${ARCH}.AppImage"
    echo "Built moltis-${VERSION}-${ARCH}.AppImage"

# Build Snap package
snap:
    snapcraft

# Build Flatpak
flatpak:
    cd flatpak && flatpak-builder --repo=repo --force-clean builddir org.moltbot.Moltis.yml

# Run all CI checks (format, lint, build, test)
ci: format-check lint build test

# Run all tests
test:
    cargo test --all-features

# Build all Linux packages (deb + rpm + arch + appimage) for all architectures
packages-all: deb-all rpm-all arch-pkg-all
