---
description: Commit all changes, push branch, create/update PR, and run local validation
allowed-tools: Bash(git rev-parse:*), Bash(git status:*), Bash(git add:*), Bash(git commit:*), Bash(git push:*), Bash(gh pr view:*), Bash(gh pr create:*), Bash(scripts/local-validate.sh:*), Bash(./scripts/ship-pr.sh:*), Read
argument-hint: ["<commit message>"] ["<pr title>"] ["<pr body>"]
---

Run the one-shot ship flow from this repository:

1. Commit all files.
2. Push current branch.
3. Create PR if missing (or reuse current branch PR).
4. Run local validation.
5. Push again if validation auto-created a commit.

## Command

Use:

```bash
./scripts/ship-pr.sh $ARGUMENTS
```

## Notes

- This command refuses to run on `main`/`master` by design.
- You can run plain `/ship` with no arguments.
- If args are omitted, commit message / PR title / PR body are auto-generated from changed files and branch.
- Set `SHIP_BASE_BRANCH=...` before running if you need a base branch other than `main`.
