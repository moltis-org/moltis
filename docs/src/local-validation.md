# Local Validation

Moltis provides a local validation script that runs the same checks as CI
(format, lint, test, e2e) on your machine.

## Why this exists

- Faster feedback for Rust-heavy branches (no long runner queues for every push)
- Better parity with a developer's local environment while iterating
- Clear visibility in the PR UI (`fmt`, `biome`, `zizmor`, `clippy`, `test`, `e2e`)

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
- `local/e2e` — runs gateway UI Playwright coverage

In PR mode, the PR workflow verifies these contexts and surfaces them as
checks in the PR.

## Notes

- The script requires a clean working tree (no uncommitted or untracked
  changes). Commit or stash local changes before running.
- On macOS without CUDA (`nvcc`), the script automatically falls back to
  non-CUDA test/coverage defaults for local runs.
- `local/lint` uses the same clippy flags as CI and release:
  `cargo +nightly-2025-11-30 clippy -Z unstable-options --workspace --all-features --all-targets --timings -- -D warnings`.
- `zizmor` is installed automatically (Homebrew on macOS, apt on Linux) when
  not already available.
- `zizmor` is advisory in local runs and does not block lint/test execution.
- Test output is suppressed unless tests fail.
- `local/e2e` auto-runs `npm ci` only when `crates/gateway/ui/node_modules`
  is missing, then runs `npm run e2e:install` and `npm run e2e`. Override with
  `LOCAL_VALIDATE_E2E_CMD`.

## Merge and release safety

This local-first flow is for pull requests. Full CI still runs on GitHub
runners for non-PR events (for example push to `main`, scheduled runs, and
release paths).
