#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
BRIDGE_CRATE_DIR="${REPO_ROOT}/crates/swift-bridge"
MACOS_APP_DIR="${REPO_ROOT}/apps/macos"
OUTPUT_DIR="${MACOS_APP_DIR}/Generated"
UNIVERSAL_DIR="${REPO_ROOT}/target/universal-macos/release"
MACOS_DEPLOYMENT_TARGET="${MACOS_DEPLOYMENT_TARGET:-14.0}"

if ! command -v cbindgen >/dev/null 2>&1; then
  echo "error: cbindgen is required (install with: cargo install cbindgen)" >&2
  exit 1
fi

if ! command -v lipo >/dev/null 2>&1; then
  echo "error: lipo is required (install Xcode command line tools)" >&2
  exit 1
fi

rustup target add x86_64-apple-darwin aarch64-apple-darwin
rustup target add wasm32-wasip2

# Build embedded WASM guest components before release host builds.
# moltis-tools includes these bytes at compile time in release mode.
cargo build --target wasm32-wasip2 -p moltis-wasm-calc -p moltis-wasm-web-fetch -p moltis-wasm-web-search --release

# The release build embeds WASM guest components via include_bytes!.
# Build them first if they don't already exist.
WASM_DIR="${REPO_ROOT}/target/wasm32-wasip2/release"
if [ ! -f "${WASM_DIR}/moltis_wasm_calc.wasm" ] || \
   [ ! -f "${WASM_DIR}/moltis_wasm_web_fetch.wasm" ] || \
   [ ! -f "${WASM_DIR}/moltis_wasm_web_search.wasm" ]; then
  echo "Building WASM tool components (required by release build)..."
  rustup target add wasm32-wasip2
  cargo build --target wasm32-wasip2 -p moltis-wasm-calc -p moltis-wasm-web-fetch -p moltis-wasm-web-search --release
fi

# Keep Rust and C/C++ deps aligned with Xcode app link settings to avoid min-version mismatch.
export MACOSX_DEPLOYMENT_TARGET="${MACOS_DEPLOYMENT_TARGET}"
export CMAKE_OSX_DEPLOYMENT_TARGET="${MACOS_DEPLOYMENT_TARGET}"
export CARGO_TARGET_X86_64_APPLE_DARWIN_RUSTFLAGS="${CARGO_TARGET_X86_64_APPLE_DARWIN_RUSTFLAGS:-} -C link-arg=-mmacosx-version-min=${MACOS_DEPLOYMENT_TARGET}"
export CARGO_TARGET_AARCH64_APPLE_DARWIN_RUSTFLAGS="${CARGO_TARGET_AARCH64_APPLE_DARWIN_RUSTFLAGS:-} -C link-arg=-mmacosx-version-min=${MACOS_DEPLOYMENT_TARGET}"
export CFLAGS_x86_64_apple_darwin="${CFLAGS_x86_64_apple_darwin:-} -mmacosx-version-min=${MACOS_DEPLOYMENT_TARGET}"
export CFLAGS_aarch64_apple_darwin="${CFLAGS_aarch64_apple_darwin:-} -mmacosx-version-min=${MACOS_DEPLOYMENT_TARGET}"
export CXXFLAGS_x86_64_apple_darwin="${CXXFLAGS_x86_64_apple_darwin:-} -mmacosx-version-min=${MACOS_DEPLOYMENT_TARGET}"
export CXXFLAGS_aarch64_apple_darwin="${CXXFLAGS_aarch64_apple_darwin:-} -mmacosx-version-min=${MACOS_DEPLOYMENT_TARGET}"

cargo build -p moltis-swift-bridge --release --target x86_64-apple-darwin
cargo build -p moltis-swift-bridge --release --target aarch64-apple-darwin

mkdir -p "${UNIVERSAL_DIR}" "${OUTPUT_DIR}"

lipo -create \
  "${REPO_ROOT}/target/x86_64-apple-darwin/release/libmoltis_swift_bridge.a" \
  "${REPO_ROOT}/target/aarch64-apple-darwin/release/libmoltis_swift_bridge.a" \
  -output "${UNIVERSAL_DIR}/libmoltis_bridge.a"

cbindgen "${BRIDGE_CRATE_DIR}" \
  --config "${BRIDGE_CRATE_DIR}/cbindgen.toml" \
  --crate moltis-swift-bridge \
  --output "${OUTPUT_DIR}/moltis_bridge.h"

cp "${UNIVERSAL_DIR}/libmoltis_bridge.a" "${OUTPUT_DIR}/libmoltis_bridge.a"

echo "Built Rust bridge artifacts in ${OUTPUT_DIR}"
