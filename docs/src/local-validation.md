# Local Validation

Moltis provides a local validation script that runs the same checks as CI
(format, lint, test) on your machine.

## Why this exists

- Faster feedback for Rust-heavy branches (no long runner queues for every push)
- Better parity with a developer's local environment while iterating
- Clear visibility in the PR UI (`fmt`, `biome`, `zizmor`, `clippy`, `test`)

## Run local validation

Run all checks on your current checkout:

```bash
./scripts/local-validate.sh
```

When working on a pull request, pass the PR number to also publish commit
statuses to GitHub:

```bash
./scripts/local-validate.sh 63
```

The script runs these checks:

- `local/fmt`
- `local/biome`
- `local/zizmor`
- `local/lockfile` — verifies `Cargo.lock` is in sync (`cargo fetch --locked`)
- `local/lint`
- `local/test`

In PR mode, the PR workflow verifies these contexts and surfaces them as
checks in the PR.

## Notes

- The script requires a clean working tree (no uncommitted or untracked
  changes). Commit or stash local changes before running.
- On macOS without CUDA (`nvcc`), the script automatically falls back to
  non-CUDA lint/test defaults for local runs.
- `zizmor` is installed automatically (Homebrew on macOS, apt on Linux) when
  not already available.
- `zizmor` is advisory in local runs and does not block lint/test execution.
- Test output is suppressed unless tests fail.

## Merge and release safety

This local-first flow is for pull requests. Full CI still runs on GitHub
runners for non-PR events (for example push to `main`, scheduled runs, and
release paths).
