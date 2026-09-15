#!/usr/bin/env bash
#
# Build the Moltis docker image from this checkout.
#
#   ./scripts/build-image.sh                       # for this machine
#   ./scripts/build-image.sh --cache-dir ~/.cache/moltis-img
#
# The Dockerfile keeps cargo's target directory and crate registry in BuildKit
# cache mounts, so a rebuild recompiles what changed rather than the whole
# dependency tree. Those mounts live in the BuildKit cache and survive
# --no-cache; --cache-dir is a separate thing, an exportable cache of the image
# *layers*, which is what carries a build across machines or a pruned daemon.
#
# scripts/build-and-load-remote.sh builds through this script and then streams
# the result onto another host.

set -euo pipefail

usage() {
  cat <<'EOF'
Usage:
  ./scripts/build-image.sh [options]

Options:
  -t, --tag TAG              Image tag to build (default: moltis:local)
  -p, --platform PLATFORM    Target platform (default: this machine's)
  -f, --file FILE            Dockerfile to build (default: the repo's own)
      --version VERSION      Sets the MOLTIS_VERSION build arg. Without it the
                             binary reports an empty version - see Version below
      --build-arg KEY=VALUE  Extra build arg (repeatable)
      --build-context N=PATH Extra named build context (repeatable)
      --cache-dir DIR        Import and export the image layer cache here
      --no-cache             Ignore the layer cache. Does not empty the cargo
                             cache mounts, which is usually what you want
      --allow-emulation      Build a platform this machine is not. Required for
                             that case - see Emulation below
      --no-binfmt            Never try to register QEMU handlers
  -n, --dry-run              Print what would run, touch nothing
  -h, --help                 This text

Version:
  Release builds get their version from the MOLTIS_VERSION build arg, and CI
  always passes it. A local build without --version produces a binary whose
  version string is empty, which also means it does not identify as a dev build:
  its update checker stays active and `/update` will offer to replace the binary.
  Pass --version, or leave it and know that the image is the update mechanism.

Emulation:
  Building for a platform this machine is not means QEMU, and QEMU emulates
  every compile in this workspace - commonly five to ten times slower than
  native. That is why --allow-emulation exists and is not the default.

  For scale, this workspace builds in about 47 minutes on a Raspberry Pi 5
  (4 cores, cold cargo cache). An x86_64 desktop is perhaps five to ten times
  that machine natively, which the emulation penalty then cancels - so an
  emulated build is a wash at best, and slower than the target device at worst.
  QEMU's user-mode emulation also has gaps, and long LLVM builds under it hit
  them occasionally.

  Prefer a machine that is natively the target platform: an Apple Silicon Mac or
  an arm64 cloud instance needs no flag here at all.

  Registering the QEMU handlers writes to the kernel's binfmt_misc table and
  needs a privileged container (`docker run --privileged tonistiigi/binfmt`).
  It only happens when the builder does not already advertise the target
  platform, and --no-binfmt switches it off entirely.
EOF
}

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

case "$(uname -m)" in
  x86_64|amd64)  host_platform="linux/amd64" ;;
  aarch64|arm64) host_platform="linux/arm64" ;;
  *)             host_platform="linux/$(uname -m)" ;;
esac

tag="moltis:local"
platform="$host_platform"
dockerfile=""
version=""
cache_dir=""
no_cache=0
allow_emulation=0
install_binfmt=1
dry_run=0
build_args=()
build_contexts=()

need_value() {
  if [[ -z "${2:-}" || "${2}" == -* ]]; then
    echo "$1 needs a value" >&2
    usage >&2
    exit 2
  fi
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    -t|--tag)             need_value "$1" "${2:-}"; tag="$2"; shift 2 ;;
    -p|--platform)        need_value "$1" "${2:-}"; platform="$2"; shift 2 ;;
    -f|--file)            need_value "$1" "${2:-}"; dockerfile="$2"; shift 2 ;;
    --version)            need_value "$1" "${2:-}"; version="$2"; shift 2 ;;
    --build-arg)          need_value "$1" "${2:-}"; build_args+=("$2"); shift 2 ;;
    --build-context)      need_value "$1" "${2:-}"; build_contexts+=("$2"); shift 2 ;;
    --cache-dir)          need_value "$1" "${2:-}"; cache_dir="$2"; shift 2 ;;
    --no-cache)           no_cache=1; shift ;;
    --allow-emulation)    allow_emulation=1; shift ;;
    --no-binfmt)          install_binfmt=0; shift ;;
    -n|--dry-run)         dry_run=1; shift ;;
    -h|--help)            usage; exit 0 ;;
    *)
      echo "unexpected argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

log() {
  echo "==> $*" >&2
}

run() {
  if (( dry_run )); then
    printf '+' >&2
    printf ' %q' "$@" >&2
    printf '\n' >&2
    return 0
  fi
  "$@"
}

if ! command -v docker >/dev/null 2>&1; then
  echo "docker is not on PATH" >&2
  exit 1
fi

if ! docker buildx version >/dev/null 2>&1; then
  echo "docker buildx is not available, and this needs it" >&2
  exit 1
fi

if [[ -z "$version" ]]; then
  log "no --version: this image will report an empty version and will not"
  log "identify as a dev build. See Version in --help."
fi

# --- emulation ---------------------------------------------------------------

if [[ "$platform" != "$host_platform" ]]; then
  if (( ! allow_emulation )); then
    echo "refusing to build ${platform} on a ${host_platform} host." >&2
    echo "" >&2
    echo "QEMU would emulate every compile in this workspace, commonly five to ten" >&2
    echo "times slower than native. Build on a machine that is natively ${platform}," >&2
    echo "or pass --allow-emulation (with --cache-dir) to do it this way anyway." >&2
    echo "See the Emulation note in --help." >&2
    exit 1
  fi

  log "target ${platform} is not this machine's ${host_platform} - the build will be emulated"

  if (( install_binfmt )); then
    if (( dry_run )); then
      log "dry run: would check the builder for ${platform} and register QEMU handlers if absent"
    elif docker buildx inspect --bootstrap 2>/dev/null | grep -q -- "$platform"; then
      log "builder already advertises ${platform}"
    else
      log "registering QEMU handlers for ${platform#linux/} (privileged container)"
      run docker run --privileged --rm tonistiigi/binfmt --install "${platform#linux/}"
    fi
  fi
fi

# --- build -------------------------------------------------------------------

# --load puts the result in the local image store, which is where `docker save`
# and `docker run` look. Without it a buildx build can end up only in the cache.
build_cmd=(docker buildx build --platform "$platform" --load --tag "$tag")

[[ -n "$dockerfile" ]] && build_cmd+=(--file "$dockerfile")
[[ -n "$version" ]] && build_cmd+=(--build-arg "MOLTIS_VERSION=${version}")
(( no_cache )) && build_cmd+=(--no-cache)

if (( ${#build_args[@]} )); then
  for arg in "${build_args[@]}"; do build_cmd+=(--build-arg "$arg"); done
fi
if (( ${#build_contexts[@]} )); then
  for ctx in "${build_contexts[@]}"; do build_cmd+=(--build-context "$ctx"); done
fi
if [[ -n "$cache_dir" ]]; then
  run mkdir -p "$cache_dir"
  build_cmd+=(--cache-from "type=local,src=${cache_dir}")
  build_cmd+=(--cache-to "type=local,dest=${cache_dir},mode=max")
fi

build_cmd+=(--progress plain "$REPO_ROOT")

log "building ${tag} for ${platform}"
run "${build_cmd[@]}"

if ! (( dry_run )); then
  log "${tag} is $(docker image inspect --format '{{.Os}}/{{.Architecture}}' "$tag")"
fi
