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

# Install build dependencies for llama-cpp-sys-2
RUN apt-get update && \
    apt-get install -y --no-install-recommends cmake build-essential libclang-dev pkg-config git && \
    rm -rf /var/lib/apt/lists/*

# Build release binary
RUN cargo build --release

# Runtime stage
FROM debian:bookworm-slim

# Install runtime dependencies:
# - ca-certificates: for HTTPS connections to LLM providers
# - docker.io: Docker CLI for sandbox execution (talks to mounted socket)
RUN apt-get update && \
    apt-get install -y --no-install-recommends \
        ca-certificates \
        libgomp1 \
        docker.io && \
    rm -rf /var/lib/apt/lists/*

# Create non-root user and add to docker group for socket access
RUN useradd --create-home --user-group moltis && \
    usermod -aG docker moltis

# Copy binary from builder
COPY --from=builder /build/target/release/moltis /usr/local/bin/moltis

# Create config and data directories
RUN mkdir -p /home/moltis/.config/moltis /home/moltis/.moltis && \
    chown -R moltis:moltis /home/moltis/.config /home/moltis/.moltis

# Volume mount points for persistence and container runtime
VOLUME ["/home/moltis/.config/moltis", "/home/moltis/.moltis", "/var/run/docker.sock"]

USER moltis
WORKDIR /home/moltis

# Expose gateway port (HTTPS) and HTTP port for CA certificate download (gateway port + 1)
EXPOSE 13131 13132

# Bind 0.0.0.0 so Docker port forwarding works (localhost only binds to
# the container's loopback, making the port unreachable from the host).
ENTRYPOINT ["moltis"]
CMD ["--bind", "0.0.0.0", "--port", "13131"]
