use {
    super::*,
    crate::{env_subst::substitute_env, schema::MoltisConfig},
    std::{
        path::{Path, PathBuf},
        sync::Mutex,
    },
    tracing::{debug, info, warn},
};

/// Load config from the given path (any supported format).
///
/// After parsing, `MOLTIS_*` env vars are applied as overrides.
///
/// Uses a two-pass approach so that `[env]` section values are available
/// for `${VAR}` substitution in other sections of the same config file.
pub fn load_config(path: &Path) -> crate::Result<MoltisConfig> {
    let raw = std::fs::read_to_string(path).map_err(|source| {
        crate::Error::external(format!("failed to read {}", path.display()), source)
    })?;

    // First pass: resolve process env vars, parse to extract [env] section.
    let first_pass = substitute_env(&raw);
    let preliminary: MoltisConfig = parse_config(&first_pass, path)?;

    // Second pass: re-substitute using both process env and [env] values.
    // This allows ${VAR} in other sections to reference [env]-defined vars.
    let config = if preliminary.env.is_empty() {
        preliminary
    } else {
        let second_pass = crate::env_subst::substitute_env_with_overrides(&raw, &preliminary.env);
        parse_config(&second_pass, path)?
    };

    Ok(apply_env_overrides(config))
}

/// Load and parse the config file with env substitution and includes.
pub fn load_config_value(path: &Path) -> crate::Result<serde_json::Value> {
    let raw = std::fs::read_to_string(path).map_err(|source| {
        crate::Error::external(format!("failed to read {}", path.display()), source)
    })?;
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
    let mut cfg = if let Some(path) = find_config_file() {
        debug!(path = %path.display(), "loading config");
        match load_config(&path) {
            Ok(mut cfg) => {
                // If port is 0 (default/missing), generate a random port and save it.
                // Use `save_config_to_path` directly instead of `save_config` because
                // this function may be called from within `update_config`, which already
                // holds `CONFIG_SAVE_LOCK`. Re-acquiring a `std::sync::Mutex` on the
                // same thread would deadlock.
                if cfg.server.port == 0 {
                    cfg.server.port = generate_random_port();
                    debug!(
                        port = cfg.server.port,
                        "generated random port for existing config"
                    );
                    if let Err(e) = save_config_to_path(&path, &cfg) {
                        warn!(error = %e, "failed to save config with generated port");
                    }
                }
                cfg // env overrides already applied by load_config
            },
            Err(e) => {
                warn!(path = %path.display(), error = %e, "failed to load config, using defaults");
                apply_env_overrides(MoltisConfig::default())
            },
        }
    } else {
        let default_path = find_or_default_config_path();
        debug!(
            path = %default_path.display(),
            "no config file found, writing default config with random port"
        );
        let mut config = MoltisConfig::default();
        // Generate a unique port for this installation
        config.server.port = generate_random_port();
        if let Err(e) = write_default_config(&default_path, &config) {
            warn!(
                path = %default_path.display(),
                error = %e,
                "failed to write default config file, continuing with in-memory defaults"
            );
        } else {
            info!(
                path = %default_path.display(),
                "wrote default config template"
            );
        }
        apply_env_overrides(config)
    };

    // Merge markdown agent definitions (TOML presets take precedence).
    let agent_defs = crate::agent_defs::discover_agent_defs();
    if !agent_defs.is_empty() {
        debug!(
            count = agent_defs.len(),
            "discovered markdown agent definitions"
        );
        crate::agent_defs::merge_agent_defs(&mut cfg.agents.presets, agent_defs);
    }

    cfg
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
pub fn update_config(f: impl FnOnce(&mut MoltisConfig)) -> crate::Result<PathBuf> {
    let mut guard = CONFIG_SAVE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
pub fn save_config(config: &MoltisConfig) -> crate::Result<PathBuf> {
    let mut guard = CONFIG_SAVE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let target_path = find_or_default_config_path();
    guard.target_path = Some(target_path.clone());
    save_config_to_path(&target_path, config)
}

/// Write raw TOML to the config file, preserving comments.
///
/// Validates the input by parsing it first. Acquires the config save lock
/// so concurrent callers cannot race.  Returns the path written to.
pub fn save_raw_config(toml_str: &str) -> crate::Result<PathBuf> {
    let _: MoltisConfig = toml::from_str(toml_str)
        .map_err(|source| crate::Error::external(format!("invalid config: {source}"), source))?;
    let mut guard = CONFIG_SAVE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let path = find_or_default_config_path();
    guard.target_path = Some(path.clone());
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, toml_str)?;
    debug!(path = %path.display(), "saved raw config");
    Ok(path)
}

/// Serialize `config` to TOML and write it to the provided path.
///
/// For existing TOML files, this preserves user comments by merging the new
/// serialized values into the current document structure before writing.
pub fn save_config_to_path(path: &Path, config: &MoltisConfig) -> crate::Result<PathBuf> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let toml_str = toml::to_string_pretty(config)
        .map_err(|source| crate::Error::external("serialize config", source))?;

    let is_toml_path = path
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("toml"));

    if is_toml_path && path.exists() {
        if let Err(error) = merge_toml_preserving_comments(path, &toml_str) {
            warn!(
                path = %path.display(),
                error = %error,
                "failed to preserve TOML comments, rewriting config without comments"
            );
            std::fs::write(path, toml_str)?;
        }
    } else {
        std::fs::write(path, toml_str)?;
    }

    debug!(path = %path.display(), "saved config");
    Ok(path.to_path_buf())
}

fn merge_toml_preserving_comments(path: &Path, updated_toml: &str) -> crate::Result<()> {
    let current_toml = std::fs::read_to_string(path)?;
    let mut current_doc = current_toml
        .parse::<toml_edit::DocumentMut>()
        .map_err(|source| crate::Error::external("parse existing TOML", source))?;
    let updated_doc = updated_toml
        .parse::<toml_edit::DocumentMut>()
        .map_err(|source| crate::Error::external("parse updated TOML", source))?;

    merge_toml_tables(current_doc.as_table_mut(), updated_doc.as_table());
    std::fs::write(path, current_doc.to_string())?;
    Ok(())
}

pub(super) fn merge_toml_tables(current: &mut toml_edit::Table, updated: &toml_edit::Table) {
    let current_keys: Vec<String> = current.iter().map(|(key, _)| key.to_string()).collect();
    for key in current_keys {
        if !updated.contains_key(&key) {
            let _ = current.remove(&key);
        }
    }

    for (key, updated_item) in updated.iter() {
        if let Some(current_item) = current.get_mut(key) {
            merge_toml_items(current_item, updated_item);
        } else {
            // Clone the item and strip `doc_position` metadata inherited from
            // the source document.  Without this, toml_edit uses the position
            // from the *serialized* document, causing new sub-tables to be
            // interleaved among existing sections instead of appearing after
            // their parent (GH-684).
            current.insert(key, clone_item_without_positions(updated_item));
        }
    }
}

/// Deep-clone a `toml_edit::Item`, stripping `doc_position` from every table
/// so that newly inserted entries get auto-positioned by `toml_edit` rather
/// than inheriting stale positions from a different document.
fn clone_item_without_positions(item: &toml_edit::Item) -> toml_edit::Item {
    match item {
        toml_edit::Item::Table(t) => toml_edit::Item::Table(clone_table_without_positions(t)),
        toml_edit::Item::ArrayOfTables(arr) => {
            let mut new_arr = toml_edit::ArrayOfTables::new();
            for table in arr.iter() {
                new_arr.push(clone_table_without_positions(table));
            }
            toml_edit::Item::ArrayOfTables(new_arr)
        },
        other => other.clone(),
    }
}

/// Clone a table, recursively stripping `doc_position` so new tables get
/// auto-positioned when inserted into a different document.
fn clone_table_without_positions(src: &toml_edit::Table) -> toml_edit::Table {
    let mut dst = toml_edit::Table::new();
    // doc_position is None for manually created tables → auto-positioned
    dst.set_implicit(src.is_implicit());
    dst.set_dotted(src.is_dotted());
    *dst.decor_mut() = src.decor().clone();
    for (key, item) in src.iter() {
        dst.insert(key, clone_item_without_positions(item));
        // Preserve key decorations (whitespace/comments around the key)
        if let (Some(src_key), Some(mut dst_key)) = (src.key(key), dst.key_mut(key)) {
            *dst_key.leaf_decor_mut() = src_key.leaf_decor().clone();
            *dst_key.dotted_decor_mut() = src_key.dotted_decor().clone();
        }
    }
    dst
}

fn merge_toml_items(current: &mut toml_edit::Item, updated: &toml_edit::Item) {
    match (current, updated) {
        (toml_edit::Item::Table(current_table), toml_edit::Item::Table(updated_table)) => {
            merge_toml_tables(current_table, updated_table);
        },
        (toml_edit::Item::Value(current_value), toml_edit::Item::Value(updated_value)) => {
            let existing_decor = current_value.decor().clone();
            *current_value = updated_value.clone();
            *current_value.decor_mut() = existing_decor;
        },
        (current_item, updated_item) => {
            *current_item = updated_item.clone();
        },
    }
}

/// Write the default config file to the user-global config path.
/// Only called when no config file exists yet.
/// Uses a comprehensive template with all options documented.
pub(super) fn write_default_config(path: &Path, config: &MoltisConfig) -> crate::Result<()> {
    if path.exists() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // Use the documented template instead of plain serialization
    let toml_str = crate::template::default_config_template(config.server.port);
    std::fs::write(path, &toml_str)?;
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
/// considered. `MOLTIS_CONFIG_DIR`, `MOLTIS_DATA_DIR`, `MOLTIS_SHARE_DIR`,
/// `MOLTIS_ASSETS_DIR`, `MOLTIS_TOKEN`, `MOLTIS_PASSWORD`, `MOLTIS_TAILSCALE`,
/// `MOLTIS_WEBAUTHN_RP_ID`, and `MOLTIS_WEBAUTHN_ORIGIN` are excluded
/// (they are handled separately).
pub fn apply_env_overrides(config: MoltisConfig) -> MoltisConfig {
    apply_env_overrides_with(config, std::env::vars())
}

/// Apply env overrides from an arbitrary iterator of (key, value) pairs.
/// Exposed for testing without mutating the process environment.
pub(super) fn apply_env_overrides_with(
    config: MoltisConfig,
    vars: impl Iterator<Item = (String, String)>,
) -> MoltisConfig {
    use serde_json::Value;

    const EXCLUDED: &[&str] = &[
        "MOLTIS_CONFIG_DIR",
        "MOLTIS_DATA_DIR",
        "MOLTIS_SHARE_DIR",
        "MOLTIS_ASSETS_DIR",
        "MOLTIS_TOKEN",
        "MOLTIS_PASSWORD",
        "MOLTIS_TAILSCALE",
        "MOLTIS_WEBAUTHN_RP_ID",
        "MOLTIS_WEBAUTHN_ORIGIN",
        "MOLTIS_EXTERNAL_URL",
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

/// Re-resolve `${VAR}` placeholders in a loaded config using additional overrides.
///
/// Call this after runtime env vars (e.g. DB-stored UI variables) become
/// available.  Substitution happens at the JSON value level (not textual
/// TOML), so override values that contain quotes or backslashes are safe.
///
/// Lookup precedence: process env → `overrides` map.
pub fn resubstitute_config(
    config: &MoltisConfig,
    overrides: &std::collections::HashMap<String, String>,
) -> crate::Result<MoltisConfig> {
    let mut json = serde_json::to_value(config)
        .map_err(|source| crate::Error::external("serialize config for resubstitution", source))?;
    resolve_placeholders_in_value(&mut json, overrides);
    let reloaded: MoltisConfig = serde_json::from_value(json).map_err(|source| {
        crate::Error::external("deserialize config after resubstitution", source)
    })?;
    Ok(apply_env_overrides(reloaded))
}

/// Recursively walk a JSON value tree and resolve `${VAR}` placeholders in
/// string values using process env + the overrides map.
fn resolve_placeholders_in_value(
    value: &mut serde_json::Value,
    overrides: &std::collections::HashMap<String, String>,
) {
    match value {
        serde_json::Value::String(s) if s.contains("${") => {
            *s = crate::env_subst::substitute_env_with_overrides(s, overrides);
        },
        serde_json::Value::Object(map) => {
            for v in map.values_mut() {
                resolve_placeholders_in_value(v, overrides);
            }
        },
        serde_json::Value::Array(arr) => {
            for v in arr.iter_mut() {
                resolve_placeholders_in_value(v, overrides);
            }
        },
        _ => {},
    }
}

/// Parse a string env value into a JSON value, trying bool and number first.
pub(super) fn parse_env_value(val: &str) -> serde_json::Value {
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
pub(super) fn set_nested(root: &mut serde_json::Value, path: &[String], val: serde_json::Value) {
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
        let Some(next) = current.get_mut(key) else {
            return;
        };
        current = next;
    }
}

pub(super) fn parse_config(raw: &str, path: &Path) -> crate::Result<MoltisConfig> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("toml");

    match ext {
        "toml" => Ok(toml::from_str(raw)?),
        "yaml" | "yml" => Ok(serde_yaml::from_str(raw)?),
        "json" => Ok(serde_json::from_str(raw)?),
        _ => Err(crate::Error::message(format!(
            "unsupported config format: .{ext}"
        ))),
    }
}

fn parse_config_value(raw: &str, path: &Path) -> crate::Result<serde_json::Value> {
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
        _ => Err(crate::Error::message(format!(
            "unsupported config format: .{ext}"
        ))),
    }
}
