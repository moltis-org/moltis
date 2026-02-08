use std::{
    net::TcpListener,
    path::{Path, PathBuf},
    sync::Mutex,
};

use tracing::{debug, warn};

use crate::{
    env_subst::substitute_env,
    schema::{AgentIdentity, MoltisConfig, UserProfile},
};

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

/// Returns the user-global config directory (`~/.config/moltis`) without
/// considering overrides like `MOLTIS_CONFIG_DIR`.
pub fn user_global_config_dir() -> Option<PathBuf> {
    home_dir().map(|h| h.join(".config").join("moltis"))
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

/// Path to workspace tool-guidance markdown.
pub fn tools_path() -> PathBuf {
    data_dir().join("TOOLS.md")
}

/// Path to workspace heartbeat markdown.
pub fn heartbeat_path() -> PathBuf {
    data_dir().join("HEARTBEAT.md")
}

/// Load identity values from `IDENTITY.md` frontmatter if present.
pub fn load_identity() -> Option<AgentIdentity> {
    let path = identity_path();
    let content = std::fs::read_to_string(path).ok()?;
    let frontmatter = extract_yaml_frontmatter(&content)?;
    let identity = parse_identity_frontmatter(frontmatter);
    if identity.name.is_none()
        && identity.emoji.is_none()
        && identity.creature.is_none()
        && identity.vibe.is_none()
    {
        None
    } else {
        Some(identity)
    }
}

/// Load user values from `USER.md` frontmatter if present.
pub fn load_user() -> Option<UserProfile> {
    let path = user_path();
    let content = std::fs::read_to_string(path).ok()?;
    let frontmatter = extract_yaml_frontmatter(&content)?;
    let user = parse_user_frontmatter(frontmatter);
    if user.name.is_none() && user.timezone.is_none() {
        None
    } else {
        Some(user)
    }
}

/// Load SOUL.md from the workspace root (`data_dir`) if present and non-empty.
pub fn load_soul() -> Option<String> {
    let path = soul_path();
    let content = std::fs::read_to_string(path).ok()?;
    let trimmed = content.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// Load AGENTS.md from the workspace root (`data_dir`) if present and non-empty.
pub fn load_agents_md() -> Option<String> {
    load_workspace_markdown(agents_path())
}

/// Load TOOLS.md from the workspace root (`data_dir`) if present and non-empty.
pub fn load_tools_md() -> Option<String> {
    load_workspace_markdown(tools_path())
}

/// Load HEARTBEAT.md from the workspace root (`data_dir`) if present and non-empty.
pub fn load_heartbeat_md() -> Option<String> {
    load_workspace_markdown(heartbeat_path())
}

/// Persist SOUL.md in the workspace root (`data_dir`).
///
/// - `Some(non-empty)` writes `SOUL.md`
/// - `None` or empty removes `SOUL.md` when it exists
pub fn save_soul(soul: Option<&str>) -> anyhow::Result<PathBuf> {
    let path = soul_path();
    match soul.map(str::trim) {
        Some(content) if !content.is_empty() => {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&path, content)?;
        },
        _ => {
            if path.exists() {
                std::fs::remove_file(&path)?;
            }
        },
    }
    Ok(path)
}

/// Persist identity values to `IDENTITY.md` using YAML frontmatter.
pub fn save_identity(identity: &AgentIdentity) -> anyhow::Result<PathBuf> {
    let path = identity_path();
    let has_values = identity.name.is_some()
        || identity.emoji.is_some()
        || identity.creature.is_some()
        || identity.vibe.is_some();

    if !has_values {
        if path.exists() {
            std::fs::remove_file(&path)?;
        }
        return Ok(path);
    }

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let mut yaml_lines = Vec::new();
    if let Some(name) = identity.name.as_deref() {
        yaml_lines.push(format!("name: {}", yaml_scalar(name)));
    }
    if let Some(emoji) = identity.emoji.as_deref() {
        yaml_lines.push(format!("emoji: {}", yaml_scalar(emoji)));
    }
    if let Some(creature) = identity.creature.as_deref() {
        yaml_lines.push(format!("creature: {}", yaml_scalar(creature)));
    }
    if let Some(vibe) = identity.vibe.as_deref() {
        yaml_lines.push(format!("vibe: {}", yaml_scalar(vibe)));
    }
    let yaml = yaml_lines.join("\n");
    let content = format!(
        "---\n{}\n---\n\n# IDENTITY.md\n\nThis file is managed by Moltis settings.\n",
        yaml
    );
    std::fs::write(&path, content)?;
    Ok(path)
}

/// Persist user values to `USER.md` using YAML frontmatter.
pub fn save_user(user: &UserProfile) -> anyhow::Result<PathBuf> {
    let path = user_path();
    let has_values = user.name.is_some() || user.timezone.is_some();

    if !has_values {
        if path.exists() {
            std::fs::remove_file(&path)?;
        }
        return Ok(path);
    }

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let mut yaml_lines = Vec::new();
    if let Some(name) = user.name.as_deref() {
        yaml_lines.push(format!("name: {}", yaml_scalar(name)));
    }
    if let Some(timezone) = user.timezone.as_deref() {
        yaml_lines.push(format!("timezone: {}", yaml_scalar(timezone)));
    }
    let yaml = yaml_lines.join("\n");
    let content = format!(
        "---\n{}\n---\n\n# USER.md\n\nThis file is managed by Moltis settings.\n",
        yaml
    );
    std::fs::write(&path, content)?;
    Ok(path)
}

fn extract_yaml_frontmatter(content: &str) -> Option<&str> {
    let trimmed = content.trim_start();
    if !trimmed.starts_with("---") {
        return None;
    }
    let rest = trimmed.strip_prefix("---")?;
    let rest = rest.strip_prefix('\n')?;
    let end = rest.find("\n---")?;
    Some(&rest[..end])
}

fn parse_identity_frontmatter(frontmatter: &str) -> AgentIdentity {
    let mut identity = AgentIdentity::default();
    for raw in frontmatter.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value_raw)) = line.split_once(':') else {
            continue;
        };
        let key = key.trim();
        let value = unquote_yaml_scalar(value_raw.trim());
        if value.is_empty() {
            continue;
        }
        match key {
            "name" => identity.name = Some(value.to_string()),
            "emoji" => identity.emoji = Some(value.to_string()),
            "creature" => identity.creature = Some(value.to_string()),
            "vibe" => identity.vibe = Some(value.to_string()),
            _ => {},
        }
    }
    identity
}

fn parse_user_frontmatter(frontmatter: &str) -> UserProfile {
    let mut user = UserProfile::default();
    for raw in frontmatter.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value_raw)) = line.split_once(':') else {
            continue;
        };
        let key = key.trim();
        let value = unquote_yaml_scalar(value_raw.trim());
        if value.is_empty() {
            continue;
        }
        match key {
            "name" => user.name = Some(value.to_string()),
            "timezone" => user.timezone = Some(value.to_string()),
            _ => {},
        }
    }
    user
}

fn unquote_yaml_scalar(value: &str) -> &str {
    if value.len() >= 2
        && ((value.starts_with('"') && value.ends_with('"'))
            || (value.starts_with('\'') && value.ends_with('\'')))
    {
        &value[1..value.len() - 1]
    } else {
        value
    }
}

fn yaml_scalar(value: &str) -> String {
    if value.contains(':')
        || value.contains('#')
        || value.starts_with(' ')
        || value.ends_with(' ')
        || value.contains('\n')
    {
        format!("'{}'", value.replace('\'', "''"))
    } else {
        value.to_string()
    }
}

fn load_workspace_markdown(path: PathBuf) -> Option<String> {
    let content = std::fs::read_to_string(path).ok()?;
    let trimmed = strip_leading_html_comments(&content).trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn strip_leading_html_comments(content: &str) -> &str {
    let mut rest = content;
    loop {
        let trimmed = rest.trim_start();
        if !trimmed.starts_with("<!--") {
            return trimmed;
        }
        let Some(end) = trimmed.find("-->") else {
            return "";
        };
        rest = &trimmed[end + 3..];
    }
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
struct ConfigSaveState {
    target_path: Option<PathBuf>,
}

/// Lock guarding config read-modify-write cycles and the target config path
/// being synchronized.
static CONFIG_SAVE_LOCK: Mutex<ConfigSaveState> = Mutex::new(ConfigSaveState { target_path: None });

/// Atomically load the current config, apply `f`, and save.
///
/// Acquires a process-wide lock so concurrent callers cannot race.
/// Returns the path written to.
pub fn update_config(f: impl FnOnce(&mut MoltisConfig)) -> anyhow::Result<PathBuf> {
    let mut guard = CONFIG_SAVE_LOCK.lock().unwrap();
    let target_path = find_or_default_config_path();
    guard.target_path = Some(target_path.clone());
    let mut config = discover_and_load();
    f(&mut config);
    save_config_to_path(&target_path, &config)
}

/// Serialize `config` to TOML and write it to the user-global config path.
///
/// Creates parent directories if needed. Returns the path written to.
///
/// Prefer [`update_config`] for read-modify-write cycles to avoid races.
pub fn save_config(config: &MoltisConfig) -> anyhow::Result<PathBuf> {
    let mut guard = CONFIG_SAVE_LOCK.lock().unwrap();
    let target_path = find_or_default_config_path();
    guard.target_path = Some(target_path.clone());
    save_config_to_path(&target_path, config)
}

fn save_config_to_path(path: &Path, config: &MoltisConfig) -> anyhow::Result<PathBuf> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let toml_str =
        toml::to_string_pretty(config).map_err(|e| anyhow::anyhow!("serialize config: {e}"))?;
    std::fs::write(path, toml_str)?;
    debug!(path = %path.display(), "saved config");
    Ok(path.to_path_buf())
}

/// Write the default config file to the user-global config path.
/// Only called when no config file exists yet.
/// Uses a comprehensive template with all options documented.
fn write_default_config(config: &MoltisConfig) -> anyhow::Result<()> {
    let path = find_or_default_config_path();
    if path.exists() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // Use the documented template instead of plain serialization
    let toml_str = crate::template::default_config_template(config.server.port);
    std::fs::write(&path, &toml_str)?;
    debug!(path = %path.display(), "wrote default config file with template");
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
    let trimmed = val.trim();

    // Support JSON arrays/objects for list-like env overrides, e.g.
    // MOLTIS_PROVIDERS__OFFERED='["openai","github-copilot"]' or '[]'.
    if ((trimmed.starts_with('[') && trimmed.ends_with(']'))
        || (trimmed.starts_with('{') && trimmed.ends_with('}')))
        && let Ok(parsed) = serde_json::from_str::<serde_json::Value>(trimmed)
    {
        return parsed;
    }

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

    struct TestDataDirState {
        _data_dir: Option<PathBuf>,
    }

    static DATA_DIR_TEST_LOCK: std::sync::Mutex<TestDataDirState> =
        std::sync::Mutex::new(TestDataDirState { _data_dir: None });

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
    fn parse_env_value_json_array() {
        assert_eq!(
            parse_env_value("[\"openai\",\"github-copilot\"]"),
            serde_json::json!(["openai", "github-copilot"])
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
    fn apply_env_overrides_providers_offered_array() {
        let vars = vec![(
            "MOLTIS_PROVIDERS__OFFERED".into(),
            "[\"openai\",\"github-copilot\"]".into(),
        )];
        let config = apply_env_overrides_with(MoltisConfig::default(), vars.into_iter());
        assert_eq!(config.providers.offered, vec!["openai", "github-copilot"]);
    }

    #[test]
    fn apply_env_overrides_providers_offered_empty_array() {
        let vars = vec![("MOLTIS_PROVIDERS__OFFERED".into(), "[]".into())];
        let mut base = MoltisConfig::default();
        base.providers.offered = vec!["openai".into()];
        let config = apply_env_overrides_with(base, vars.into_iter());
        assert!(
            config.providers.offered.is_empty(),
            "empty JSON array env override should clear providers.offered"
        );
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
        let _guard = DATA_DIR_TEST_LOCK.lock().unwrap();
        let path = PathBuf::from("/tmp/test-data-dir-override");
        set_data_dir(path.clone());
        assert_eq!(data_dir(), path);
        clear_data_dir();
    }

    #[test]
    fn save_and_load_identity_frontmatter() {
        let _guard = DATA_DIR_TEST_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().expect("tempdir");
        set_data_dir(dir.path().to_path_buf());

        let identity = AgentIdentity {
            name: Some("Rex".to_string()),
            emoji: Some("🐶".to_string()),
            creature: Some("dog".to_string()),
            vibe: Some("chill".to_string()),
        };

        let path = save_identity(&identity).expect("save identity");
        assert!(path.exists());
        let raw = std::fs::read_to_string(&path).expect("read identity file");

        let loaded = load_identity().expect("load identity");
        assert_eq!(loaded.name.as_deref(), Some("Rex"));
        assert_eq!(loaded.emoji.as_deref(), Some("🐶"), "raw file:\n{raw}");
        assert_eq!(loaded.creature.as_deref(), Some("dog"));
        assert_eq!(loaded.vibe.as_deref(), Some("chill"));

        clear_data_dir();
    }

    #[test]
    fn save_identity_removes_empty_file() {
        let _guard = DATA_DIR_TEST_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().expect("tempdir");
        set_data_dir(dir.path().to_path_buf());

        let seeded = AgentIdentity {
            name: Some("Rex".to_string()),
            emoji: None,
            creature: None,
            vibe: None,
        };
        let path = save_identity(&seeded).expect("seed identity");
        assert!(path.exists());

        save_identity(&AgentIdentity::default()).expect("save empty identity");
        assert!(!path.exists());

        clear_data_dir();
    }

    #[test]
    fn save_and_load_user_frontmatter() {
        let _guard = DATA_DIR_TEST_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().expect("tempdir");
        set_data_dir(dir.path().to_path_buf());

        let user = UserProfile {
            name: Some("Alice".to_string()),
            timezone: Some("Europe/Berlin".to_string()),
        };

        let path = save_user(&user).expect("save user");
        assert!(path.exists());

        let loaded = load_user().expect("load user");
        assert_eq!(loaded.name.as_deref(), Some("Alice"));
        assert_eq!(loaded.timezone.as_deref(), Some("Europe/Berlin"));

        clear_data_dir();
    }

    #[test]
    fn save_user_removes_empty_file() {
        let _guard = DATA_DIR_TEST_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().expect("tempdir");
        set_data_dir(dir.path().to_path_buf());

        let seeded = UserProfile {
            name: Some("Alice".to_string()),
            timezone: None,
        };
        let path = save_user(&seeded).expect("seed user");
        assert!(path.exists());

        save_user(&UserProfile::default()).expect("save empty user");
        assert!(!path.exists());

        clear_data_dir();
    }

    #[test]
    fn load_tools_md_reads_trimmed_content() {
        let _guard = DATA_DIR_TEST_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().expect("tempdir");
        set_data_dir(dir.path().to_path_buf());

        std::fs::write(dir.path().join("TOOLS.md"), "\n  Use safe tools first.  \n").unwrap();
        assert_eq!(load_tools_md().as_deref(), Some("Use safe tools first."));

        clear_data_dir();
    }

    #[test]
    fn load_agents_md_reads_trimmed_content() {
        let _guard = DATA_DIR_TEST_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().expect("tempdir");
        set_data_dir(dir.path().to_path_buf());

        std::fs::write(
            dir.path().join("AGENTS.md"),
            "\nLocal workspace instructions\n",
        )
        .unwrap();
        assert_eq!(
            load_agents_md().as_deref(),
            Some("Local workspace instructions")
        );

        clear_data_dir();
    }

    #[test]
    fn load_heartbeat_md_reads_trimmed_content() {
        let _guard = DATA_DIR_TEST_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().expect("tempdir");
        set_data_dir(dir.path().to_path_buf());

        std::fs::write(dir.path().join("HEARTBEAT.md"), "\n# Heartbeat\n- ping\n").unwrap();
        assert_eq!(load_heartbeat_md().as_deref(), Some("# Heartbeat\n- ping"));

        clear_data_dir();
    }

    #[test]
    fn workspace_markdown_ignores_leading_html_comments() {
        let _guard = DATA_DIR_TEST_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().expect("tempdir");
        set_data_dir(dir.path().to_path_buf());

        std::fs::write(
            dir.path().join("TOOLS.md"),
            "<!-- comment -->\n\nUse read-only tools first.",
        )
        .unwrap();
        assert_eq!(
            load_tools_md().as_deref(),
            Some("Use read-only tools first.")
        );

        clear_data_dir();
    }

    #[test]
    fn workspace_markdown_comment_only_is_treated_as_empty() {
        let _guard = DATA_DIR_TEST_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().expect("tempdir");
        set_data_dir(dir.path().to_path_buf());

        std::fs::write(dir.path().join("HEARTBEAT.md"), "<!-- guidance -->").unwrap();
        assert_eq!(load_heartbeat_md(), None);

        clear_data_dir();
    }
}
