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

# Build stage
FROM rust:bookworm AS builder

WORKDIR /build

# Copy manifests first for better caching
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY apps/courier ./apps/courier

ENV DEBIAN_FRONTEND=noninteractive
# Install build dependencies for llama-cpp-sys-2
RUN apt-get update -qq && \
    apt-get install -yqq --no-install-recommends cmake build-essential libclang-dev pkg-config git && \
    rm -rf /var/lib/apt/lists/*

# Install WASM target and build WASM components (embedded via include_bytes!)
RUN rustup target add wasm32-wasip2 && \
    cargo build --target wasm32-wasip2 -p moltis-wasm-calc -p moltis-wasm-web-fetch -p moltis-wasm-web-search --release

# Build release binary
RUN cargo build --release

# Runtime stage
FROM debian:bookworm-slim

# Install runtime dependencies:
# - ca-certificates: for HTTPS connections to LLM providers
# - chromium: headless browser for the browser tool (web search/fetch)
# - curl: makes it possible to run healthchecks from docker
# - sudo: allows moltis user to install packages at runtime (passwordless)
# - docker-ce-cli + docker-buildx-plugin: Docker CLI for sandbox execution
#   (talks to mounted socket, no daemon in-container)
# - tmux: terminal multiplexer available in deployed container
# - vim-tiny: lightweight terminal text editor
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
    install -m 0755 -d /etc/apt/keyrings && \
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

# Copy binary from builder
COPY --from=builder /build/target/release/moltis /usr/local/bin/moltis

# Create config and data directories
RUN mkdir -p /home/moltis/.config/moltis /home/moltis/.moltis && \
    chown -R moltis:moltis /home/moltis/.config /home/moltis/.moltis

# Volume mount points for persistence and container runtime
VOLUME ["/home/moltis/.config/moltis", "/home/moltis/.moltis", "/var/run/docker.sock"]

USER moltis
WORKDIR /home/moltis

# Expose gateway port (HTTPS), HTTP port for CA certificate download (gateway port + 1),
# and OAuth callback port (used by providers with pre-registered redirect URIs).
EXPOSE 13131 13132 1455

# Bind 0.0.0.0 so Docker port forwarding works (localhost only binds to
# the container's loopback, making the port unreachable from the host).
ENTRYPOINT ["moltis"]
CMD ["--bind", "0.0.0.0", "--port", "13131"]
