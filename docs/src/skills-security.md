# Third-Party Skills Security

Third-party skills and plugin repos are powerful and risky. Treat them like
untrusted code until reviewed.

## Trust Lifecycle

Installed marketplace skills/plugins now use a trust gate:

- `installed` - repo is on disk
- `trusted` - you explicitly marked the skill as reviewed
- `enabled` - skill is active for agent use

You cannot enable untrusted skills.

## Provenance Pinning

Moltis records a pinned `commit_sha` for installed repos:

- via `git rev-parse HEAD` after clone
- via GitHub commits API for tarball fallback installs

The Skills UI shows a short SHA to help review provenance.

## Re-Trust on Drift

If local repo HEAD changes from the pinned `commit_sha`:

- all skills in that repo are auto-marked `trusted=false`
- all skills in that repo are auto-disabled
- re-enable is blocked until explicit trust is granted again

The UI/API mark this state as `source changed`.

## Dependency Install Guardrails

`skills.install_dep` now includes hard gates:

- explicit `confirm=true` required
- host installs blocked when sandbox mode is off (unless explicit override)
- suspicious command chains are blocked by default (for example `curl ... | sh`,
  base64 decode chains, quarantine bypass)

For high-risk overrides, require manual review before using
`allow_risky_install=true`.

## Emergency Kill Switch

Use `skills.emergency_disable` to disable all installed third-party skills and
plugins immediately.

- Available in RPC and Skills UI action button
- Intended for incident response and containment

## Security Audit Log

Security-sensitive skill/plugin actions are appended to:

`~/.moltis/logs/security-audit.jsonl`

Logged events include installs, removals, trust changes, enable/disable,
dependency install attempts, and source drift detection.

## Recommended Production Policy

1. Keep sandbox enabled (`tools.exec.sandbox.mode = "all"`).
2. Keep approval mode at least `on-miss`.
3. Review SKILL.md and linked scripts before trust.
4. Prefer pinned, known repos over ad-hoc installs.
5. Monitor `security-audit.jsonl` for unusual events.
