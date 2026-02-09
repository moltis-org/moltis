use std::path::PathBuf;

/// Returns the configured Moltis config directory.
///
/// Resolution order comes from `moltis_config::config_dir()`:
/// 1. programmatic override (`set_config_dir`)
/// 2. `MOLTIS_CONFIG_DIR`
/// 3. `~/.config/moltis`
pub fn moltis_config_dir() -> PathBuf {
    moltis_config::config_dir().unwrap_or_else(|| PathBuf::from(".config/moltis"))
}
