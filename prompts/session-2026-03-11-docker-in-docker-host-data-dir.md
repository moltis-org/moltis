# Session Summary: Docker-in-Docker host data dir detection

- Added `host_data_dir` to sandbox config schema, validation, and generated config template.
- Updated sandbox mount resolution so Docker/Podman workspace mounts and default home persistence paths first auto-detect a host-visible `data_dir()` from the parent container's mounts, with `host_data_dir` as an explicit override.
- Added regression tests covering inspect parsing, mount translation, workspace mount overrides, and shared/session home persistence translation.
- Updated Docker, sandbox, and configuration docs to describe automatic host-path detection plus the manual override.
- Validation run:
  - `cargo +nightly-2025-11-30 fmt --all`
  - `cargo test -p moltis-config -p moltis-tools`
