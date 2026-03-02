#!/usr/bin/env bash

set -euo pipefail

usage() {
  cat <<'EOF'
Usage:
  just ship ["<commit-message>"] ["<pr-title>"] ["<pr-body>"]
  ./scripts/ship-pr.sh ["<commit-message>"] ["<pr-title>"] ["<pr-body>"]

Behavior:
  1) Stages all changes and commits (if there are changes)
  2) Pushes current branch to origin
  3) Creates a PR if one does not already exist for this branch
  4) Runs local validation against that PR
  5) Pushes again if validation auto-created a commit (e.g. Cargo.lock sync)
EOF
}

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
  usage
  exit 0
fi

commit_message="${1:-}"
pr_title="${2:-}"
pr_body="${3:-}"

is_doc_path() {
  local p="$1"
  case "$p" in
    docs/*|plans/*|README.md|CHANGELOG.md|SECURITY.md|*.md)
      return 0
      ;;
    *)
      return 1
      ;;
  esac
}

detect_area() {
  local p="$1"
  if [[ "$p" == crates/* ]]; then
    local rest="${p#crates/}"
    echo "${rest%%/*}"
    return
  fi
  if [[ "$p" == docs/* || "$p" == plans/* || "$p" == *.md ]]; then
    echo "docs"
    return
  fi
  if [[ "$p" == .claude/* || "$p" == .codex/* || "$p" == scripts/* || "$p" == justfile ]]; then
    echo "tooling"
    return
  fi
  local top="${p%%/*}"
  if [[ "$top" == "$p" ]]; then
    echo "repo"
  else
    echo "$top"
  fi
}

generate_commit_message() {
  local files=("$@")
  if [[ ${#files[@]} -eq 0 ]]; then
    echo "chore: update branch state"
    return
  fi

  local all_docs=1
  local f
  for f in "${files[@]}"; do
    if ! is_doc_path "$f"; then
      all_docs=0
      break
    fi
  done
  if [[ "$all_docs" -eq 1 ]]; then
    echo "docs: update documentation"
    return
  fi

  local -A area_counts=()
  local area
  for f in "${files[@]}"; do
    area="$(detect_area "$f")"
    area_counts["$area"]=$(( ${area_counts["$area"]:-0} + 1 ))
  done

  local area_count=0
  local top_area=""
  local top_count=0
  local k
  for k in "${!area_counts[@]}"; do
    area_count=$((area_count + 1))
    if (( area_counts["$k"] > top_count )); then
      top_area="$k"
      top_count=${area_counts["$k"]}
    fi
  done

  if [[ "$area_count" -eq 1 ]]; then
    if [[ "$top_area" == "docs" ]]; then
      echo "docs: update documentation"
    elif [[ "$top_area" == "tooling" || "$top_area" == "repo" ]]; then
      echo "chore: update tooling"
    else
      echo "chore(${top_area}): update related files"
    fi
  else
    echo "chore: update multiple areas"
  fi
}

join_areas() {
  local -A area_counts=()
  local p
  for p in "$@"; do
    local area
    area="$(detect_area "$p")"
    area_counts["$area"]=$(( ${area_counts["$area"]:-0} + 1 ))
  done

  local parts=()
  local key
  for key in "${!area_counts[@]}"; do
    parts+=("${key} (${area_counts[$key]})")
  done
  if [[ ${#parts[@]} -eq 0 ]]; then
    echo "none"
  else
    printf '%s\n' "${parts[@]}" | sort | paste -sd ', ' -
  fi
}

generate_pr_title_from_branch() {
  local b="$1"
  b="${b//_/ }"
  b="${b//-/ }"
  echo "chore: ${b}"
}

generate_pr_body() {
  local branch_name="$1"
  shift
  local files=("$@")
  local areas
  areas="$(join_areas "${files[@]}")"
  {
    echo "## Summary"
    echo "- Automated ship via \`/ship\`."
    echo "- Branch: \`$branch_name\`"
    echo "- Changed files: ${#files[@]}"
    echo "- Areas: ${areas}"
    echo
    echo "## Changed Files"
    if [[ ${#files[@]} -eq 0 ]]; then
      echo "- No file changes in this push."
    else
      local max=40
      local i=0
      local file
      for file in "${files[@]}"; do
        echo "- \`$file\`"
        i=$((i + 1))
        if (( i >= max )); then
          local remaining=$(( ${#files[@]} - max ))
          if (( remaining > 0 )); then
            echo "- ... and ${remaining} more"
          fi
          break
        fi
      done
    fi
  }
}

if ! command -v gh >/dev/null 2>&1; then
  echo "gh CLI is required (install GitHub CLI first)." >&2
  exit 1
fi

repo_root="$(git rev-parse --show-toplevel)"
cd "$repo_root"

branch="$(git rev-parse --abbrev-ref HEAD)"
if [[ "$branch" == "HEAD" ]]; then
  echo "Detached HEAD is not supported. Check out a branch first." >&2
  exit 1
fi

if [[ "$branch" == "main" || "$branch" == "master" ]]; then
  echo "Refusing to run on $branch. Create/switch to a feature branch first." >&2
  exit 1
fi

changed_files=()
if [[ -n "$(git status --porcelain)" ]]; then
  while IFS= read -r line; do
    [[ -z "$line" ]] && continue
    path="${line:3}"
    path="${path#* -> }"
    changed_files+=("$path")
  done < <(git status --porcelain)
fi

if [[ -z "$commit_message" ]]; then
  commit_message="$(generate_commit_message "${changed_files[@]}")"
fi
if [[ -z "$pr_title" ]]; then
  if [[ -n "$commit_message" ]]; then
    pr_title="$commit_message"
  else
    pr_title="$(generate_pr_title_from_branch "$branch")"
  fi
fi
if [[ -z "$pr_body" ]]; then
  pr_body="$(generate_pr_body "$branch" "${changed_files[@]}")"
fi

if [[ ${#changed_files[@]} -gt 0 ]]; then
  git add -A
  git commit -m "$commit_message"
else
  echo "No local changes to commit."
fi

git push -u origin "$branch"

existing_pr_number="$(gh pr view --json number -q .number 2>/dev/null || true)"
if [[ -n "$existing_pr_number" ]]; then
  pr_number="$existing_pr_number"
else
  base_branch="${SHIP_BASE_BRANCH:-}"
  if [[ -z "$base_branch" ]]; then
    base_branch="$(
      git symbolic-ref --short refs/remotes/origin/HEAD 2>/dev/null \
        | sed 's@^origin/@@' \
        || true
    )"
  fi
  base_branch="${base_branch:-main}"

  gh pr create --base "$base_branch" --title "$pr_title" --body "$pr_body" >/dev/null
  pr_number="$(gh pr view --json number -q .number)"
fi

head_before_validation="$(git rev-parse HEAD)"
"$repo_root/scripts/local-validate.sh" "$pr_number"
head_after_validation="$(git rev-parse HEAD)"
if [[ "$head_after_validation" != "$head_before_validation" ]]; then
  git push origin "$branch"
fi

pr_url="$(gh pr view --json url -q .url)"
echo "Done. PR: $pr_url"
