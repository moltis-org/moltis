#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
BRIDGE_CRATE_DIR="${REPO_ROOT}/crates/swift-bridge"
MACOS_APP_DIR="${REPO_ROOT}/apps/macos"
OUTPUT_DIR="${MACOS_APP_DIR}/Generated"
UNIVERSAL_DIR="${REPO_ROOT}/target/universal-macos/release"
MACOS_DEPLOYMENT_TARGET="${MACOS_DEPLOYMENT_TARGET:-14.0}"
SWIFT_BRIDGE_TARGETS_CSV="${MOLTIS_SWIFT_BRIDGE_TARGETS:-x86_64-apple-darwin,aarch64-apple-darwin}"
SKIP_WASM_PRECOMPILE="${MOLTIS_SWIFT_BRIDGE_SKIP_WASM_PRECOMPILE:-0}"

if ! command -v cbindgen >/dev/null 2>&1; then
  echo "error: cbindgen is required (install with: cargo install cbindgen)" >&2
  exit 1
fi

if ! command -v lipo >/dev/null 2>&1; then
  echo "error: lipo is required (install Xcode command line tools)" >&2
  exit 1
fi

IFS=',' read -r -a RAW_TARGETS <<< "${SWIFT_BRIDGE_TARGETS_CSV}"
TARGETS=()
for raw_target in "${RAW_TARGETS[@]}"; do
  target="${raw_target//[[:space:]]/}"
  if [ -z "${target}" ]; then
    continue
  fi

  case "${target}" in
    x86_64-apple-darwin|aarch64-apple-darwin)
      TARGETS+=("${target}")
      ;;
    *)
      echo "error: unsupported target '${target}' in MOLTIS_SWIFT_BRIDGE_TARGETS" >&2
      exit 1
      ;;
  esac
done

if [ "${#TARGETS[@]}" -eq 0 ]; then
  echo "error: no valid bridge targets configured in MOLTIS_SWIFT_BRIDGE_TARGETS" >&2
  exit 1
fi

rustup target add wasm32-wasip2 "${TARGETS[@]}"

# Build and pre-compile embedded WASM guest components before release host builds.
# moltis-tools includes these bytes at compile time in release mode.
cargo build --target wasm32-wasip2 -p moltis-wasm-calc -p moltis-wasm-web-fetch -p moltis-wasm-web-search --release
if [ "${SKIP_WASM_PRECOMPILE}" = "1" ]; then
  echo "Skipping wasm precompile (MOLTIS_SWIFT_BRIDGE_SKIP_WASM_PRECOMPILE=1)"
else
  cargo run -p moltis-wasm-precompile --release
fi

# Keep Rust and C/C++ deps aligned with Xcode app link settings to avoid min-version mismatch.
export MACOSX_DEPLOYMENT_TARGET="${MACOS_DEPLOYMENT_TARGET}"
export CMAKE_OSX_DEPLOYMENT_TARGET="${MACOS_DEPLOYMENT_TARGET}"
for target in "${TARGETS[@]}"; do
  case "${target}" in
    x86_64-apple-darwin)
      export CARGO_TARGET_X86_64_APPLE_DARWIN_RUSTFLAGS="${CARGO_TARGET_X86_64_APPLE_DARWIN_RUSTFLAGS:-} -C link-arg=-mmacosx-version-min=${MACOS_DEPLOYMENT_TARGET}"
      export CFLAGS_x86_64_apple_darwin="${CFLAGS_x86_64_apple_darwin:-} -mmacosx-version-min=${MACOS_DEPLOYMENT_TARGET}"
      export CXXFLAGS_x86_64_apple_darwin="${CXXFLAGS_x86_64_apple_darwin:-} -mmacosx-version-min=${MACOS_DEPLOYMENT_TARGET}"
      ;;
    aarch64-apple-darwin)
      export CARGO_TARGET_AARCH64_APPLE_DARWIN_RUSTFLAGS="${CARGO_TARGET_AARCH64_APPLE_DARWIN_RUSTFLAGS:-} -C link-arg=-mmacosx-version-min=${MACOS_DEPLOYMENT_TARGET}"
      export CFLAGS_aarch64_apple_darwin="${CFLAGS_aarch64_apple_darwin:-} -mmacosx-version-min=${MACOS_DEPLOYMENT_TARGET}"
      export CXXFLAGS_aarch64_apple_darwin="${CXXFLAGS_aarch64_apple_darwin:-} -mmacosx-version-min=${MACOS_DEPLOYMENT_TARGET}"
      ;;
  esac
done

for target in "${TARGETS[@]}"; do
  cargo build -p moltis-swift-bridge --release --target "${target}"
done

mkdir -p "${UNIVERSAL_DIR}" "${OUTPUT_DIR}"

if [ "${#TARGETS[@]}" -eq 1 ]; then
  cp \
    "${REPO_ROOT}/target/${TARGETS[0]}/release/libmoltis_swift_bridge.a" \
    "${UNIVERSAL_DIR}/libmoltis_bridge.a"
else
  LIPO_INPUTS=()
  for target in "${TARGETS[@]}"; do
    LIPO_INPUTS+=("${REPO_ROOT}/target/${target}/release/libmoltis_swift_bridge.a")
  done
  lipo -create "${LIPO_INPUTS[@]}" -output "${UNIVERSAL_DIR}/libmoltis_bridge.a"
fi

cbindgen "${BRIDGE_CRATE_DIR}" \
  --config "${BRIDGE_CRATE_DIR}/cbindgen.toml" \
  --crate moltis-swift-bridge \
  --output "${OUTPUT_DIR}/moltis_bridge.h"

cp "${UNIVERSAL_DIR}/libmoltis_bridge.a" "${OUTPUT_DIR}/libmoltis_bridge.a"

echo "Built Rust bridge artifacts in ${OUTPUT_DIR}"
