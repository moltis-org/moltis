#!/usr/bin/env bash
set -euo pipefail

current_root="$(git rev-parse --show-toplevel)"
common_git_dir="$(git rev-parse --path-format=absolute --git-common-dir)"
main_root="$(cd "${common_git_dir}/.." && pwd -P)"

if [[ "${current_root}" == "${main_root}" ]]; then
  echo "bd-worktree-attach: current checkout is the main repo, no redirect needed" >&2
  exit 0
fi

main_beads_dir="${main_root}/.beads"
if [[ ! -d "${main_beads_dir}" ]]; then
  echo "bd-worktree-attach: main repo beads directory not found at ${main_beads_dir}" >&2
  exit 1
fi

mkdir -p "${current_root}/.beads"
printf '%s\n' "${main_beads_dir}" > "${current_root}/.beads/redirect"

echo "Attached Beads worktree redirect:"
echo "  worktree: ${current_root}"
echo "  beads:    ${main_beads_dir}"
