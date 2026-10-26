#!/usr/bin/env bash
#
# Stage the zvec runtime library that belongs to a just-finished release build.
#
#   ./scripts/stage-zvec-runtime.sh target/release/moltis /out/lib
#
# This exists for a build whose target/ is a BuildKit cache mount, and therefore
# is not part of any image layer: what the runtime stage needs has to be copied
# somewhere else before the RUN ends.
#
# Finding the library is more than a glob, because a persistent target/ keeps
# `zvec-rust-sys-*` build directories from earlier builds. The newest match is
# the current one, so candidates are tried newest first - but a timestamp alone
# is not proof. Where one cache serves builds for more than one platform, the
# newest sibling can be for the other architecture, and a build that recompiled
# nothing does not refresh its own directory's timestamp to beat it. So each
# candidate is checked against the binary it has to load next to, and the first
# one that matches wins.
#
# zvec is a feature of `full`. A smaller feature set produces no such library at
# all, and that is a no-op here rather than a failure - the runtime stage copies
# from a directory that exists either way.

set -euo pipefail

usage() {
  echo "usage: $0 <reference-binary> <staging-dir>" >&2
}

binary="${1:-}"
staging="${2:-}"

if [[ -z "$binary" || -z "$staging" ]]; then
  usage
  exit 2
fi

if [[ ! -f "$binary" ]]; then
  echo "no such binary: ${binary}" >&2
  exit 1
fi

if ! command -v readelf >/dev/null 2>&1; then
  echo "readelf is not on PATH, and the architecture check needs it" >&2
  exit 1
fi

target_dir="${CARGO_TARGET_DIR:-target}"

elf_machine() {
  readelf -h "$1" 2>/dev/null | sed -n 's/^[[:space:]]*Machine:[[:space:]]*//p'
}

want="$(elf_machine "$binary")"
if [[ -z "$want" ]]; then
  echo "cannot read the ELF machine of ${binary}" >&2
  exit 1
fi

mkdir -p "$staging"

while read -r candidate; do
  [[ -n "$candidate" ]] || continue

  found="$(elf_machine "$candidate")"
  if [[ "$found" == "$want" ]]; then
    cp "$candidate" "$staging/"
    echo "staged ${candidate} (${want})"
    exit 0
  fi

  echo "skipping ${candidate}: it is ${found:-unreadable}, not ${want}" >&2
done < <(find "${target_dir}/release/build" \
           -path '*zvec-rust-sys-*/out/zvec-prebuilt/libzvec_c_api.so' \
           -printf '%T@ %p\n' 2>/dev/null | sort -rn | cut -d' ' -f2-)

echo "no zvec runtime in this feature set"
