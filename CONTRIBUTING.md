# Contributing to Moltis

Thanks for contributing. This project is a local-first AI gateway written in Rust, and we welcome fixes, features, docs improvements, and test coverage.

## Ground Rules

- Keep changes focused and easy to review.
- Prefer small PRs over large rewrites.
- Add or update tests for every behavior change.
- Use conventional commit messages. Do not edit `CHANGELOG.md` in normal PRs, CI blocks manual edits.
- Never commit secrets, tokens, or private keys.

For security issues, do not open a public issue. Follow `SECURITY.md` instead.

## Development Setup

Prerequisites:

- Rust toolchain (stable)
- `rustup` with nightly `nightly-2025-11-30` available
- `just` task runner
- `git-cliff` changelog generator
- Node.js (for web UI e2e tests)
- `gh` CLI (optional, needed for PR status publishing via local validation)

Quick start:

```bash
git clone https://github.com/moltis-org/moltis.git
cd moltis
cargo build
cargo run
```

## Development Workflow

1. Create a branch from `main`.
2. Make your change, keeping commits scoped and readable.
3. Add/update tests.
4. Run validation locally.
5. Open a PR with a clear summary, validation output, and any manual QA notes.

## Share Full Context (Important)

When you open a PR, share as much implementation context as possible.

- Include the full chat/session export from the session UI whenever possible.
- Include key prompts, constraints, decisions, command outputs, and debugging notes.
- If you used AI assistance to build a feature, the full session is more valuable than the final code diff. Maintainers may re-implement with their own agents based on your session context.
- If you cannot share the full session, explain what is missing and why.

Before sharing any transcript or logs, redact sensitive data:

- API keys and private keys
- Access tokens, cookies, and passwords
- Any personal or confidential data

## Validation Commands

Use `just` targets when available.

Core checks:

```bash
just format-check
just release-preflight
just test
```

Changelog preview (for unreleased commits since the last tag):

```bash
just changelog-unreleased
```

Optional local guard check (matches CI behavior):

```bash
bash ./scripts/check-changelog-guard.sh origin/main HEAD
```

For web UI changes, run e2e tests:

```bash
just ui-e2e-install
just ui-e2e
```

For CI-parity local validation (format, lint, test, e2e, lockfile, workflow security):

```bash
./scripts/local-validate.sh
```

If you are working on an existing PR and have permissions to publish statuses:

```bash
./scripts/local-validate.sh <PR_NUMBER>
```

See also:

- `docs/src/local-validation.md`
- `docs/src/e2e-testing.md`

## Testing Expectations

- Rust changes should include unit/integration coverage.
- Web UI changes should include Playwright coverage in `crates/web/ui/e2e/specs/`.
- Prefer real behavior tests over heavy mocking.
- Keep tests deterministic and avoid timing-based flakiness.

## Style and Project Conventions

- Do not use `unwrap()` / `expect()` in production Rust code.
- Add new dependencies in workspace-level `Cargo.toml` (`[workspace.dependencies]`), then reference with `{ workspace = true }`.
- Use conventional commit style where possible:
  - `feat(scope): ...`
  - `fix(scope): ...`
  - `docs(scope): ...`
  - `refactor(scope): ...`
  - `test(scope): ...`
  - `chore(scope): ...`

## Pull Request Checklist

- [ ] Tests added or updated for changed behavior
- [ ] `just format-check` passes
- [ ] `just release-preflight` passes
- [ ] `just test` passes
- [ ] `just ui-e2e` run for web UI changes
- [ ] Commit messages follow conventional commit style
- [ ] Full session/context shared (or clear explanation if partial)
- [ ] Shared session/logs are redacted (no API keys, private keys, tokens, passwords)
- [ ] No secrets or sensitive data in the diff

## Where to Start

- Good first issues and feature requests: check the GitHub issue templates in `.github/ISSUE_TEMPLATE/`.
- Docs fixes are always welcome and usually fast to review.

If you are unsure about architecture for a larger change, open an issue first so we can align on approach before you spend cycles on implementation.
