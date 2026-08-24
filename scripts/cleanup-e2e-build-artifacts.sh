#!/usr/bin/env bash
set -euo pipefail

# GitHub-hosted runners have limited disk. E2E only needs the built CLI binary
# plus web assets; the bulky Rust build intermediates can be removed safely.
if [ -x target/debug/moltis ]; then
  mkdir -p target/e2e-bin
  cp target/debug/moltis target/e2e-bin/moltis-bin
  chmod +x target/e2e-bin/moltis-bin

  zvec_library="$(find target/debug/build -type f \( -name 'libzvec_c_api.so' -o -name 'libzvec_c_api.dylib' -o -name 'zvec_c_api.dll' \) -print -quit)"
  if [ -z "$zvec_library" ]; then
    echo "Could not find the zvec runtime required by the default Moltis build" >&2
    exit 1
  fi
  cp "$zvec_library" target/e2e-bin/

  cat > target/e2e-bin/moltis <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

runtime_dir="$(cd -- "$(dirname -- "$0")" && pwd)"
case "$(uname -s)" in
  Darwin) export DYLD_LIBRARY_PATH="${runtime_dir}${DYLD_LIBRARY_PATH:+:${DYLD_LIBRARY_PATH}}" ;;
  Linux) export LD_LIBRARY_PATH="${runtime_dir}${LD_LIBRARY_PATH:+:${LD_LIBRARY_PATH}}" ;;
esac
exec "${runtime_dir}/moltis-bin" "$@"
EOF
  chmod +x target/e2e-bin/moltis

  echo "Preserved E2E binary at target/e2e-bin/moltis-bin"
  echo "Preserved E2E zvec runtime at target/e2e-bin/$(basename "$zvec_library")"
fi

rm -rf \
  target/debug/build \
  target/debug/deps \
  target/debug/examples \
  target/debug/incremental \
  target/debug/.fingerprint \
  target/debug/.cargo-lock \
  target/debug/lib*.rlib \
  target/debug/lib*.rmeta \
  target/debug/*.d \
  target/debug/*.dSYM \
  target/cargo-timings

df -h .
