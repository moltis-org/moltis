//! Configuration for codebase indexing.

use std::sync::LazyLock;

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Default maximum file size for indexing: 1 MB.
const DEFAULT_MAX_FILE_SIZE_BYTES: u64 = 1024 * 1024;

/// Default extensions considered text-equivalent and safe to index.
static DEFAULT_EXTENSIONS: LazyLock<Vec<String>> = LazyLock::new(|| {
    [
        // Systems languages
        "rs",
        "c",
        "h",
        "cpp",
        "hpp",
        "cc",
        "cxx",
        "go",
        "zig",
        // Scripting languages
        "py",
        "pyi",
        "rb",
        "php",
        "sh",
        "bash",
        // JVM languages
        "java",
        "kt",
        "kts",
        "scala",
        // Web languages
        "js",
        "jsx",
        "mjs",
        "cjs",
        "ts",
        "tsx",
        "mts",
        "cts",
        "css",
        "scss",
        "less",
        "html",
        "htm",
        // .NET
        "cs",
        // Apple
        "swift",
        // Data / config
        "sql",
        "json",
        "toml",
        "yaml",
        "yml",
        "nix",
        // Documentation
        "md",
        "markdown",
        "mdx",
        "txt",
        // Docker / CI
        "dockerfile",
        "containerfile",
        // DSLs
        "graphql",
        "proto",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
});

/// Default path patterns to skip during indexing.
static DEFAULT_SKIP_PATHS: LazyLock<Vec<String>> = LazyLock::new(|| {
    [
        "vendor/",
        "third_party/",
        "node_modules/",
        "__pycache__/",
        ".venv/",
        "venv/",
        "dist/",
        "build/",
        "target/",
        ".next/",
        ".nuxt/",
        "coverage/",
        ".tox/",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
});

/// Configuration for codebase indexing, per-project or global.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CodeIndexConfig {
    /// Whether code indexing is enabled for this project.
    pub enabled: bool,
    /// File extensions to index (without leading dot).
    /// Empty means use the default set.
    pub extensions: Vec<String>,
    /// Maximum file size in bytes. Files larger than this are skipped.
    pub max_file_size_bytes: u64,
    /// Whether to skip files that appear to be binary (contain null bytes).
    pub skip_binary: bool,
    /// Path prefixes to skip (e.g. "vendor/", "node_modules/").
    pub skip_paths: Vec<String>,
    /// Root directory for snapshot storage.
    /// Defaults to `<moltis_data_dir>/code-index/` when `None`.
    #[serde(skip)]
    pub data_dir: Option<PathBuf>,
}

impl Default for CodeIndexConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            extensions: DEFAULT_EXTENSIONS.clone(),
            max_file_size_bytes: DEFAULT_MAX_FILE_SIZE_BYTES,
            skip_binary: true,
            skip_paths: DEFAULT_SKIP_PATHS.clone(),
            data_dir: None,
        }
    }
}

impl CodeIndexConfig {
    /// Create a config with only the given extensions, replacing defaults.
    #[must_use]
    pub fn with_extensions(mut self, extensions: Vec<String>) -> Self {
        self.extensions = extensions;
        self
    }

    /// Check whether a file extension (without dot) is in the allowlist.
    pub fn extension_allowed(&self, ext: &str) -> bool {
        let ext_lower = ext.to_ascii_lowercase();
        self.extensions
            .iter()
            .any(|e| e.to_ascii_lowercase() == ext_lower)
    }

    /// Check whether a relative path matches any skip pattern.
    ///
    /// Matches at any directory depth — `vendor` matches both `vendor/foo.rs`
    /// and `src/vendor/foo.rs`. Patterns are treated as path segments unless
    /// they already contain a `/`.
    pub fn path_skipped(&self, relative_path: &str) -> bool {
        let path_lower = relative_path.to_ascii_lowercase();
        // Normalize separators.
        let path_forward = path_lower.replace('\\', "/");
        self.skip_paths.iter().any(|pattern| {
            // Strip trailing slashes so "vendor/" is treated as "vendor" for
            // segment matching.
            let p = pattern.trim_end_matches('/').to_ascii_lowercase();
            // Exact prefix match (handles both "vendor/foo" and "vendor/").
            if path_forward.starts_with(&p) || path_forward.starts_with(&format!("{p}/")) {
                return true;
            }
            // Segment match: check if any path component equals the pattern.
            // This catches "src/vendor/foo.rs" when pattern is "vendor".
            if !p.contains('/') {
                path_forward.split('/').any(|segment| segment == p.as_str())
            } else {
                false
            }
        })
    }

    /// Return a [`FilterConfig`](crate::filter::FilterConfig) for the file watcher.
    #[must_use]
    pub fn filter(&self) -> crate::filter::FilterConfig {
        crate::filter::FilterConfig {
            extensions: self.extensions.clone(),
            skip_paths: self.skip_paths.clone(),
        }
    }

    /// Return a [`ChunkerConfig`](crate::chunker::ChunkerConfig) for the chunker.
    #[must_use]
    pub fn chunker(&self) -> crate::chunker::ChunkerConfig {
        crate::chunker::ChunkerConfig::default()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config_has_extensions() {
        let config = CodeIndexConfig::default();
        assert!(!config.extensions.is_empty());
        assert!(config.enabled);
        assert!(config.skip_binary);
    }

    #[test]
    fn test_extension_allowed() {
        let config = CodeIndexConfig::default();
        assert!(config.extension_allowed("rs"));
        assert!(config.extension_allowed("py"));
        assert!(config.extension_allowed("JS")); // case-insensitive
        assert!(!config.extension_allowed("png"));
        assert!(!config.extension_allowed("exe"));
    }

    #[test]
    fn test_path_skipped() {
        let config = CodeIndexConfig::default();
        assert!(config.path_skipped("vendor/lib/foo.rs"));
        assert!(config.path_skipped("node_modules/react/index.js"));
        assert!(config.path_skipped("target/debug/moltis"));
        assert!(!config.path_skipped("src/main.rs"));
        // Nested path matching — pattern at non-root depth.
        assert!(config.path_skipped("src/vendor/lib/foo.rs"));
        assert!(config.path_skipped("packages/node_modules/pkg/index.js"));
    }

    #[test]
    fn test_serde_round_trip() {
        let config = CodeIndexConfig::default();
        let json = serde_json::to_string(&config).unwrap();
        let deserialized: CodeIndexConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(config.extensions, deserialized.extensions);
        assert_eq!(config.max_file_size_bytes, deserialized.max_file_size_bytes);
        assert_eq!(config.skip_binary, deserialized.skip_binary);
    }

    #[test]
    fn test_with_extensions() {
        let config = CodeIndexConfig::default().with_extensions(vec!["rs".into()]);
        assert!(config.extension_allowed("rs"));
        assert!(!config.extension_allowed("py"));
    }
}
