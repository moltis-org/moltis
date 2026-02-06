use std::{
    net::TcpListener,
    path::{Path, PathBuf},
    sync::Mutex,
};

use tracing::{debug, warn};

use crate::{env_subst::substitute_env, schema::MoltisConfig};

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

/// Set a custom config directory. When set, config discovery only looks in
/// this directory (project-local and user-global paths are skipped).
/// Can be called multiple times (e.g. in tests) — each call replaces the
/// previous override.
pub fn set_config_dir(path: PathBuf) {
    *CONFIG_DIR_OVERRIDE.lock().unwrap() = Some(path);
}

/// Clear the config directory override, restoring default discovery.
pub fn clear_config_dir() {
    *CONFIG_DIR_OVERRIDE.lock().unwrap() = None;
}

fn config_dir_override() -> Option<PathBuf> {
    CONFIG_DIR_OVERRIDE.lock().unwrap().clone()
}

/// Set a custom data directory. When set, `data_dir()` returns this path
/// instead of the default.
pub fn set_data_dir(path: PathBuf) {
    *DATA_DIR_OVERRIDE.lock().unwrap() = Some(path);
}

/// Clear the data directory override, restoring default discovery.
pub fn clear_data_dir() {
    *DATA_DIR_OVERRIDE.lock().unwrap() = None;
}

fn data_dir_override() -> Option<PathBuf> {
    DATA_DIR_OVERRIDE.lock().unwrap().clone()
}

/// Load config from the given path (any supported format).
///
/// After parsing, `MOLTIS_*` env vars are applied as overrides.
pub fn load_config(path: &Path) -> anyhow::Result<MoltisConfig> {
    let raw = std::fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!("failed to read {}: {e}", path.display()))?;
    let raw = substitute_env(&raw);
    let config = parse_config(&raw, path)?;
    Ok(apply_env_overrides(config))
}

/// Load and parse the config file with env substitution and includes.
pub fn load_config_value(path: &Path) -> anyhow::Result<serde_json::Value> {
    let raw = std::fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!("failed to read {}: {e}", path.display()))?;
    let raw = substitute_env(&raw);
    parse_config_value(&raw, path)
}

/// Discover and load config from standard locations.
///
/// Search order:
/// 1. `./moltis.{toml,yaml,yml,json}` (project-local)
/// 2. `~/.config/moltis/moltis.{toml,yaml,yml,json}` (user-global)
///
/// Returns `MoltisConfig::default()` if no config file is found.
///
/// If the config has port 0 (either from defaults or missing `[server]` section),
/// a random available port is generated and saved to the config file.
pub fn discover_and_load() -> MoltisConfig {
    if let Some(path) = find_config_file() {
        debug!(path = %path.display(), "loading config");
        match load_config(&path) {
            Ok(mut cfg) => {
                // If port is 0 (default/missing), generate a random port and save it
                if cfg.server.port == 0 {
                    cfg.server.port = generate_random_port();
                    debug!(
                        port = cfg.server.port,
                        "generated random port for existing config"
                    );
                    if let Err(e) = save_config(&cfg) {
                        warn!(error = %e, "failed to save config with generated port");
                    }
                }
                return cfg; // env overrides already applied by load_config
            },
            Err(e) => {
                warn!(path = %path.display(), error = %e, "failed to load config, using defaults");
            },
        }
    } else {
        debug!("no config file found, writing default config with random port");
        let mut config = MoltisConfig::default();
        // Generate a unique port for this installation
        config.server.port = generate_random_port();
        if let Err(e) = write_default_config(&config) {
            warn!(error = %e, "failed to write default config file");
        }
        return apply_env_overrides(config);
    }
    apply_env_overrides(MoltisConfig::default())
}

/// Find the first config file in standard locations.
///
/// When a config dir override is set, only that directory is searched —
/// project-local and user-global paths are skipped for isolation.
pub fn find_config_file() -> Option<PathBuf> {
    if let Some(dir) = config_dir_override() {
        for name in CONFIG_FILENAMES {
            let p = dir.join(name);
            if p.exists() {
                return Some(p);
            }
        }
        // Override is set — don't fall through to other locations.
        return None;
    }

    // Project-local
    for name in CONFIG_FILENAMES {
        let p = PathBuf::from(name);
        if p.exists() {
            return Some(p);
        }
    }

    // User-global: ~/.config/moltis/
    if let Some(dir) = home_dir().map(|h| h.join(".config").join("moltis")) {
        for name in CONFIG_FILENAMES {
            let p = dir.join(name);
            if p.exists() {
                return Some(p);
            }
        }
    }

    None
}

/// Returns the config directory: programmatic override → `MOLTIS_CONFIG_DIR` env →
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

/// Returns the data directory: programmatic override → `MOLTIS_DATA_DIR` env →
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

fn home_dir() -> Option<PathBuf> {
    directories::BaseDirs::new().map(|d| d.home_dir().to_path_buf())
}

/// Returns the path of an existing config file, or the default TOML path.
pub fn find_or_default_config_path() -> PathBuf {
    if let Some(path) = find_config_file() {
        return path;
    }
    config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("moltis.toml")
}

/// Lock guarding config read-modify-write cycles.
static CONFIG_SAVE_LOCK: Mutex<()> = Mutex::new(());

/// Atomically load the current config, apply `f`, and save.
///
/// Acquires a process-wide lock so concurrent callers cannot race.
/// Returns the path written to.
pub fn update_config(f: impl FnOnce(&mut MoltisConfig)) -> anyhow::Result<PathBuf> {
    let _guard = CONFIG_SAVE_LOCK.lock().unwrap();
    let mut config = discover_and_load();
    f(&mut config);
    save_config_inner(&config)
}

/// Serialize `config` to TOML and write it to the user-global config path.
///
/// Creates parent directories if needed. Returns the path written to.
///
/// Prefer [`update_config`] for read-modify-write cycles to avoid races.
pub fn save_config(config: &MoltisConfig) -> anyhow::Result<PathBuf> {
    let _guard = CONFIG_SAVE_LOCK.lock().unwrap();
    save_config_inner(config)
}

fn save_config_inner(config: &MoltisConfig) -> anyhow::Result<PathBuf> {
    let path = find_or_default_config_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let toml_str =
        toml::to_string_pretty(config).map_err(|e| anyhow::anyhow!("serialize config: {e}"))?;
    std::fs::write(&path, toml_str)?;
    debug!(path = %path.display(), "saved config");
    Ok(path)
}

/// Write the default config file to the user-global config path.
/// Only called when no config file exists yet.
fn write_default_config(config: &MoltisConfig) -> anyhow::Result<()> {
    let path = find_or_default_config_path();
    if path.exists() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let toml_str =
        toml::to_string_pretty(config).map_err(|e| anyhow::anyhow!("serialize config: {e}"))?;
    std::fs::write(&path, &toml_str)?;
    debug!(path = %path.display(), "wrote default config file");
    Ok(())
}

/// Apply `MOLTIS_*` environment variable overrides to a loaded config.
///
/// Maps env vars to config fields using `__` as a section separator and
/// lowercasing. For example:
/// - `MOLTIS_AUTH_DISABLED=true` → `auth.disabled = true`
/// - `MOLTIS_TOOLS_EXEC_DEFAULT_TIMEOUT_SECS=60` → `tools.exec.default_timeout_secs = 60`
/// - `MOLTIS_CHAT_MESSAGE_QUEUE_MODE=collect` → `chat.message_queue_mode = "collect"`
///
/// The config is serialized to a JSON value, env overrides are merged in,
/// then deserialized back. Only env vars with the `MOLTIS_` prefix are
/// considered. `MOLTIS_CONFIG_DIR`, `MOLTIS_DATA_DIR`, `MOLTIS_ASSETS_DIR`,
/// `MOLTIS_TOKEN`, `MOLTIS_PASSWORD`, `MOLTIS_TAILSCALE`,
/// `MOLTIS_WEBAUTHN_RP_ID`, and `MOLTIS_WEBAUTHN_ORIGIN` are excluded
/// (they are handled separately).
pub fn apply_env_overrides(config: MoltisConfig) -> MoltisConfig {
    apply_env_overrides_with(config, std::env::vars())
}

/// Apply env overrides from an arbitrary iterator of (key, value) pairs.
/// Exposed for testing without mutating the process environment.
fn apply_env_overrides_with(
    config: MoltisConfig,
    vars: impl Iterator<Item = (String, String)>,
) -> MoltisConfig {
    use serde_json::Value;

    const EXCLUDED: &[&str] = &[
        "MOLTIS_CONFIG_DIR",
        "MOLTIS_DATA_DIR",
        "MOLTIS_ASSETS_DIR",
        "MOLTIS_TOKEN",
        "MOLTIS_PASSWORD",
        "MOLTIS_TAILSCALE",
        "MOLTIS_WEBAUTHN_RP_ID",
        "MOLTIS_WEBAUTHN_ORIGIN",
    ];

    let mut root: Value = match serde_json::to_value(&config) {
        Ok(v) => v,
        Err(e) => {
            warn!(error = %e, "failed to serialize config for env override");
            return config;
        },
    };

    for (key, val) in vars {
        if !key.starts_with("MOLTIS_") {
            continue;
        }
        if EXCLUDED.contains(&key.as_str()) {
            continue;
        }

        // MOLTIS_AUTH__DISABLED → ["auth", "disabled"]
        let path_parts: Vec<String> = key["MOLTIS_".len()..]
            .split("__")
            .map(|segment| segment.to_lowercase())
            .collect();

        if path_parts.is_empty() {
            continue;
        }

        // Navigate to the parent object and set the leaf value.
        let parsed_val = parse_env_value(&val);
        set_nested(&mut root, &path_parts, parsed_val);
    }

    match serde_json::from_value(root) {
        Ok(cfg) => cfg,
        Err(e) => {
            warn!(error = %e, "failed to apply env overrides, using config as-is");
            config
        },
    }
}

/// Parse a string env value into a JSON value, trying bool and number first.
fn parse_env_value(val: &str) -> serde_json::Value {
    if val.eq_ignore_ascii_case("true") {
        return serde_json::Value::Bool(true);
    }
    if val.eq_ignore_ascii_case("false") {
        return serde_json::Value::Bool(false);
    }
    if let Ok(n) = val.parse::<i64>() {
        return serde_json::Value::Number(n.into());
    }
    if let Ok(n) = val.parse::<f64>()
        && let Some(n) = serde_json::Number::from_f64(n)
    {
        return serde_json::Value::Number(n);
    }
    serde_json::Value::String(val.to_string())
}

/// Set a value at a nested JSON path, creating intermediate objects as needed.
fn set_nested(root: &mut serde_json::Value, path: &[String], val: serde_json::Value) {
    if path.is_empty() {
        return;
    }
    let mut current = root;
    for (i, key) in path.iter().enumerate() {
        if i == path.len() - 1 {
            if let serde_json::Value::Object(map) = current {
                map.insert(key.clone(), val);
            }
            return;
        }
        if !current.get(key).is_some_and(|v| v.is_object())
            && let serde_json::Value::Object(map) = current
        {
            map.insert(key.clone(), serde_json::Value::Object(Default::default()));
        }
        current = current.get_mut(key).unwrap();
    }
}

fn parse_config(raw: &str, path: &Path) -> anyhow::Result<MoltisConfig> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("toml");

    match ext {
        "toml" => Ok(toml::from_str(raw)?),
        "yaml" | "yml" => Ok(serde_yaml::from_str(raw)?),
        "json" => Ok(serde_json::from_str(raw)?),
        _ => anyhow::bail!("unsupported config format: .{ext}"),
    }
}

fn parse_config_value(raw: &str, path: &Path) -> anyhow::Result<serde_json::Value> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("toml");

    match ext {
        "toml" => {
            let v: toml::Value = toml::from_str(raw)?;
            Ok(serde_json::to_value(v)?)
        },
        "yaml" | "yml" => {
            let v: serde_yaml::Value = serde_yaml::from_str(raw)?;
            Ok(serde_json::to_value(v)?)
        },
        "json" => Ok(serde_json::from_str(raw)?),
        _ => anyhow::bail!("unsupported config format: .{ext}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_env_value_bool() {
        assert_eq!(parse_env_value("true"), serde_json::Value::Bool(true));
        assert_eq!(parse_env_value("TRUE"), serde_json::Value::Bool(true));
        assert_eq!(parse_env_value("false"), serde_json::Value::Bool(false));
    }

    #[test]
    fn parse_env_value_number() {
        assert_eq!(parse_env_value("42"), serde_json::json!(42));
        assert_eq!(parse_env_value("1.5"), serde_json::json!(1.5));
    }

    #[test]
    fn parse_env_value_string() {
        assert_eq!(
            parse_env_value("hello"),
            serde_json::Value::String("hello".into())
        );
    }

    #[test]
    fn set_nested_creates_intermediate_objects() {
        let mut root = serde_json::json!({});
        set_nested(
            &mut root,
            &["a".into(), "b".into(), "c".into()],
            serde_json::json!(42),
        );
        assert_eq!(root, serde_json::json!({"a": {"b": {"c": 42}}}));
    }

    #[test]
    fn set_nested_overwrites_existing() {
        let mut root = serde_json::json!({"auth": {"disabled": false}});
        set_nested(
            &mut root,
            &["auth".into(), "disabled".into()],
            serde_json::Value::Bool(true),
        );
        assert_eq!(root, serde_json::json!({"auth": {"disabled": true}}));
    }

    #[test]
    fn apply_env_overrides_auth_disabled() {
        let vars = vec![("MOLTIS_AUTH__DISABLED".into(), "true".into())];
        let config = MoltisConfig::default();
        assert!(!config.auth.disabled);
        let config = apply_env_overrides_with(config, vars.into_iter());
        assert!(config.auth.disabled);
    }

    #[test]
    fn apply_env_overrides_tools_agent_timeout() {
        let vars = vec![("MOLTIS_TOOLS__AGENT_TIMEOUT_SECS".into(), "120".into())];
        let config = apply_env_overrides_with(MoltisConfig::default(), vars.into_iter());
        assert_eq!(config.tools.agent_timeout_secs, 120);
    }

    #[test]
    fn apply_env_overrides_ignores_excluded() {
        // MOLTIS_CONFIG_DIR should not be treated as a config field override.
        let vars = vec![("MOLTIS_CONFIG_DIR".into(), "/tmp/test".into())];
        let config = apply_env_overrides_with(MoltisConfig::default(), vars.into_iter());
        assert!(!config.auth.disabled);
    }

    #[test]
    fn apply_env_overrides_multiple() {
        let vars = vec![
            ("MOLTIS_AUTH__DISABLED".into(), "true".into()),
            ("MOLTIS_TOOLS__AGENT_TIMEOUT_SECS".into(), "300".into()),
            ("MOLTIS_TAILSCALE__MODE".into(), "funnel".into()),
        ];
        let config = apply_env_overrides_with(MoltisConfig::default(), vars.into_iter());
        assert!(config.auth.disabled);
        assert_eq!(config.tools.agent_timeout_secs, 300);
        assert_eq!(config.tailscale.mode, "funnel");
    }

    #[test]
    fn apply_env_overrides_deep_nesting() {
        let vars = vec![(
            "MOLTIS_TOOLS__EXEC__DEFAULT_TIMEOUT_SECS".into(),
            "60".into(),
        )];
        let config = apply_env_overrides_with(MoltisConfig::default(), vars.into_iter());
        assert_eq!(config.tools.exec.default_timeout_secs, 60);
    }

    #[test]
    fn generate_random_port_returns_valid_port() {
        // Generate a few random ports and verify they're in the valid range
        for _ in 0..5 {
            let port = generate_random_port();
            // Port should be in the ephemeral range (1024-65535) or fallback (18789)
            assert!(
                port >= 1024 || port == 0,
                "generated port {port} is out of expected range"
            );
        }
    }

    #[test]
    fn generate_random_port_returns_different_ports() {
        // Generate multiple ports and verify we get at least some variation
        let ports: Vec<u16> = (0..10).map(|_| generate_random_port()).collect();
        let unique: std::collections::HashSet<_> = ports.iter().collect();
        // With 10 random ports, we should have at least 2 different values
        // (unless somehow all ports are in use, which is extremely unlikely)
        assert!(
            unique.len() >= 2,
            "expected variation in generated ports, got {:?}",
            ports
        );
    }

    #[test]
    fn server_config_default_port_is_zero() {
        // Default port should be 0 (to be replaced with random port on config creation)
        let config = crate::schema::ServerConfig::default();
        assert_eq!(config.port, 0);
        assert_eq!(config.bind, "127.0.0.1");
    }

    #[test]
    fn data_dir_override_works() {
        let path = PathBuf::from("/tmp/test-data-dir-override");
        set_data_dir(path.clone());
        assert_eq!(data_dir(), path);
        clear_data_dir();
    }
}
