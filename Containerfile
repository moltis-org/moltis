FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates libssl3 && rm -rf /var/lib/apt/lists/*
COPY target/release/moltis /usr/bin/moltis
COPY target/release/moltis_wasm_*.wasm /usr/share/moltis/wasm/
COPY target/release/moltis_wasm_*.cwasm /usr/share/moltis/wasm/
COPY crates/web/src/assets/ /usr/share/moltis/web/
ENTRYPOINT ["moltis"]
