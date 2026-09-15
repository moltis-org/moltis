# Multi-stage Dockerfile for moltis
# Builds a minimal debian-based image with the moltis gateway
#
# Moltis uses Docker/Podman for sandboxed command execution. To enable this,
# mount the container runtime socket when running:
#
#   Docker:    -v /var/run/docker.sock:/var/run/docker.sock
#   Podman:    -v /run/podman/podman.sock:/var/run/docker.sock
#   OrbStack:  -v /var/run/docker.sock:/var/run/docker.sock (same as Docker)
#
# See README.md for detailed instructions.

# Build stage — nightly required for wacore-binary (portable_simd)
FROM rust:trixie AS builder

WORKDIR /build

# Copy rust-toolchain.toml first so the nightly pin is defined in one place.
COPY rust-toolchain.toml ./
RUN NIGHTLY="$(sed -nE 's/^channel[[:space:]]*=[[:space:]]*"([^"]+)"/\1/p' rust-toolchain.toml)" \
    && rustup install "$NIGHTLY" && rustup default "$NIGHTLY"

# Copy manifests first for better caching
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY apps/courier ./apps/courier
COPY scripts ./scripts
COPY vendor ./vendor
COPY wit ./wit
# docs/src is embedded into moltis-agents via include_dir! (crates/agents/src/docs.rs).
# CHANGELOG.md is the target of the docs/src/changelog.md symlink, so it must be
# present at the repo root for that symlink to resolve during the embed.
COPY CHANGELOG.md ./CHANGELOG.md
COPY docs/src ./docs/src

ENV DEBIAN_FRONTEND=noninteractive
# Install build dependencies for llama-cpp-sys-2
RUN apt-get update -qq && \
    apt-get install -yqq --no-install-recommends cmake build-essential libclang-dev pkg-config git && \
    rm -rf /var/lib/apt/lists/*

# Install Node.js for Vite/esbuild builds (web assets are gitignored)
RUN apt-get update -qq && \
    apt-get install -yqq --no-install-recommends ca-certificates curl gnupg && \
    curl -fsSL https://deb.nodesource.com/setup_22.x | bash - && \
    apt-get install -yqq --no-install-recommends nodejs && \
    rm -rf /var/lib/apt/lists/*

# Build all web assets (Vite JS + Tailwind CSS + service worker)
# npm's own cache is mounted; node_modules deliberately is not.
# build-web-assets.sh skips `npm ci` when node_modules already exists, so a
# cached one would silently be used against a newer package-lock.json. A warm
# ~/.npm makes the honest `npm ci` fast enough.
RUN --mount=type=cache,id=moltis-npm,target=/root/.npm,sharing=locked \
    ARCH=$(uname -m) && \
    case "$ARCH" in x86_64) TW="tailwindcss-linux-x64";; aarch64) TW="tailwindcss-linux-arm64";; esac && \
    curl -sLO "https://github.com/tailwindlabs/tailwindcss/releases/latest/download/$TW" && \
    chmod +x "$TW" && \
    TAILWINDCSS="./$TW" ./scripts/build-web-assets.sh

# Install WASM target and build WASM components (embedded via include_bytes!)
#
# From here on cargo builds into cache mounts, so that a rebuild recompiles only
# what changed instead of the whole dependency tree. A cache mount is not part of
# any layer, which has one consequence that has to be honoured: after the RUN
# ends, target/ is empty again as far as the image is concerned. Everything the
# runtime stage needs is therefore copied to /out inside the same RUN.
#
# `sharing=locked` because two concurrent builds over one cargo target directory
# is corruption rather than concurrency.
RUN --mount=type=cache,id=moltis-cargo-target,target=/build/target,sharing=locked \
    --mount=type=cache,id=moltis-cargo-registry,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,id=moltis-cargo-git,target=/usr/local/cargo/git,sharing=locked \
    rustup target add wasm32-wasip2 && \
    cargo build --target wasm32-wasip2 -p moltis-wasm-calc -p moltis-wasm-web-fetch -p moltis-wasm-web-search --release && \
    mkdir -p /out/wasm && \
    cp target/wasm32-wasip2/release/moltis_wasm_calc.wasm \
       target/wasm32-wasip2/release/moltis_wasm_web_fetch.wasm \
       target/wasm32-wasip2/release/moltis_wasm_web_search.wasm \
       /out/wasm/

# Build release binary with the same portable production feature set used by
# release/package builds.
ARG MOLTIS_VERSION
ENV MOLTIS_VERSION=${MOLTIS_VERSION}
#
# The zvec runtime is found rather than globbed, for two reasons that both come
# from the target cache: a persistent target/ accumulates `zvec-rust-sys-*` build
# directories from earlier builds, so the newest match is the only correct one -
# a glob could pick a stale sibling. And zvec is a feature of `full`, so a
# smaller feature set produces no such library at all; copying into a directory
# that exists even when empty keeps that case a no-op instead of a failed COPY.
RUN --mount=type=cache,id=moltis-cargo-target,target=/build/target,sharing=locked \
    --mount=type=cache,id=moltis-cargo-registry,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,id=moltis-cargo-git,target=/usr/local/cargo/git,sharing=locked \
    ./scripts/cargo-build-moltis.sh --release && \
    mkdir -p /out/lib && \
    cp target/release/moltis /out/moltis && \
    so="$(find target/release/build \
            -path '*zvec-rust-sys-*/out/zvec-prebuilt/libzvec_c_api.so' \
            -printf '%T@ %p\n' 2>/dev/null | sort -rn | head -n1 | cut -d' ' -f2-)" && \
    if [ -n "$so" ]; then cp "$so" /out/lib/; else echo "no zvec runtime in this feature set"; fi

# Runtime stage
FROM debian:trixie-slim

# Install base runtime dependencies
ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update -qq && \
    apt-get install -yqq --no-install-recommends \
        ca-certificates \
        chromium \
        curl \
        gnupg \
        libgomp1 \
        sudo \
        tmux \
        vim-tiny && \
    rm -rf /var/lib/apt/lists/*

# Install Node.js 22 LTS via NodeSource (npm/npx bundled) for stdio-based MCP servers
RUN install -m 0755 -d /etc/apt/keyrings && \
    curl -fsSL https://deb.nodesource.com/gpgkey/nodesource-repo.gpg.key \
        | gpg --dearmor -o /etc/apt/keyrings/nodesource.gpg && \
    echo "deb [signed-by=/etc/apt/keyrings/nodesource.gpg] https://deb.nodesource.com/node_22.x nodistro main" \
        > /etc/apt/sources.list.d/nodesource.list && \
    apt-get update -qq && \
    apt-get install -yqq --no-install-recommends nodejs && \
    rm -rf /var/lib/apt/lists/*

# Install Docker CLI for sandbox execution (talks to mounted socket, no daemon in-container)
RUN install -m 0755 -d /etc/apt/keyrings && \
    curl -fsSL https://download.docker.com/linux/debian/gpg \
        | gpg --dearmor -o /etc/apt/keyrings/docker.gpg && \
    chmod a+r /etc/apt/keyrings/docker.gpg && \
    echo "deb [arch=$(dpkg --print-architecture) signed-by=/etc/apt/keyrings/docker.gpg] https://download.docker.com/linux/debian $(. /etc/os-release && echo \"$VERSION_CODENAME\") stable" \
        > /etc/apt/sources.list.d/docker.list && \
    apt-get update -qq && \
    apt-get install -yqq --no-install-recommends \
        docker-buildx-plugin \
        docker-ce-cli && \
    rm -rf /var/lib/apt/lists/*

# Create non-root user and add to docker group for socket access.
# Grant passwordless sudo so moltis can install host packages at startup.
RUN groupadd -f docker && \
    useradd --create-home --user-group moltis && \
    usermod -aG docker moltis && \
    echo "moltis ALL=(ALL) NOPASSWD:ALL" > /etc/sudoers.d/moltis

# Copy binary and its zvec shared library from builder.
#
# Three of the four come out of /out, because target/ was a cache mount in the
# builder and is therefore not in its layers. The web assets are copied from the
# source tree, where build-web-assets.sh wrote them. The trailing slash on
# /out/lib/ is what makes an absent zvec library a no-op rather than a failure.
COPY --from=builder /out/moltis /usr/local/bin/moltis
COPY --from=builder /out/lib/ /usr/local/lib/
COPY --from=builder /build/crates/web/src/assets /usr/share/moltis/web
COPY --from=builder /out/wasm/ /usr/share/moltis/wasm/
RUN ldconfig && moltis --version

# Create config and data directories.
# Persistent state lives under these paths; mount a volume or bind mount at
# runtime (see deployment compose). Intentionally NOT declared as VOLUME -- an
# anonymous VOLUME on a nested path shadows a parent bind mount and seeds stub
# data over real data, which silently sends the app through onboarding again.
RUN mkdir -p /home/moltis/.config/moltis /home/moltis/.moltis /home/moltis/.npm && \
    chown -R moltis:moltis /home/moltis/.config /home/moltis/.moltis /home/moltis/.npm

USER moltis
WORKDIR /home/moltis

# Expose gateway port (HTTPS), HTTP port for CA certificate download (gateway port + 1),
# and OAuth callback port (used by providers with pre-registered redirect URIs).
EXPOSE 13131 13132 1455

# Bind 0.0.0.0 so Docker port forwarding works (localhost only binds to
# the container's loopback, making the port unreachable from the host).
ENTRYPOINT ["moltis"]
CMD ["--bind", "0.0.0.0", "--port", "13131"]
