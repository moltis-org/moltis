#!/bin/sh
# Moltis installer script
# https://www.moltis.org/
#
# Usage:
#   curl -fsSL https://www.moltis.org/install.sh | sh
#
# Or with options:
#   curl -fsSL https://www.moltis.org/install.sh | sh -s -- --no-homebrew
#   curl -fsSL https://www.moltis.org/install.sh | sh -s -- --method=binary
#   curl -fsSL https://www.moltis.org/install.sh | sh -s -- --version=0.1.3

set -e

GITHUB_REPO="moltis-org/moltis"
HOMEBREW_TAP="moltis-org/tap"
BINARY_NAME="moltis"
INSTALL_DIR="${INSTALL_DIR:-$HOME/.local/bin}"

# Default options
USE_HOMEBREW=true
PREFERRED_METHOD=""
VERSION=""

# Colors (disabled if not a terminal)
if [ -t 1 ]; then
    RED='\033[0;31m'
    GREEN='\033[0;32m'
    YELLOW='\033[0;33m'
    BLUE='\033[0;34m'
    BOLD='\033[1m'
    NC='\033[0m'
else
    RED=''
    GREEN=''
    YELLOW=''
    BLUE=''
    BOLD=''
    NC=''
fi

info() {
    printf "${BLUE}==>${NC} ${BOLD}%s${NC}\n" "$1"
}

success() {
    printf "${GREEN}==>${NC} ${BOLD}%s${NC}\n" "$1"
}

warn() {
    printf "${YELLOW}Warning:${NC} %s\n" "$1" >&2
}

error() {
    printf "${RED}Error:${NC} %s\n" "$1" >&2
    exit 1
}

# Parse arguments
while [ $# -gt 0 ]; do
    case "$1" in
        --no-homebrew)
            USE_HOMEBREW=false
            ;;
        --method=*)
            PREFERRED_METHOD="${1#*=}"
            ;;
        --version=*)
            VERSION="${1#*=}"
            ;;
        -h|--help)
            cat <<EOF
Moltis installer

Usage:
    install.sh [OPTIONS]

Options:
    --no-homebrew       Skip Homebrew even if available (macOS)
    --method=METHOD     Force installation method: homebrew, binary, deb, rpm, arch, snap, source
    --version=VERSION   Install a specific version (default: latest)
    -h, --help          Show this help message

Environment variables:
    INSTALL_DIR         Binary installation directory (default: ~/.local/bin)

Examples:
    curl -fsSL https://www.moltis.org/install.sh | sh
    curl -fsSL https://www.moltis.org/install.sh | sh -s -- --method=binary
    curl -fsSL https://www.moltis.org/install.sh | sh -s -- --version=0.1.3
EOF
            exit 0
            ;;
        *)
            warn "Unknown option: $1"
            ;;
    esac
    shift
done

detect_os() {
    OS="$(uname -s)"
    case "$OS" in
        Darwin)
            echo "macos"
            ;;
        Linux)
            echo "linux"
            ;;
        MINGW*|MSYS*|CYGWIN*)
            echo "windows"
            ;;
        *)
            echo "unknown"
            ;;
    esac
}

detect_arch() {
    ARCH="$(uname -m)"
    case "$ARCH" in
        x86_64|amd64)
            echo "x86_64"
            ;;
        aarch64|arm64)
            echo "aarch64"
            ;;
        armv7l)
            echo "armv7"
            ;;
        i386|i686)
            echo "i686"
            ;;
        *)
            echo "$ARCH"
            ;;
    esac
}

detect_linux_distro() {
    if [ -f /etc/os-release ]; then
        # shellcheck disable=SC1091
        . /etc/os-release
        echo "$ID"
    elif [ -f /etc/debian_version ]; then
        echo "debian"
    elif [ -f /etc/redhat-release ]; then
        echo "rhel"
    elif [ -f /etc/arch-release ]; then
        echo "arch"
    else
        echo "unknown"
    fi
}

command_exists() {
    command -v "$1" >/dev/null 2>&1
}

get_latest_version() {
    if command_exists curl; then
        curl -fsSL "https://api.github.com/repos/${GITHUB_REPO}/releases/latest" | grep '"tag_name":' | sed -E 's/.*"v([^"]+)".*/\1/'
    elif command_exists wget; then
        wget -qO- "https://api.github.com/repos/${GITHUB_REPO}/releases/latest" | grep '"tag_name":' | sed -E 's/.*"v([^"]+)".*/\1/'
    else
        error "Neither curl nor wget found. Please install one of them."
    fi
}

download() {
    url="$1"
    dest="$2"
    if command_exists curl; then
        curl -fsSL "$url" -o "$dest"
    elif command_exists wget; then
        wget -q "$url" -O "$dest"
    else
        error "Neither curl nor wget found. Please install one of them."
    fi
}

verify_checksum() {
    file="$1"
    expected_sha256="$2"

    if command_exists sha256sum; then
        actual=$(sha256sum "$file" | cut -d' ' -f1)
    elif command_exists shasum; then
        actual=$(shasum -a 256 "$file" | cut -d' ' -f1)
    else
        warn "Cannot verify checksum (sha256sum/shasum not found)"
        return 0
    fi

    if [ "$actual" != "$expected_sha256" ]; then
        error "Checksum verification failed!\nExpected: $expected_sha256\nActual: $actual"
    fi
}

ensure_install_dir() {
    if [ ! -d "$INSTALL_DIR" ]; then
        mkdir -p "$INSTALL_DIR"
    fi
}

add_to_path_instructions() {
    shell_name=$(basename "$SHELL")
    case "$shell_name" in
        bash)
            rc_file="$HOME/.bashrc"
            ;;
        zsh)
            rc_file="$HOME/.zshrc"
            ;;
        fish)
            rc_file="$HOME/.config/fish/config.fish"
            ;;
        *)
            rc_file="$HOME/.profile"
            ;;
    esac

    # Check if already in PATH
    case ":$PATH:" in
        *":$INSTALL_DIR:"*)
            return
            ;;
    esac

    printf "\n"
    warn "$INSTALL_DIR is not in your PATH."
    printf "Add it by running:\n\n"
    if [ "$shell_name" = "fish" ]; then
        printf "  ${BOLD}fish_add_path %s${NC}\n\n" "$INSTALL_DIR"
    else
        printf "  ${BOLD}echo 'export PATH=\"%s:\$PATH\"' >> %s${NC}\n\n" "$INSTALL_DIR" "$rc_file"
    fi
    printf "Then restart your shell or run:\n"
    printf "  ${BOLD}source %s${NC}\n" "$rc_file"
}

# Installation methods

install_homebrew() {
    info "Installing via Homebrew..."
    if ! command_exists brew; then
        error "Homebrew not found. Install it from https://brew.sh/"
    fi
    brew tap "$HOMEBREW_TAP" 2>/dev/null || true
    brew install moltis
    success "Moltis installed via Homebrew"
}

install_binary() {
    os="$1"
    arch="$2"
    version="$3"

    # Determine target triple
    case "$os" in
        macos)
            target="${arch}-apple-darwin"
            ;;
        linux)
            target="${arch}-unknown-linux-gnu"
            ;;
        *)
            error "Unsupported OS for binary installation: $os"
            ;;
    esac

    tarball="${BINARY_NAME}-${version}-${target}.tar.gz"
    url="https://github.com/${GITHUB_REPO}/releases/download/v${version}/${tarball}"
    checksum_url="${url}.sha256"

    info "Downloading ${BINARY_NAME} v${version} for ${target}..."

    tmpdir=$(mktemp -d)
    trap 'rm -rf "$tmpdir"' EXIT

    download "$url" "$tmpdir/$tarball" || error "Failed to download $tarball. Check if a release exists for your platform."

    # Verify checksum
    if download "$checksum_url" "$tmpdir/checksum.sha256" 2>/dev/null; then
        expected_sha=$(cut -d' ' -f1 "$tmpdir/checksum.sha256")
        verify_checksum "$tmpdir/$tarball" "$expected_sha"
        info "Checksum verified"
    else
        warn "Could not download checksum file, skipping verification"
    fi

    # Extract and install
    tar -xzf "$tmpdir/$tarball" -C "$tmpdir"

    ensure_install_dir
    mv "$tmpdir/$BINARY_NAME" "$INSTALL_DIR/$BINARY_NAME"
    chmod +x "$INSTALL_DIR/$BINARY_NAME"

    success "Moltis installed to $INSTALL_DIR/$BINARY_NAME"
    add_to_path_instructions
}

install_deb() {
    arch="$1"
    version="$2"

    case "$arch" in
        x86_64) deb_arch="amd64" ;;
        aarch64) deb_arch="arm64" ;;
        *) error "Unsupported architecture for .deb: $arch" ;;
    esac

    # Package naming: moltis_VERSION-REV_ARCH.deb
    deb_file="moltis_${version}-1_${deb_arch}.deb"
    url="https://github.com/${GITHUB_REPO}/releases/download/v${version}/${deb_file}"

    info "Downloading ${deb_file}..."

    tmpdir=$(mktemp -d)
    trap 'rm -rf "$tmpdir"' EXIT

    download "$url" "$tmpdir/$deb_file" || error "Failed to download $deb_file"

    info "Installing .deb package (requires sudo)..."
    sudo dpkg -i "$tmpdir/$deb_file" || sudo apt-get install -f -y

    success "Moltis installed via .deb package"
}

install_rpm() {
    arch="$1"
    version="$2"

    case "$arch" in
        x86_64) rpm_arch="x86_64" ;;
        aarch64) rpm_arch="aarch64" ;;
        *) error "Unsupported architecture for .rpm: $arch" ;;
    esac

    # Package naming: moltis-VERSION-1.ARCH.rpm
    rpm_file="moltis-${version}-1.${rpm_arch}.rpm"
    url="https://github.com/${GITHUB_REPO}/releases/download/v${version}/${rpm_file}"

    info "Downloading ${rpm_file}..."

    tmpdir=$(mktemp -d)
    trap 'rm -rf "$tmpdir"' EXIT

    download "$url" "$tmpdir/$rpm_file" || error "Failed to download $rpm_file"

    info "Installing .rpm package (requires sudo)..."
    if command_exists dnf; then
        sudo dnf install -y "$tmpdir/$rpm_file"
    elif command_exists yum; then
        sudo yum install -y "$tmpdir/$rpm_file"
    else
        sudo rpm -i "$tmpdir/$rpm_file"
    fi

    success "Moltis installed via .rpm package"
}

install_arch() {
    arch="$1"
    version="$2"

    pkg_file="moltis-${version}-1-${arch}.pkg.tar.zst"
    url="https://github.com/${GITHUB_REPO}/releases/download/v${version}/${pkg_file}"

    info "Downloading ${pkg_file}..."

    tmpdir=$(mktemp -d)
    trap 'rm -rf "$tmpdir"' EXIT

    download "$url" "$tmpdir/$pkg_file" || error "Failed to download $pkg_file"

    info "Installing Arch package (requires sudo)..."
    sudo pacman -U --noconfirm "$tmpdir/$pkg_file"

    success "Moltis installed via Arch package"
}

install_snap() {
    info "Installing via Snap..."

    if ! command_exists snap; then
        error "Snap not found. Install it first: https://snapcraft.io/docs/installing-snapd"
    fi

    sudo snap install moltis

    success "Moltis installed via Snap"
}

install_from_source() {
    warn "Building from source. This may take several minutes..."

    if ! command_exists cargo; then
        info "Rust not found. Installing via rustup..."
        if command_exists curl; then
            curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
        else
            wget -qO- https://sh.rustup.rs | sh -s -- -y
        fi
        # shellcheck disable=SC1091
        . "$HOME/.cargo/env"
    fi

    if ! command_exists git; then
        error "Git is required to build from source. Please install it first."
    fi

    version="$1"

    tmpdir=$(mktemp -d)
    trap 'rm -rf "$tmpdir"' EXIT

    info "Cloning repository..."
    git clone --depth 1 --branch "v${version}" "https://github.com/${GITHUB_REPO}.git" "$tmpdir/moltis"

    cd "$tmpdir/moltis"

    info "Building release binary..."
    cargo build --release

    ensure_install_dir
    cp "target/release/$BINARY_NAME" "$INSTALL_DIR/$BINARY_NAME"
    chmod +x "$INSTALL_DIR/$BINARY_NAME"

    success "Moltis built and installed to $INSTALL_DIR/$BINARY_NAME"
    add_to_path_instructions
}

# Main installation logic

main() {
    printf "\n"
    printf "  ${BOLD}Moltis Installer${NC}\n"
    printf "  Personal AI gateway - one binary, multiple LLM providers\n"
    printf "\n"

    OS=$(detect_os)
    ARCH=$(detect_arch)

    info "Detected: $OS ($ARCH)"

    if [ "$OS" = "windows" ]; then
        error "Windows is not supported by this installer. Please download the binary manually from:\nhttps://github.com/${GITHUB_REPO}/releases"
    fi

    if [ "$OS" = "unknown" ]; then
        error "Unsupported operating system: $(uname -s)"
    fi

    # Get version
    if [ -z "$VERSION" ]; then
        info "Fetching latest version..."
        VERSION=$(get_latest_version)
        if [ -z "$VERSION" ]; then
            error "Failed to determine latest version"
        fi
    fi
    info "Version: $VERSION"

    # Determine installation method
    if [ -n "$PREFERRED_METHOD" ]; then
        case "$PREFERRED_METHOD" in
            homebrew)
                install_homebrew
                ;;
            binary)
                install_binary "$OS" "$ARCH" "$VERSION"
                ;;
            deb)
                install_deb "$ARCH" "$VERSION"
                ;;
            rpm)
                install_rpm "$ARCH" "$VERSION"
                ;;
            arch)
                install_arch "$ARCH" "$VERSION"
                ;;
            snap)
                install_snap
                ;;
            source)
                install_from_source "$VERSION"
                ;;
            *)
                error "Unknown installation method: $PREFERRED_METHOD"
                ;;
        esac
    elif [ "$OS" = "macos" ]; then
        # macOS: prefer Homebrew, fall back to binary
        if [ "$USE_HOMEBREW" = true ] && command_exists brew; then
            install_homebrew
        else
            install_binary "$OS" "$ARCH" "$VERSION"
        fi
    elif [ "$OS" = "linux" ]; then
        # Linux: detect distro and use appropriate package manager
        DISTRO=$(detect_linux_distro)

        case "$DISTRO" in
            ubuntu|debian|linuxmint|pop|elementary|zorin)
                if command_exists apt-get; then
                    install_deb "$ARCH" "$VERSION"
                else
                    install_binary "$OS" "$ARCH" "$VERSION"
                fi
                ;;
            fedora|rhel|centos|rocky|alma|ol)
                if command_exists dnf || command_exists yum; then
                    install_rpm "$ARCH" "$VERSION"
                else
                    install_binary "$OS" "$ARCH" "$VERSION"
                fi
                ;;
            arch|manjaro|endeavouros|garuda)
                if command_exists pacman; then
                    install_arch "$ARCH" "$VERSION"
                else
                    install_binary "$OS" "$ARCH" "$VERSION"
                fi
                ;;
            *)
                # Unknown distro: try binary, offer source as fallback
                if [ "$ARCH" = "x86_64" ] || [ "$ARCH" = "aarch64" ]; then
                    install_binary "$OS" "$ARCH" "$VERSION"
                else
                    warn "No pre-built binary available for $ARCH architecture."
                    info "Falling back to building from source..."
                    install_from_source "$VERSION"
                fi
                ;;
        esac
    fi

    # Verify installation
    if command_exists "$BINARY_NAME"; then
        installed_version=$("$BINARY_NAME" --version 2>/dev/null | head -1 || echo "unknown")
        printf "\n"
        success "Installation complete!"
        printf "  ${BOLD}%s${NC}\n" "$installed_version"
        printf "\n"
        printf "Get started:\n"
        printf "  ${BOLD}moltis${NC}          # Start the gateway\n"
        printf "  ${BOLD}moltis --help${NC}   # Show help\n"
        printf "\n"
        printf "Documentation: ${BLUE}https://www.moltis.org/${NC}\n"
    elif [ -x "$INSTALL_DIR/$BINARY_NAME" ]; then
        printf "\n"
        success "Installation complete!"
        printf "\n"
        add_to_path_instructions
    fi
}

main
