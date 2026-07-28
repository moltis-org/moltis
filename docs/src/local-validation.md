# Local Validation

Moltis provides a local validation script that runs broad CI-style checks
(format, lint, build, line limits), plus targeted Rust and Playwright tests
for tests changed or added on the branch.

## Why this exists

- Faster feedback for Rust-heavy branches (no long runner queues for every push)
- Better parity with a developer's local environment while iterating
- Clear visibility in the PR UI (`fmt`, `biome`, `zizmor`, `clippy`, `test`, `macos-app`, `e2e`)

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

To intentionally run the full Rust and Playwright suites locally, use:

```bash
just local-validate-full
just local-validate-full 63
```

The script runs these checks:

- `local/fmt`
- `local/biome`
- `local/zizmor`
- `local/lockfile` — verifies `Cargo.lock` is in sync (`cargo fetch --locked`)
- `local/lint`
- `local/test` — runs only changed or added Rust tests from the branch diff
- `local/macos-app` — validates the native Swift macOS app build (`Darwin` only)
- `local/e2e` — runs only changed or added gateway UI Playwright specs
- `local/e2e-ollama` — opt-in live Ollama/Qwen Playwright regression check

In PR mode, the PR workflow verifies these contexts and surfaces them as
checks in the PR.

## Notes

- The script requires a clean working tree (no uncommitted or untracked
  changes). Commit or stash local changes before running.
- On macOS without CUDA (`nvcc`), the script automatically falls back to
  non-CUDA test/coverage defaults for local runs.
- On Linux, `local/lint` and `local/test` use `--all-features`. If you want
  the opt-in Vulkan path covered locally, install the Vulkan development
  packages first, for example `libvulkan-dev` and `glslang-tools` on Debian/Ubuntu
  (on Ubuntu 22.04, install `glslang-tools` from the LunarG Vulkan SDK).
- `local/lint` uses the same clippy flags as CI and release:
  `cargo clippy -Z unstable-options --workspace --all-features --all-targets --timings -- -D warnings` (uses the nightly pinned in `rust-toolchain.toml`).
- `zizmor` is installed automatically (Homebrew on macOS, apt on Linux) when
  not already available.
- `zizmor` is advisory in local runs and does not block lint/test execution.
- Test output is suppressed unless tests fail.
- `local/test` derives changed Rust tests from the branch diff. Integration
  tests under `crates/<crate>/tests/*.rs` run as specific test targets;
  changed in-source test modules run by package with a filename filter. Override
  with `LOCAL_VALIDATE_TEST_CMD` when a branch needs broader local coverage.
- `local/macos-app` runs only on macOS; on Linux it is marked skipped.
- Override or disable macOS app validation with:
  `LOCAL_VALIDATE_MACOS_APP_CMD` and `LOCAL_VALIDATE_SKIP_MACOS_APP=1`.
- `local/e2e` auto-runs `npm ci` only when `crates/web/ui/node_modules`
  is missing, then runs `npm run e2e:install` and changed Playwright specs from
  `crates/web/ui/e2e/specs/`. Override with `LOCAL_VALIDATE_E2E_CMD`.
- Enable the live Ollama/Qwen regression check with
  `LOCAL_VALIDATE_OLLAMA_QWEN_E2E=1`. It starts a local Ollama server on
  `MOLTIS_E2E_OLLAMA_QWEN_API_PORT` (default `11435`), pulls the configured
  Qwen model if missing, and runs the dedicated Playwright project. Override
  the command with `LOCAL_VALIDATE_OLLAMA_QWEN_E2E_CMD`.
- Coverage is opt-in because it runs Rust tests. Enable it with
  `LOCAL_VALIDATE_COVERAGE=1` when needed.

## Merge and release safety

This local-first flow is for pull requests. Full Rust and Playwright suites
still run on GitHub runners for CI/release paths where required.
