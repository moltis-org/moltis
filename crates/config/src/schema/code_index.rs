/// Code-index configuration for `moltis.toml`.
///
/// Controls how codebases are indexed for `codebase_search`, `codebase_peek`,
/// and `codebase_status` agent tools.
///
/// ```toml
/// [code_index]
/// enabled = true
/// # extensions = ["rs", "py", "ts"]  # empty = use built-in list of 50+
/// # max_file_size = "2MB"
/// # skip_paths = ["generated/"]
/// ```
use serde::{Deserialize, Serialize};

/// TOML-friendly code-index configuration.
///
/// Empty `extensions` / `skip_paths` means "use crate defaults".
/// This avoids duplicating the 50+ default extension list in the config schema.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CodeIndexTomlConfig {
    /// Whether code indexing is enabled globally. Default: `true`.
    pub enabled: bool,
    /// File extensions to index (without leading dot).
    /// Empty means "use the built-in default list" (50+ extensions).
    /// Non-empty replaces the default list entirely.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extensions: Vec<String>,
    /// Maximum file size for indexing. Human-friendly string: `"1MB"`, `"512KB"`, `"2GB"`.
    /// Default: `"1MB"`.
    #[serde(default = "default_max_file_size")]
    pub max_file_size: String,
    /// Whether to skip files that appear to be binary (contain null bytes). Default: `true`.
    pub skip_binary: bool,
    /// Additional path prefixes to skip (e.g. `"generated/"`, `"proto/"`).
    /// These are appended to the built-in skip list (`vendor/`, `node_modules/`, etc.).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skip_paths: Vec<String>,
    /// Override the data directory for index storage.
    /// When unset, defaults to `<data_dir>/code-index/`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_dir: Option<String>,
}

fn default_max_file_size() -> String {
    "1MB".to_string()
}

impl Default for CodeIndexTomlConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            extensions: Vec::new(),
            max_file_size: default_max_file_size(),
            skip_binary: true,
            skip_paths: Vec::new(),
            data_dir: None,
        }
    }
}

/// Parse a human-friendly byte size string into a `u64`.
///
/// Supports: `"1MB"`, `"512KB"`, `"2GB"`, `"1048576"`, `"1.5MB"`.
/// Case-insensitive. No suffix = raw bytes.
///
/// # Errors
///
/// Returns a descriptive error for unknown suffixes or non-numeric input.
pub fn parse_byte_size(s: &str) -> Result<u64, String> {
    let s = s.trim();
    let (num_part, suffix) = if let Some(pos) = s.find(|c: char| c.is_ascii_alphabetic()) {
        let (n, suf) = s.split_at(pos);
        (n, suf.trim())
    } else {
        (s, "")
    };

    let value: f64 = num_part
        .parse()
        .map_err(|_| format!("invalid byte size number: {num_part}"))?;

    let multiplier: u64 = match suffix.to_ascii_lowercase().as_str() {
        "" | "b" => 1,
        "kb" => 1024,
        "mb" => 1024 * 1024,
        "gb" => 1024 * 1024 * 1024,
        other => return Err(format!("unknown byte size suffix: {other}")),
    };

    let bytes = value * multiplier as f64;
    if bytes < 0.0 || bytes > u64::MAX as f64 {
        return Err(format!("byte size out of range: {s}"));
    }

    Ok(bytes as u64)
}

#[allow(clippy::unwrap_used)]
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let cfg = CodeIndexTomlConfig::default();
        assert!(cfg.enabled);
        assert!(cfg.extensions.is_empty());
        assert_eq!(cfg.max_file_size, "1MB");
        assert!(cfg.skip_binary);
        assert!(cfg.skip_paths.is_empty());
        assert!(cfg.data_dir.is_none());
    }

    #[test]
    fn test_parse_byte_size() {
        assert_eq!(parse_byte_size("1MB").unwrap(), 1_048_576);
        assert_eq!(parse_byte_size("512KB").unwrap(), 524_288);
        assert_eq!(parse_byte_size("2GB").unwrap(), 2_147_483_648);
        assert_eq!(parse_byte_size("1048576").unwrap(), 1_048_576);
        assert_eq!(parse_byte_size("1.5MB").unwrap(), 1_572_864);
        assert_eq!(parse_byte_size("1mb").unwrap(), 1_048_576);
        assert_eq!(parse_byte_size("1Mb").unwrap(), 1_048_576);
        assert_eq!(parse_byte_size("42").unwrap(), 42);
        assert!(parse_byte_size("1TB").is_err());
        assert!(parse_byte_size("abc").is_err());
    }

    #[test]
    fn test_serde_round_trip() {
        let cfg = CodeIndexTomlConfig::default();
        let toml = toml::to_string(&cfg).unwrap();
        let back: CodeIndexTomlConfig = toml::from_str(&toml).unwrap();
        assert_eq!(cfg.enabled, back.enabled);
        assert_eq!(cfg.max_file_size, back.max_file_size);
    }

    #[test]
    fn test_serde_with_overrides() {
        let toml = r#"
enabled = false
extensions = ["rs", "py"]
max_file_size = "2MB"
skip_paths = ["generated/"]
"#;
        let cfg: CodeIndexTomlConfig = toml::from_str(toml).unwrap();
        assert!(!cfg.enabled);
        assert_eq!(cfg.extensions, vec!["rs", "py"]);
        assert_eq!(cfg.max_file_size, "2MB");
        assert_eq!(cfg.skip_paths, vec!["generated/"]);
    }
}
