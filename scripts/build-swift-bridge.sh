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
SWIFT_BRIDGE_PROFILE="${MOLTIS_SWIFT_BRIDGE_PROFILE:-release}"
SWIFT_BRIDGE_TARGET_DIR="release"
if [ "${SWIFT_BRIDGE_PROFILE}" != "release" ]; then
  SWIFT_BRIDGE_TARGET_DIR="${SWIFT_BRIDGE_PROFILE}"
fi
SKIP_WASM_PRECOMPILE="${MOLTIS_SWIFT_BRIDGE_SKIP_WASM_PRECOMPILE:-0}"
SKIP_WASM_BUILD="${MOLTIS_SWIFT_BRIDGE_SKIP_WASM_BUILD:-0}"
OPENSSL_CFLAGS=""
OPENSSL_LDFLAGS=""

if [ "${SKIP_WASM_BUILD}" = "1" ] && [ "${SWIFT_BRIDGE_PROFILE}" = "release" ]; then
  echo "error: MOLTIS_SWIFT_BRIDGE_SKIP_WASM_BUILD=1 requires a non-release bridge profile" >&2
  exit 1
fi

if ! command -v cbindgen >/dev/null 2>&1; then
  echo "error: cbindgen is required (install with: cargo install cbindgen)" >&2
  exit 1
fi

if ! command -v lipo >/dev/null 2>&1; then
  echo "error: lipo is required (install Xcode command line tools)" >&2
  exit 1
fi

cargo_cmd() {
  if command -v rustup >/dev/null 2>&1; then
    active_toolchain="$(rustup show active-toolchain 2>/dev/null || true)"
    active_toolchain="${active_toolchain%% *}"
    if [ -n "${active_toolchain}" ]; then
      rustup run "${active_toolchain}" cargo "$@"
      return
    fi
  fi

  command cargo "$@"
}

configure_macos_openssl() {
  if [ "$(uname -s)" != "Darwin" ]; then
    return
  fi

  local openssl_prefix="${OPENSSL_DIR:-}"
  if [ -z "${openssl_prefix}" ] && command -v brew >/dev/null 2>&1; then
    openssl_prefix="$(brew --prefix openssl@3 2>/dev/null || true)"
  fi

  if [ -z "${openssl_prefix}" ] || [ ! -d "${openssl_prefix}/include/openssl" ]; then
    return
  fi

  export OPENSSL_DIR="${openssl_prefix}"
  export OPENSSL_ROOT_DIR="${openssl_prefix}"
  export PKG_CONFIG_PATH="${openssl_prefix}/lib/pkgconfig${PKG_CONFIG_PATH:+:${PKG_CONFIG_PATH}}"
  OPENSSL_CFLAGS="-I${openssl_prefix}/include"
  OPENSSL_LDFLAGS="-L${openssl_prefix}/lib"
}

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

if [ "${SKIP_WASM_BUILD}" = "1" ]; then
  rustup target add "${TARGETS[@]}"
else
  rustup target add wasm32-wasip2 "${TARGETS[@]}"
fi

if [ "${SKIP_WASM_BUILD}" = "1" ]; then
  echo "Skipping wasm guest build (MOLTIS_SWIFT_BRIDGE_SKIP_WASM_BUILD=1)"
else
  # Build and pre-compile embedded WASM guest components before release host builds.
  # moltis-tools includes these bytes at compile time in release-like profiles.
  cargo_cmd build --target wasm32-wasip2 -p moltis-wasm-calc -p moltis-wasm-web-fetch -p moltis-wasm-web-search --release
  if [ "${SKIP_WASM_PRECOMPILE}" = "1" ]; then
    echo "Skipping wasm precompile (MOLTIS_SWIFT_BRIDGE_SKIP_WASM_PRECOMPILE=1)"
  else
    cargo_cmd run -p moltis-wasm-precompile --release
  fi
fi

configure_macos_openssl

# Rust caches can restore an already-configured llama.cpp CMake tree that was
# created before OpenSSL flags were present. Force reconfiguration here.
if [ -n "${OPENSSL_CFLAGS}" ]; then
  shopt -s nullglob
  for dir in \
    "${REPO_ROOT}"/target/*/build/llama-cpp-sys-2-* \
    "${REPO_ROOT}"/target/*/build/llama-cpp-2-* \
    "${REPO_ROOT}"/target/*/*/build/llama-cpp-sys-2-* \
    "${REPO_ROOT}"/target/*/*/build/llama-cpp-2-*; do
    rm -rf "${dir}"
  done
  shopt -u nullglob
fi

# Keep Rust and C/C++ deps aligned with Xcode app link settings to avoid min-version mismatch.
export MACOSX_DEPLOYMENT_TARGET="${MACOS_DEPLOYMENT_TARGET}"
export CMAKE_OSX_DEPLOYMENT_TARGET="${MACOS_DEPLOYMENT_TARGET}"
for target in "${TARGETS[@]}"; do
  case "${target}" in
    x86_64-apple-darwin)
      export CARGO_TARGET_X86_64_APPLE_DARWIN_RUSTFLAGS="${CARGO_TARGET_X86_64_APPLE_DARWIN_RUSTFLAGS:-} -C link-arg=-mmacosx-version-min=${MACOS_DEPLOYMENT_TARGET}"
      export CFLAGS_x86_64_apple_darwin="${CFLAGS_x86_64_apple_darwin:-} -mmacosx-version-min=${MACOS_DEPLOYMENT_TARGET} ${OPENSSL_CFLAGS}"
      export CXXFLAGS_x86_64_apple_darwin="${CXXFLAGS_x86_64_apple_darwin:-} -mmacosx-version-min=${MACOS_DEPLOYMENT_TARGET} ${OPENSSL_CFLAGS}"
      export LDFLAGS_x86_64_apple_darwin="${LDFLAGS_x86_64_apple_darwin:-} ${OPENSSL_LDFLAGS}"
      ;;
    aarch64-apple-darwin)
      export CARGO_TARGET_AARCH64_APPLE_DARWIN_RUSTFLAGS="${CARGO_TARGET_AARCH64_APPLE_DARWIN_RUSTFLAGS:-} -C link-arg=-mmacosx-version-min=${MACOS_DEPLOYMENT_TARGET}"
      export CFLAGS_aarch64_apple_darwin="${CFLAGS_aarch64_apple_darwin:-} -mmacosx-version-min=${MACOS_DEPLOYMENT_TARGET} ${OPENSSL_CFLAGS}"
      export CXXFLAGS_aarch64_apple_darwin="${CXXFLAGS_aarch64_apple_darwin:-} -mmacosx-version-min=${MACOS_DEPLOYMENT_TARGET} ${OPENSSL_CFLAGS}"
      export LDFLAGS_aarch64_apple_darwin="${LDFLAGS_aarch64_apple_darwin:-} ${OPENSSL_LDFLAGS}"
      ;;
  esac
done

BUILD_ARGS=(build -p moltis-swift-bridge --profile "${SWIFT_BRIDGE_PROFILE}")
for target in "${TARGETS[@]}"; do
  BUILD_ARGS+=(--target "${target}")
done
cargo_cmd "${BUILD_ARGS[@]}"

mkdir -p "${UNIVERSAL_DIR}" "${OUTPUT_DIR}"

if [ "${#TARGETS[@]}" -eq 1 ]; then
  cp \
    "${REPO_ROOT}/target/${TARGETS[0]}/${SWIFT_BRIDGE_TARGET_DIR}/libmoltis_swift_bridge.a" \
    "${UNIVERSAL_DIR}/libmoltis_bridge.a"
else
  LIPO_INPUTS=()
  for target in "${TARGETS[@]}"; do
    LIPO_INPUTS+=("${REPO_ROOT}/target/${target}/${SWIFT_BRIDGE_TARGET_DIR}/libmoltis_swift_bridge.a")
  done
  lipo -create "${LIPO_INPUTS[@]}" -output "${UNIVERSAL_DIR}/libmoltis_bridge.a"
fi

cbindgen "${BRIDGE_CRATE_DIR}" \
  --config "${BRIDGE_CRATE_DIR}/cbindgen.toml" \
  --crate moltis-swift-bridge \
  --output "${OUTPUT_DIR}/moltis_bridge.h"

cp "${UNIVERSAL_DIR}/libmoltis_bridge.a" "${OUTPUT_DIR}/libmoltis_bridge.a"

echo "Built Rust bridge artifacts in ${OUTPUT_DIR}"
