# Third-Party Skills Security

Third-party skills and plugin repos are powerful and risky. Treat them like
untrusted code until reviewed.

## Trust Lifecycle

Installed skills use a trust gate with three fields per skill:

- `trusted` - you explicitly marked the skill as reviewed (defaults to `true`
  for backward compatibility with pre-trust-gate manifests)
- `enabled` - skill is active for agent use
- `quarantined` - repo-level flag; blocks all skills until explicitly cleared

You cannot enable untrusted skills.

## Skill Sources

Skills are discovered from four source types:

| Source | Location | Description |
|--------|----------|-------------|
| `Project` | `<data_dir>/.moltis/skills/` | Project-local skills |
| `Personal` | `<data_dir>/skills/` | Personal skills across projects |
| `Plugin` | Plugin directory | Bundled with a plugin repo |
| `Registry` | Installed repos | Installed from a registry (e.g. skills.sh) |

## Prompt Injection Scanning

When a skill is read via `read_skill`, its body is scanned for known
prompt-injection patterns (e.g. "ignore previous instructions", "system
prompt:"). Matches are logged as warnings but never block the read.

Patterns are case-insensitive and kept conservative to minimize false positives.

## Portable Bundle Import/Export

Installed repos can be exported to `.tar.gz` bundles and re-imported:

- Bundles include a `bundle.json` manifest with repo metadata and provenance
- Imported bundles are automatically quarantined (`quarantined = true`)
- All skills in a quarantined repo start as `trusted=false, enabled=false`
- Use `skills.repos.unquarantine` to clear quarantine after reviewing contents
- Bundle archives reject symlinks, hard links, and path traversal attempts

Imported repos keep provenance metadata (`RepoProvenance`):

- `original_source` - original repo source identifier
- `original_commit_sha` - commit SHA at export time (when available)
- `imported_from` - path of the imported bundle
- `exported_at_ms` - export timestamp

## Provenance

Installed repos record a `commit_sha` pinned at install time. The Skills UI
shows a short SHA to help review provenance.

## Live Skill Watching

A filesystem watcher monitors skill directories for `SKILL.md` and
`skills-manifest.json` changes. When a skill file is modified, the gateway
broadcasts a `skills.changed` event so the agent reloads without restart.

## Recommended Production Policy

1. Keep sandbox enabled (`tools.exec.sandbox.mode = "all"`).
2. Keep approval mode at least `on-miss`.
3. Review SKILL.md and linked scripts before trusting.
4. Prefer pinned, known repos over ad-hoc installs.
5. Keep imported bundles quarantined until you review their contents locally.
