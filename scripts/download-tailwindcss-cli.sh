#!/usr/bin/env bash

set -euo pipefail

if [[ $# -ne 1 ]]; then
  echo "usage: $0 <tailwind-asset-name>" >&2
  exit 2
fi

asset="$1"
url="https://github.com/tailwindlabs/tailwindcss/releases/latest/download/${asset}"

curl \
  --fail \
  --silent \
  --show-error \
  --location \
  --retry 5 \
  --retry-delay 2 \
  --retry-connrefused \
  --retry-all-errors \
  --output "${asset}" \
  "${url}"

if [[ ! -s "${asset}" ]]; then
  echo "downloaded ${asset} is empty" >&2
  exit 1
fi

magic="$(od -An -tx1 -N4 "${asset}" | tr -d ' \n')"

case "${asset}" in
  tailwindcss-linux-*)
    if [[ "${magic}" != "7f454c46" ]]; then
      echo "downloaded ${asset} does not look like an ELF binary (magic=${magic})" >&2
      exit 1
    fi
    ;;
  tailwindcss-macos-*)
    case "${magic}" in
      cffaedfe | cefaedfe | cafebabe | bebafeca) ;;
      *)
        echo "downloaded ${asset} does not look like a Mach-O binary (magic=${magic})" >&2
        exit 1
        ;;
    esac
    ;;
  *.exe)
    if [[ "${magic:0:4}" != "4d5a" ]]; then
      echo "downloaded ${asset} does not look like a PE executable (magic=${magic})" >&2
      exit 1
    fi
    ;;
esac

chmod +x "${asset}"
