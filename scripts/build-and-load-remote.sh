#!/usr/bin/env bash
#
# Build the Moltis image and load it onto a remote host over ssh.
#
# The case this exists for: the machine that runs Moltis is slower than one you
# can build on - a Raspberry Pi or a small ARM box. This builds the image on the
# faster machine and streams it straight into the remote docker daemon, so the
# device compiles nothing and no registry is involved.
#
#   ./scripts/build-and-load-remote.sh pi@raspberrypi
#
# The build itself is scripts/build-image.sh, and everything build-shaped below
# is handed to it - so the cargo cache, the platform handling and the emulation
# refusal live in one place. This pays off when the build machine is natively the
# target platform; when it is not, read the Emulation note in
# `./scripts/build-image.sh --help` first.

set -euo pipefail

usage() {
  cat <<'EOF'
Usage:
  ./scripts/build-and-load-remote.sh [options] <ssh-target>

Build the image, then `docker save | compress | ssh docker load` it onto
<ssh-target>. The target is handed to ssh unchanged, so aliases from
~/.ssh/config work.

Options:
  -t, --tag TAG              Image tag to build and load (default: moltis:local)
  -p, --platform PLATFORM    Target platform (default: linux/arm64, which is
                             what the devices this exists for usually are)
      --compress CMD         Compressor for the transfer (default: gzip).
                             `pigz` is much faster on many cores; `zstd -T0`
                             is faster still where the remote `docker load`
                             understands it.
      --skip-build           Ship the tag already in the local image store
  -n, --dry-run              Print what would run, touch nothing
  -h, --help                 This text

Handed to scripts/build-image.sh unchanged, see its --help:
  -f, --file FILE          --version VERSION       --build-arg KEY=VALUE
      --build-context N=P      --cache-dir DIR         --no-cache
      --allow-emulation        --no-binfmt

Examples:
  ./scripts/build-and-load-remote.sh pi@raspberrypi
  ./scripts/build-and-load-remote.sh --cache-dir ~/.cache/moltis-arm64 pi@pi5
  ./scripts/build-and-load-remote.sh --version 20260902.03 --compress pigz pi@pi5
  SSH='ssh -p 2222' ./scripts/build-and-load-remote.sh pi@box
EOF
}

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

tag="moltis:local"
platform="linux/arm64"
compress="gzip"
skip_build=0
dry_run=0
target=""
build_flags=()

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
    --compress)           need_value "$1" "${2:-}"; compress="$2"; shift 2 ;;
    --skip-build)         skip_build=1; shift ;;

    # Build-shaped, handed straight to build-image.sh.
    -f|--file|--version|--build-arg|--build-context|--cache-dir)
      need_value "$1" "${2:-}"; build_flags+=("$1" "$2"); shift 2 ;;
    --no-cache|--allow-emulation|--no-binfmt)
      build_flags+=("$1"); shift ;;

    -n|--dry-run)         dry_run=1; shift ;;
    -h|--help)            usage; exit 0 ;;
    -*)
      echo "unknown option: $1" >&2
      usage >&2
      exit 2
      ;;
    *)
      if [[ -n "$target" ]]; then
        echo "unexpected argument: $1 (ssh target already given as '$target')" >&2
        exit 2
      fi
      target="$1"
      shift
      ;;
  esac
done

if [[ -z "$target" ]]; then
  echo "no ssh target given" >&2
  usage >&2
  exit 2
fi

read -r -a ssh_cmd <<<"${SSH:-ssh}"
read -r -a compress_cmd <<<"$compress"

log() {
  echo "==> $*" >&2
}

human_bytes() {
  awk -v b="$1" 'BEGIN {
    split("B KiB MiB GiB TiB", u, " ")
    i = 1
    while (b >= 1024 && i < 5) { b /= 1024; i++ }
    printf "%.1f %s", b, u[i]
  }'
}

# --- local preflight ---------------------------------------------------------

if ! command -v docker >/dev/null 2>&1; then
  echo "docker is not on PATH" >&2
  exit 1
fi

# --- remote preflight, before the expensive part -----------------------------
#
# Deliberately first: a build can run for a long time, and finding out afterwards
# that the host is unreachable or has no docker is the one failure that wastes
# all of it.

if (( dry_run )); then
  log "dry run: skipping the reachability check for ${target}"
else
  log "checking ${target}"
  if ! remote_docker="$("${ssh_cmd[@]}" "$target" 'docker version --format "{{.Server.Version}}"' 2>&1)"; then
    echo "cannot reach a docker daemon on ${target}:" >&2
    echo "  ${remote_docker}" >&2
    echo "ssh has to work non-interactively and the login has to be able to talk" >&2
    echo "to the docker socket there." >&2
    exit 1
  fi
  log "${target} runs docker ${remote_docker}"
fi

# --- build -------------------------------------------------------------------

if (( skip_build )); then
  log "--skip-build: using ${tag} as it is in the local image store"
else
  build_cmd=("${SCRIPT_DIR}/build-image.sh" --tag "$tag" --platform "$platform")
  if (( ${#build_flags[@]} )); then
    build_cmd+=("${build_flags[@]}")
  fi
  (( dry_run )) && build_cmd+=(--dry-run)

  "${build_cmd[@]}"
fi

# --- check what we are about to ship -----------------------------------------

if ! (( dry_run )); then
  if ! image_arch="$(docker image inspect --format '{{.Os}}/{{.Architecture}}' "$tag" 2>/dev/null)"; then
    echo "${tag} is not in the local image store" >&2
    exit 1
  fi

  if [[ "$image_arch" != "$platform" ]]; then
    echo "${tag} is ${image_arch}, not the requested ${platform}." >&2
    echo "Loading it on a ${platform} host would give 'exec format error'." >&2
    exit 1
  fi

  image_size="$(docker image inspect --format '{{.Size}}' "$tag")"
  log "${tag} is ${image_arch}, $(human_bytes "$image_size") uncompressed"
  log "the transfer compresses that, but expect it to take a while"
fi

# --- ship --------------------------------------------------------------------

log "sending ${tag} to ${target} via ${compress_cmd[*]}"

if (( dry_run )); then
  printf "+ docker save %q | %s | %s %q 'docker load'\n" \
    "$tag" "${compress_cmd[*]}" "${ssh_cmd[*]}" "$target" >&2
else
  # pipefail is on, so a failure in save, in the compressor or on the remote
  # side fails the script rather than being swallowed by a happy `docker load`.
  docker save "$tag" | "${compress_cmd[@]}" | "${ssh_cmd[@]}" "$target" 'docker load'
fi

# --- verify ------------------------------------------------------------------

if (( dry_run )); then
  log "dry run: nothing to verify"
  exit 0
fi

local_id="$(docker image inspect --format '{{.Id}}' "$tag")"
if ! remote_id="$("${ssh_cmd[@]}" "$target" "docker image inspect --format '{{.Id}}' '${tag}'" 2>&1)"; then
  echo "${tag} did not turn up on ${target}:" >&2
  echo "  ${remote_id}" >&2
  exit 1
fi

if [[ "$local_id" != "$remote_id" ]]; then
  echo "${tag} on ${target} is not the image that was just sent." >&2
  echo "  local:  ${local_id}" >&2
  echo "  remote: ${remote_id}" >&2
  exit 1
fi

log "${tag} is on ${target} (${remote_id})"
