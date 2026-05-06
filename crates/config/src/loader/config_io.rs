use {
    super::*,
    crate::{env_subst::substitute_env, schema::MoltisConfig},
    std::{
        path::{Path, PathBuf},
        sync::Mutex,
    },
    tracing::{debug, info, warn},
};

/// Write content to a file atomically via write-to-temp + rename.
///
/// This prevents corruption when two processes (e.g. CLI + server) write
/// the config concurrently — `rename` is atomic on POSIX filesystems so
/// readers always see either the old or new content, never a partial mix.
fn atomic_write(path: &Path, content: impl AsRef<[u8]>) -> std::io::Result<()> {
    use std::io::Write;
    let dir = path.parent().unwrap_or(Path::new("."));
    let mut tmp = tempfile::NamedTempFile::new_in(dir)?;
    tmp.write_all(content.as_ref())?;
    tmp.persist(path).map_err(|e| e.error)?;
    Ok(())
}

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

/// Discover and load config from standard locations using layered merge.
///
/// Merge order:
/// 1. Built-in Rust defaults (`MoltisConfig::default()`)
/// 2. Moltis-managed `defaults.toml` (refreshed on every startup)
/// 3. User override file `moltis.{toml,yaml,yml,json}`
/// 4. `MOLTIS_*` environment variable overrides
///
/// One-time config initialization — call once at process startup.
///
/// Performs all write side-effects that prepare the config directory:
/// - Refreshes Moltis-managed `defaults.toml`
/// - Auto-compacts user config (strips materialized defaults)
/// - Writes a default config template on first run
/// - Persists a randomly generated port so it stays stable
///
/// After this, use [`discover_and_load`] (read-only) to load config.
pub fn initialize_config() {
    // Refresh Moltis-managed defaults.toml.
    if let Some(dir) = config_dir()
        && let Err(e) = crate::defaults::write_defaults_toml(&dir)
    {
        warn!(error = %e, "failed to write defaults.toml");
    }

    // Auto-compact: strip default values that were materialized by older
    // versions. Idempotent — no-op when already compact.
    if find_config_file().is_some() {
        match compact_config() {
            Ok((before, after)) if before > after => {
                info!(
                    before,
                    after,
                    removed = before - after,
                    "auto-compacted user config (stripped default values)"
                );
            },
            Err(e) => {
                debug!(error = %e, "auto-compact skipped");
            },
            _ => {},
        }
    }

    // Write default user config on first run (when no config file exists).
    if find_config_file().is_none() {
        let default_path = find_or_default_config_path();
        debug!(
            path = %default_path.display(),
            "no config file found, writing default config with random port"
        );
        let mut config = MoltisConfig::default();
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
    }

    let cfg = discover_and_load_readonly();

    // Persist randomly generated port so it stays stable across restarts.
    // discover_and_load_readonly generates an in-memory port when the on-disk
    // value is 0 — write it back so the port is stable across restarts.
    if let Some(path) = find_config_file()
        && cfg.server.port != 0
        && let Ok(raw) = std::fs::read_to_string(&path)
        && let Ok(on_disk) = parse_config(&raw, &path)
        && on_disk.server.port == 0
    {
        debug!(
            port = cfg.server.port,
            "persisting generated port to config"
        );
        if let Err(e) = save_user_config_to_path(&path, &cfg) {
            warn!(error = %e, "failed to save config with generated port");
        }
    }
}

/// Discover and load config from disk (read-only, no side-effects).
///
/// This is the primary config loading function. Call [`initialize_config`]
/// once at process startup to prepare the config directory, then use this
/// function everywhere else.
///
/// User config search order:
/// 1. `./moltis.{toml,yaml,yml,json}` (project-local)
/// 2. `~/.config/moltis/moltis.{toml,yaml,yml,json}` (user-global)
///
/// Returns `MoltisConfig::default()` if no config file is found.
pub fn discover_and_load() -> MoltisConfig {
    discover_and_load_readonly()
}

/// Load config using layered merge without writing any files.
///
/// Identical to [`discover_and_load`]. Retained for backward compatibility.
pub fn discover_and_load_readonly() -> MoltisConfig {
    let mut cfg = if let Some(path) = find_config_file() {
        debug!(path = %path.display(), "loading config (read-only)");
        match load_layered_config(&path) {
            Ok(mut cfg) => {
                if cfg.server.port == 0 {
                    cfg.server.port = generate_random_port();
                }
                cfg
            },
            Err(e) => {
                warn!(path = %path.display(), error = %e, "failed to load config, using defaults");
                apply_env_overrides(MoltisConfig::default())
            },
        }
    } else {
        apply_env_overrides(MoltisConfig::default())
    };

    // Merge markdown agent definitions (TOML presets take precedence).
    let agent_defs = crate::agent_defs::discover_agent_defs();
    if !agent_defs.is_empty() {
        crate::agent_defs::merge_agent_defs(&mut cfg.agents.presets, agent_defs);
    }

    cfg
}

/// Load config with layered merge: defaults.toml + user file + env overrides.
///
/// For TOML user files, performs a deep merge at the TOML document level so
/// that user overrides are additive (only keys present in the user file
/// override the corresponding defaults).  For YAML/JSON user files, falls
/// back to a struct-level load (since `defaults.toml` is always TOML).
fn load_layered_config(user_path: &Path) -> crate::Result<MoltisConfig> {
    let is_toml = user_path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("toml"));

    if is_toml {
        load_layered_config_toml(user_path)
    } else {
        // YAML/JSON: simple struct-level load (no TOML-level merge).
        load_config(user_path)
    }
}

/// TOML-specific layered loading with deep document merge.
fn load_layered_config_toml(user_path: &Path) -> crate::Result<MoltisConfig> {
    let user_raw = std::fs::read_to_string(user_path).map_err(|source| {
        crate::Error::external(format!("failed to read {}", user_path.display()), source)
    })?;

    // Two-pass env substitution on the user file (same as load_config).
    let user_first_pass = substitute_env(&user_raw);
    let preliminary: MoltisConfig = parse_config(&user_first_pass, user_path)?;

    let user_substituted = if preliminary.env.is_empty() {
        user_first_pass
    } else {
        crate::env_subst::substitute_env_with_overrides(&user_raw, &preliminary.env)
    };

    // Load defaults TOML (generate fresh if missing).
    let defaults_toml = crate::defaults::generate_defaults_toml().unwrap_or_else(|_| String::new());

    // Deep merge: defaults ← user overrides.
    let config = if defaults_toml.is_empty() {
        parse_config(&user_substituted, user_path)?
    } else {
        crate::defaults::merge_defaults_with_user_toml(
            &defaults_toml,
            &user_substituted,
            user_path,
        )?
    };

    Ok(apply_env_overrides(config))
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

/// Atomically load the current config, apply `f`, and save only the user
/// override file.
///
/// The closure receives the **effective** (merged) config for reading, but
/// only the fields that differ from defaults are written back to the user
/// file.  This prevents built-in defaults from being materialized into the
/// user config.
///
/// Acquires a process-wide lock so concurrent callers cannot race.
/// Returns the path written to.
pub fn update_config(f: impl FnOnce(&mut MoltisConfig)) -> crate::Result<PathBuf> {
    let mut guard = CONFIG_SAVE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let target_path = find_or_default_config_path();
    guard.target_path = Some(target_path.clone());
    let mut config = discover_and_load_readonly();
    f(&mut config);
    save_user_config_to_path(&target_path, &config)
}

/// Serialize `config` to TOML and write it to the user-global config path.
///
/// Only writes the user override layer (fields that differ from defaults
/// are preserved; built-in defaults are not materialized).
///
/// Creates parent directories if needed. Returns the path written to.
///
/// Prefer [`update_config`] for read-modify-write cycles to avoid races.
pub fn save_config(config: &MoltisConfig) -> crate::Result<PathBuf> {
    let mut guard = CONFIG_SAVE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let target_path = find_or_default_config_path();
    guard.target_path = Some(target_path.clone());
    save_user_config_to_path(&target_path, config)
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
    atomic_write(&path, toml_str)?;
    debug!(path = %path.display(), "saved raw config");
    Ok(path)
}

/// Serialize `config` to TOML and write it to the provided path.
///
/// For existing TOML files, this preserves user comments by merging the new
/// serialized values into the current document structure before writing.
pub fn save_config_to_path(path: &Path, config: &MoltisConfig) -> crate::Result<PathBuf> {
    let mut guard = CONFIG_SAVE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    guard.target_path = Some(path.to_path_buf());
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
            atomic_write(path, toml_str)?;
        }
    } else {
        atomic_write(path, toml_str)?;
    }

    debug!(path = %path.display(), "saved config");
    Ok(path.to_path_buf())
}

/// Save only the user-override layer to the given path.
///
/// For existing TOML files, preserves all keys already in the user file
/// (even if they match defaults — those are intentional freezes).  Only
/// *newly added* keys that match defaults are suppressed, preventing
/// built-in values from being materialized during unrelated config writes.
///
/// For new files, writes only non-default values.
pub fn save_user_config_to_path(path: &Path, config: &MoltisConfig) -> crate::Result<PathBuf> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let effective_toml = toml::to_string_pretty(config)
        .map_err(|source| crate::Error::external("serialize config", source))?;
    let defaults_toml = toml::to_string_pretty(&MoltisConfig::default())
        .map_err(|source| crate::Error::external("serialize defaults", source))?;

    let effective_doc = effective_toml
        .parse::<toml_edit::DocumentMut>()
        .map_err(|source| crate::Error::external("parse effective TOML", source))?;
    let defaults_doc = defaults_toml
        .parse::<toml_edit::DocumentMut>()
        .map_err(|source| crate::Error::external("parse defaults TOML", source))?;

    let is_toml_path = path
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("toml"));

    if !is_toml_path {
        // YAML/JSON configs: fall back to full struct serialization (no
        // TOML-level diffing). This preserves the pre-layered behavior.
        return save_config_to_path(path, config);
    }

    if path.exists() {
        // Existing TOML file: preserve keys the user already has.
        // Only strip defaults from keys that are NEW (not already on disk).
        let current_toml = std::fs::read_to_string(path)?;
        let current_doc = current_toml
            .parse::<toml_edit::DocumentMut>()
            .map_err(|source| crate::Error::external("parse existing user TOML", source))?;

        let mut override_doc = effective_doc;
        strip_new_default_values(
            override_doc.as_table_mut(),
            defaults_doc.as_table(),
            current_doc.as_table(),
        );

        let mut result_doc = current_doc;
        merge_toml_tables(result_doc.as_table_mut(), override_doc.as_table());
        atomic_write(path, result_doc.to_string())?;
    } else {
        // New TOML file: strip all default values.
        let mut override_doc = effective_doc;
        strip_default_values(override_doc.as_table_mut(), defaults_doc.as_table());
        atomic_write(path, override_doc.to_string())?;
    }

    debug!(path = %path.display(), "saved user config (override layer only)");
    Ok(path.to_path_buf())
}

/// Remove keys from `effective` that are identical to the corresponding key
/// in `defaults`.  After this call, `effective` contains only user overrides.
pub fn strip_default_values(effective: &mut toml_edit::Table, defaults: &toml_edit::Table) {
    let keys: Vec<String> = effective.iter().map(|(k, _)| k.to_string()).collect();
    for key in keys {
        let Some(eff_item) = effective.get(&key) else {
            continue;
        };
        let Some(def_item) = defaults.get(&key) else {
            // Key only in effective → user-added, keep it.
            continue;
        };

        match (eff_item, def_item) {
            (toml_edit::Item::Table(_), toml_edit::Item::Table(def_table)) => {
                // Recurse into sub-tables.
                if let Some(toml_edit::Item::Table(eff_mut)) = effective.get_mut(&key) {
                    strip_default_values(eff_mut, def_table);
                    // If the table is now empty, remove it.
                    if eff_mut.is_empty() {
                        effective.remove(&key);
                    }
                }
            },
            (toml_edit::Item::Value(eff_val), toml_edit::Item::Value(def_val))
                if values_equal(eff_val, def_val) =>
            {
                effective.remove(&key);
            },
            _ => {
                // Type mismatch (e.g. table vs value) → user override, keep it.
            },
        }
    }
}

/// Strip default values only for keys that are NEW — not already present
/// in the on-disk user file.  Keys the user already has are preserved even
/// if they match defaults (those are intentional freezes from a prior
/// version).  This prevents `update_config()` from silently trimming an
/// existing user config on upgrade.
fn strip_new_default_values(
    effective: &mut toml_edit::Table,
    defaults: &toml_edit::Table,
    on_disk: &toml_edit::Table,
) {
    let keys: Vec<String> = effective.iter().map(|(k, _)| k.to_string()).collect();
    for key in keys {
        let Some(eff_item) = effective.get(&key) else {
            continue;
        };
        let Some(def_item) = defaults.get(&key) else {
            // Not in defaults → user-added, always keep.
            continue;
        };

        // If this key already exists on disk, preserve it unconditionally.
        if let Some(disk_item) = on_disk.get(&key) {
            match (eff_item, def_item, disk_item) {
                (
                    toml_edit::Item::Table(_),
                    toml_edit::Item::Table(def_table),
                    toml_edit::Item::Table(disk_table),
                ) => {
                    // Recurse: check sub-keys individually.
                    if let Some(toml_edit::Item::Table(eff_mut)) = effective.get_mut(&key) {
                        strip_new_default_values(eff_mut, def_table, disk_table);
                    }
                },
                _ => {
                    // Key exists on disk → keep it (user put it there).
                },
            }
            continue;
        }

        // Key is NEW (not on disk). Strip it if it matches the default.
        match (eff_item, def_item) {
            (toml_edit::Item::Table(_), toml_edit::Item::Table(def_table)) => {
                let empty_disk = toml_edit::Table::new();
                if let Some(toml_edit::Item::Table(eff_mut)) = effective.get_mut(&key) {
                    strip_new_default_values(eff_mut, def_table, &empty_disk);
                    if eff_mut.is_empty() {
                        effective.remove(&key);
                    }
                }
            },
            (toml_edit::Item::Value(eff_val), toml_edit::Item::Value(def_val))
                if values_equal(eff_val, def_val) =>
            {
                effective.remove(&key);
            },
            _ => {},
        }
    }
}

/// Compare two `toml_edit::Value`s by their display representation.
fn values_equal(a: &toml_edit::Value, b: &toml_edit::Value) -> bool {
    // Compare using the display format, trimming whitespace decorations that
    // toml_edit preserves from source documents.
    a.to_string().trim() == b.to_string().trim()
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
    atomic_write(path, current_doc.to_string())?;
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

/// Strip all default values from the user config file, leaving only overrides.
///
/// Returns `(keys_before, keys_after)` so callers can report the reduction.
/// Does nothing if the config file doesn't exist or isn't TOML.
pub fn compact_config() -> crate::Result<(usize, usize)> {
    let mut guard = CONFIG_SAVE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let path = find_or_default_config_path();
    guard.target_path = Some(path.clone());
    if !path.exists() {
        return Ok((0, 0));
    }
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    if !ext.eq_ignore_ascii_case("toml") {
        return Ok((0, 0));
    }

    let user_toml = std::fs::read_to_string(&path)?;
    let mut user_doc = user_toml
        .parse::<toml_edit::DocumentMut>()
        .map_err(|source| crate::Error::external("parse user TOML", source))?;

    let defaults_toml = toml::to_string_pretty(&MoltisConfig::default())
        .map_err(|source| crate::Error::external("serialize defaults", source))?;
    let defaults_doc = defaults_toml
        .parse::<toml_edit::DocumentMut>()
        .map_err(|source| crate::Error::external("parse defaults TOML", source))?;

    let keys_before = count_leaf_keys(user_doc.as_table());
    strip_default_values(user_doc.as_table_mut(), defaults_doc.as_table());
    let keys_after = count_leaf_keys(user_doc.as_table());

    atomic_write(&path, user_doc.to_string())?;
    debug!(
        path = %path.display(),
        before = keys_before,
        after = keys_after,
        "compacted user config"
    );
    Ok((keys_before, keys_after))
}

/// Count leaf (non-table) keys recursively for reporting.
fn count_leaf_keys(table: &toml_edit::Table) -> usize {
    let mut count = 0;
    for (_, item) in table.iter() {
        match item {
            toml_edit::Item::Table(sub) => count += count_leaf_keys(sub),
            toml_edit::Item::Value(_) => count += 1,
            _ => {},
        }
    }
    count
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
    atomic_write(path, &toml_str)?;
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
