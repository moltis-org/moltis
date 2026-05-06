use std::{net::TcpListener, path::PathBuf, sync::Mutex};

#[path = "loader/config_io.rs"]
mod config_io;
#[path = "loader/workspace.rs"]
mod workspace;

pub use {config_io::*, workspace::*};

/// Generate a random available port by binding to port 0 and reading the assigned port.
fn generate_random_port() -> u16 {
    // Bind to port 0 to get an OS-assigned available port
    TcpListener::bind("127.0.0.1:0")
        .and_then(|listener| listener.local_addr())
        .map(|addr| addr.port())
        .unwrap_or(18789) // Fallback to default if binding fails
}

/// Standard config file names, checked in order.
const CONFIG_FILENAMES: &[&str] = &["moltis.toml", "moltis.yaml", "moltis.yml", "moltis.json"];

/// Override for the config directory, set via `set_config_dir()`.
static CONFIG_DIR_OVERRIDE: Mutex<Option<PathBuf>> = Mutex::new(None);

/// Override for the data directory, set via `set_data_dir()`.
static DATA_DIR_OVERRIDE: Mutex<Option<PathBuf>> = Mutex::new(None);

/// Override for the share directory, set via `set_share_dir()`.
static SHARE_DIR_OVERRIDE: Mutex<Option<PathBuf>> = Mutex::new(None);

/// Set a custom config directory. When set, config discovery only looks in
/// this directory (project-local and user-global paths are skipped).
/// Can be called multiple times (e.g. in tests) - each call replaces the
/// previous override.
pub fn set_config_dir(path: PathBuf) {
    *CONFIG_DIR_OVERRIDE
        .lock()
        .unwrap_or_else(|e| e.into_inner()) = Some(path);
}

/// Clear the config directory override, restoring default discovery.
pub fn clear_config_dir() {
    *CONFIG_DIR_OVERRIDE
        .lock()
        .unwrap_or_else(|e| e.into_inner()) = None;
}

fn config_dir_override() -> Option<PathBuf> {
    CONFIG_DIR_OVERRIDE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
}

/// Set a custom data directory. When set, `data_dir()` returns this path
/// instead of the default.
pub fn set_data_dir(path: PathBuf) {
    *DATA_DIR_OVERRIDE.lock().unwrap_or_else(|e| e.into_inner()) = Some(path);
}

/// Clear the data directory override, restoring default discovery.
pub fn clear_data_dir() {
    *DATA_DIR_OVERRIDE.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

fn data_dir_override() -> Option<PathBuf> {
    DATA_DIR_OVERRIDE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
}

/// Set a custom share directory (for tests or alternative layouts).
pub fn set_share_dir(path: PathBuf) {
    *SHARE_DIR_OVERRIDE.lock().unwrap_or_else(|e| e.into_inner()) = Some(path);
}

/// Clear the share directory override, restoring default discovery.
pub fn clear_share_dir() {
    *SHARE_DIR_OVERRIDE.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

fn share_dir_override() -> Option<PathBuf> {
    SHARE_DIR_OVERRIDE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
}

/// Returns the share directory for external assets (web files, WASM components).
///
/// Resolution order:
/// 1. Programmatic override via `set_share_dir()`
/// 2. `MOLTIS_SHARE_DIR` env var
/// 3. `/usr/share/moltis/` (Linux system packages) - only if it exists
/// 4. `data_dir()/share/` (`~/.moltis/share/`) - only if it exists
/// 5. `None` (fall back to embedded assets)
pub fn share_dir() -> Option<PathBuf> {
    if let Some(dir) = share_dir_override() {
        return Some(dir);
    }
    if let Ok(dir) = std::env::var("MOLTIS_SHARE_DIR")
        && !dir.is_empty()
    {
        return Some(PathBuf::from(dir));
    }
    let system = PathBuf::from("/usr/share/moltis");
    if system.is_dir() {
        return Some(system);
    }
    let user = data_dir().join("share");
    if user.is_dir() {
        return Some(user);
    }
    None
}

/// Returns the user's home directory (`$HOME` / `~`).
///
/// This is the **single call-site** for `directories::BaseDirs` - all other
/// crates must call this via `moltis_config::home_dir()` instead of using the
/// `directories` crate directly.
pub fn home_dir() -> Option<PathBuf> {
    directories::BaseDirs::new().map(|d| d.home_dir().to_path_buf())
}

/// Returns the config directory: programmatic override -> `MOLTIS_CONFIG_DIR` env ->
/// `~/.config/moltis/`.
pub fn config_dir() -> Option<PathBuf> {
    if let Some(dir) = config_dir_override() {
        return Some(dir);
    }
    if let Ok(dir) = std::env::var("MOLTIS_CONFIG_DIR")
        && !dir.is_empty()
    {
        return Some(PathBuf::from(dir));
    }
    home_dir().map(|h| h.join(".config").join("moltis"))
}

/// Returns the user-global config directory (`~/.config/moltis`) without
/// considering overrides like `MOLTIS_CONFIG_DIR`.
pub fn user_global_config_dir() -> Option<PathBuf> {
    home_dir().map(|h| h.join(".config").join("moltis"))
}

/// Returns the user-global config directory only when it differs from the
/// active config directory (i.e. when `MOLTIS_CONFIG_DIR` or `--config-dir`
/// is overriding the default). Returns `None` when they are the same path.
pub fn user_global_config_dir_if_different() -> Option<PathBuf> {
    let home = user_global_config_dir()?;
    let current = config_dir()?;
    if home == current {
        None
    } else {
        Some(home)
    }
}

/// Finds a config file in the user-global config directory only.
pub fn find_user_global_config_file() -> Option<PathBuf> {
    let dir = user_global_config_dir()?;
    for name in CONFIG_FILENAMES {
        let p = dir.join(name);
        if p.exists() {
            return Some(p);
        }
    }
    None
}

/// Returns the data directory: programmatic override -> `MOLTIS_DATA_DIR` env ->
/// `~/.moltis/`.
pub fn data_dir() -> PathBuf {
    if let Some(dir) = data_dir_override() {
        return dir;
    }
    if let Ok(dir) = std::env::var("MOLTIS_DATA_DIR")
        && !dir.is_empty()
    {
        return PathBuf::from(dir);
    }
    home_dir()
        .map(|h| h.join(".moltis"))
        .unwrap_or_else(|| PathBuf::from(".moltis"))
}

/// Path to the workspace soul file.
pub fn soul_path() -> PathBuf {
    data_dir().join("SOUL.md")
}

/// Path to the workspace AGENTS markdown.
pub fn agents_path() -> PathBuf {
    data_dir().join("AGENTS.md")
}

/// Path to the workspace identity file.
pub fn identity_path() -> PathBuf {
    data_dir().join("IDENTITY.md")
}

/// Path to the workspace user profile file.
pub fn user_path() -> PathBuf {
    data_dir().join("USER.md")
}

/// Path to workspace boot context markdown.
pub fn boot_path() -> PathBuf {
    data_dir().join("BOOT.md")
}

/// Path to workspace tool-guidance markdown.
pub fn tools_path() -> PathBuf {
    data_dir().join("TOOLS.md")
}

/// Path to workspace guidelines override markdown.
pub fn guidelines_path() -> PathBuf {
    data_dir().join("docs/moltis/GUIDELINES.md")
}

/// Path to workspace heartbeat markdown.
pub fn heartbeat_path() -> PathBuf {
    data_dir().join("HEARTBEAT.md")
}

/// Path to the workspace `MEMORY.md` file.
pub fn memory_path() -> PathBuf {
    data_dir().join("MEMORY.md")
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
#[path = "loader/tests.rs"]
mod tests;
