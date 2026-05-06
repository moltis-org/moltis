#!/usr/bin/env bash
set -euo pipefail

repo_root="${SUPERSET_ROOT_PATH:-}"
if [[ -z "${repo_root}" ]]; then
  common_git_dir="$(git rev-parse --path-format=absolute --git-common-dir)"
  repo_root="$(cd "${common_git_dir}/.." && pwd -P)"
fi
current_root="$(pwd -P)"

mise trust
./scripts/bd-worktree-attach.sh

envrc_source="${repo_root}/.envrc"
envrc_target="${current_root}/.envrc"
if [[ ! -f "${envrc_source}" ]]; then
  echo "superset setup: no .envrc at ${envrc_source}, skipping"
elif [[ "${envrc_source}" == "${envrc_target}" ]]; then
  echo "superset setup: already in root checkout, skipping .envrc copy"
else
  cp "${envrc_source}" ./
  direnv allow
fi
