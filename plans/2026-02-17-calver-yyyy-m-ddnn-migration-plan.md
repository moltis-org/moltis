# CalVer Migration Plan (`YYYY.M.<DDNN>`)

Date: 2026-02-17
Owner: release/tooling
Status: planned

## Decision

Adopt calendar-based versions while keeping Cargo-compatible `major.minor.patch` syntax:

- `major = YYYY` (UTC year)
- `minor = M` (UTC month, `1..12`, no zero padding)
- `patch = day * 100 + sequence`
  - `day` = UTC day of month (`1..31`)
  - `sequence` = release sequence for that UTC day (`0..99`)
  - Logical interpretation is still `DDNN`, but serialized as an integer to avoid invalid leading zeros in SemVer/Cargo.

Examples:

- First release on 2026-02-17: `2026.2.1700`
- Second release on 2026-02-17: `2026.2.1701`
- First release on 2026-03-01: `2026.3.100`

Git tag format remains unchanged: `v<version>` (example `v2026.2.1700`).

## Why This Works

- Cargo requires SemVer-shaped strings in `Cargo.toml`; `2026.2.1700` is valid.
- Existing release workflows already consume `v*` tags and derive version from tag text.
- Existing update-check logic compares numeric triplets, so the cutover from `0.8.x` to `2026.x.x` is naturally treated as newer.

## Constraints and Guardrails

- Always compute version in UTC.
- Keep exactly three numeric components.
- Do not zero-pad month.
- Patch must stay numeric; no suffixes in stable releases.
- No leading zeros in any component (Cargo rejects them).
- Daily release cap is `100` (`NN = 00..99`).

## Migration Scope

Policy/docs:

- `CHANGELOG.md` (versioning policy line)
- `.github/workflows/homebrew.yml` (input example text)
- `.github/ISSUE_TEMPLATE/bug_report.yml` (version placeholder examples)
- `.github/ISSUE_TEMPLATE/model_behavior.yml` (version placeholder examples)
- `scripts/prepare-release.sh` usage examples and help text

Tooling/automation behavior:

- Keep `scripts/prepare-release.sh` semver-shape validation, add optional CalVer-specific validation path for this scheme.
- Keep release tag trigger as `v*` in `.github/workflows/release.yml`.
- Keep Docker metadata version tags enabled (`type=semver`) since versions remain SemVer-shaped.

Runtime/version compare tests:

- `crates/gateway/src/update_check.rs` tests should include CalVer cutover coverage (`0.8.37` -> `2026.2.1700`) and same-day sequence ordering.

## Implementation Plan

## Phase 1: Codify the scheme in docs and examples

1. Update changelog policy text to say CalVer-in-SemVer-shape (not SemVer policy).
2. Replace example versions (`0.x.y`) with CalVer examples in workflow/help/templates.
3. Add one short note in release docs that tags remain `v<version>`.

## Phase 2: Harden release prep script

1. Keep current argument shape: `./scripts/prepare-release.sh <version> [release-date]`.
2. Add CalVer checker:
   - `major` equals release-date year
   - `minor` equals release-date month
   - parse `patch` as integer
   - `day = patch / 100` must equal release-date day
   - `sequence = patch % 100` must be in `0..99`
3. Keep strict failure messages so mismatched date/version fails fast.

Optional ergonomic follow-up:

1. Add `--seq <NN>` mode that computes version from date+sequence to avoid manual patch construction.

## Phase 3: Add update-check regression tests

1. Add tests proving:
   - `is_newer_version("2026.2.1700", "0.8.37") == true`
   - `is_newer_version("2026.2.1701", "2026.2.1700") == true`
   - `is_newer_version("2026.3.100", "2026.2.3199") == true`
2. Keep existing SemVer tests for backward compatibility behavior.

## Phase 4: Execute cutover release

1. Choose UTC release date and sequence (`NN`).
2. Run release prep, example:
   - `./scripts/prepare-release.sh 2026.2.1700 2026-02-17`
3. Commit and tag:
   - `v2026.2.1700`
4. Verify release outputs:
   - GitHub release assets named with `2026.2.1700`
   - Homebrew tap update succeeds
   - deploy templates update image tags to `2026.2.1700`
   - update banner reports latest version correctly

## Validation Checklist (for the migration PR)

- `just format-check`
- `just release-preflight`
- Targeted Rust tests for `update_check.rs`
- Dry-run release prep on a temp branch with a CalVer version
- Confirm no workflow step assumes `0.x.y` semantics

## Rollback Plan

If any external integration breaks on CalVer:

1. Pause tagging/releases.
2. Revert migration PR.
3. Cut an emergency SemVer-style patch from the previous release flow.
4. Reintroduce CalVer with integration-specific fixes validated in a dry run.

## Success Criteria

- First CalVer tag (`v2026.2.1700` or later) builds and publishes all artifacts.
- Install paths (installer, Homebrew, package managers, Docker tags) work unchanged.
- Gateway update check and UI banner correctly identify newer CalVer releases.
