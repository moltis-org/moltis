# Local PR Validation

Moltis uses a local-first pull request flow: contributors run validation checks
on their own machine and publish the results to GitHub commit statuses.

## Why this exists

- Faster feedback for Rust-heavy branches (no long runner queues for every PR push)
- Better parity with a developer's local environment while iterating
- Clear visibility in the PR UI (`fmt`, `biome`, `zizmor`, `clippy`, `test`)

## Run local validation for a PR

Use the PR number:

```bash
./scripts/local-validate-pr.sh 63
```

The script publishes status checks for the current PR head commit:

- `local/fmt`
- `local/biome`
- `local/zizmor`
- `local/lint`
- `local/test`

The PR workflow verifies these contexts and surfaces them as checks in the PR.

## Notes

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
