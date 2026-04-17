//! File filtering for codebase indexing.
//!
//! Takes discovered git-tracked files and filters them down to text-equivalent
//! code files based on extension, size, binary detection, and path exclusions.

use std::{fs, path::Path};

use sha2::{Digest, Sha256};

#[cfg(feature = "tracing")]
use crate::log::{debug, info, trace};

use crate::{
    config::CodeIndexConfig,
    error::Result,
    types::{FilteredFile, Language},
};

/// Maximum bytes to read for binary detection.
const BINARY_CHECK_BYTES: u64 = 8192;

/// Null byte — if found in the first `BINARY_CHECK_BYTES`, the file is binary.
const NULL_BYTE: u8 = 0;

/// Lightweight filter configuration used by the file watcher.
#[derive(Debug, Clone)]
pub struct FilterConfig {
    /// File extensions to index (without leading dot).
    pub extensions: Vec<String>,
    /// Path prefixes to skip.
    pub skip_paths: Vec<String>,
}

/// Filter a list of relative file paths according to the config.
///
/// For each file: check extension, check size, check for binary content,
/// check skip paths. Returns the files that pass all filters, with
/// language hints and size metadata.
pub fn filter_tracked_files(
    repo_dir: &Path,
    relative_paths: &[std::path::PathBuf],
    config: &CodeIndexConfig,
) -> Result<Vec<FilteredFile>> {
    let mut accepted = Vec::new();
    let mut skipped_extension = 0usize;
    let mut skipped_size = 0usize;
    let mut skipped_binary = 0usize;
    let mut skipped_path = 0usize;

    for rel_path in relative_paths {
        let path_str = rel_path.to_string_lossy();

        // 1. Path skip check (cheapest — no I/O).
        if config.path_skipped(&path_str) {
            #[cfg(feature = "tracing")]
            debug!(path = %path_str, "skipped: path exclusion");
            skipped_path += 1;
            continue;
        }

        // 2. Extension check (handles extensionless files like Dockerfile).
        let effective_ext = effective_extension(rel_path);

        if !config.extension_allowed(effective_ext) {
            #[cfg(feature = "tracing")]
            trace!(path = %path_str, ext = %effective_ext, "skipped: extension not allowed");
            skipped_extension += 1;
            continue;
        }

        // 3. Size check.
        let abs_path = repo_dir.join(rel_path);
        let metadata = match fs::metadata(&abs_path) {
            Ok(m) => m,
            Err(e) => {
                #[cfg(feature = "tracing")]
                debug!(path = %path_str, error = %e, "skipped: cannot read metadata");
                continue;
            },
        };
        let size = metadata.len();
        if size > config.max_file_size_bytes {
            #[cfg(feature = "tracing")]
            debug!(
                path = %path_str,
                size,
                max = config.max_file_size_bytes,
                "skipped: file too large"
            );
            skipped_size += 1;
            continue;
        }

        // 4. Binary detection (read first 8 KiB, check for null bytes).
        if config.skip_binary && is_binary_file(&abs_path, size) {
            #[cfg(feature = "tracing")]
            debug!(path = %path_str, "skipped: binary content detected");
            skipped_binary += 1;
            continue;
        }

        let language = Language::from_extension(effective_ext);

        accepted.push(FilteredFile {
            path: abs_path,
            relative_path: rel_path.clone(),
            size,
            language,
        });
    }

    #[cfg(feature = "tracing")]
    info!(
        accepted = accepted.len(),
        skipped_extension, skipped_size, skipped_binary, skipped_path, "file filtering complete"
    );

    Ok(accepted)
}

/// Check if a file appears to be binary by looking for null bytes
/// in the first `BINARY_CHECK_BYTES` of content.
fn is_binary_file(path: &Path, size: u64) -> bool {
    if size == 0 {
        return false;
    }

    let mut file = match fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return true, // If we can't open it, treat as binary.
    };

    use std::io::Read;
    let check_len = std::cmp::min(size, BINARY_CHECK_BYTES) as usize;
    let mut buf = vec![0u8; check_len];
    match file.read(&mut buf) {
        Ok(n) => buf[..n].contains(&NULL_BYTE),
        Err(_) => true,
    }
}

/// Compute a SHA-256 content hash for a file.
/// Used for change detection (skip re-indexing if hash matches).
pub fn content_hash(path: &Path) -> Result<String> {
    let data = fs::read(path)?;
    let mut hasher = Sha256::new();
    hasher.update(&data);
    Ok(format!("{:x}", hasher.finalize()))
}

/// Determine the effective file extension for extension and language detection.
///
/// For files with a normal extension (e.g. `main.rs` → `"rs"`), returns it as-is.
/// For known extensionless filenames (Dockerfile, Makefile),
/// returns a synthetic extension mapping them to their language.
pub fn effective_extension(path: &Path) -> &str {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");

    if !ext.is_empty() {
        return ext;
    }

    // No extension — map known extensionless filenames.
    let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");

    match file_name.to_ascii_lowercase().as_str() {
        "dockerfile" | "containerfile" => "dockerfile",
        "makefile" | "gnumakefile" => "mk",
        _ => "",
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use std::path::PathBuf;

    use {super::*, crate::config::CodeIndexConfig};

    fn test_config() -> CodeIndexConfig {
        CodeIndexConfig::default()
    }

    /// Create a temp directory with the given file structure for filter testing.
    /// Each entry is `(relative_path, content)`.
    fn setup_temp_dir(files: &[(&str, &[u8])]) -> tempfile::TempDir {
        let dir = tempfile::TempDir::new().unwrap();
        for (rel_path, content) in files {
            let full_path = dir.path().join(rel_path);
            fs::create_dir_all(full_path.parent().unwrap()).unwrap();
            fs::write(&full_path, content).unwrap();
        }
        dir
    }

    #[test]
    fn test_filter_by_extension() {
        let dir = setup_temp_dir(&[
            ("src/main.rs", b"fn main() {}".as_slice()),
            ("src/lib.py", b"def hello(): pass".as_slice()),
            ("assets/logo.png", b"\x89PNG".as_slice()),
            ("data/output.bin", b"\x00\x01\x02".as_slice()),
        ]);
        let config = test_config();
        let files = vec![
            PathBuf::from("src/main.rs"),
            PathBuf::from("src/lib.py"),
            PathBuf::from("assets/logo.png"),
            PathBuf::from("data/output.bin"),
        ];

        let filtered = filter_tracked_files(dir.path(), &files, &config).unwrap();
        // rs and py should pass; png should be excluded by extension.
        assert!(
            filtered
                .iter()
                .any(|f| f.relative_path.ends_with("main.rs")),
            "main.rs should pass the extension filter"
        );
        assert!(
            filtered.iter().any(|f| f.relative_path.ends_with("lib.py")),
            "lib.py should pass the extension filter"
        );
        assert!(
            !filtered
                .iter()
                .any(|f| f.relative_path.ends_with("logo.png")),
            "logo.png should be excluded by extension"
        );
        // output.bin has a null byte so it should be excluded by binary detection
        assert!(
            !filtered
                .iter()
                .any(|f| f.relative_path.ends_with("output.bin")),
            "output.bin should be excluded (binary)"
        );
    }

    #[test]
    fn test_filter_by_path() {
        let dir = setup_temp_dir(&[
            ("src/main.rs", b"fn main() {}".as_slice()),
            ("vendor/lib/foo.rs", b"fn foo() {}".as_slice()),
            (
                "node_modules/react/index.js",
                b"module.exports = {};".as_slice(),
            ),
        ]);
        let config = test_config();
        let files = vec![
            PathBuf::from("src/main.rs"),
            PathBuf::from("vendor/lib/foo.rs"),
            PathBuf::from("node_modules/react/index.js"),
        ];

        let filtered = filter_tracked_files(dir.path(), &files, &config).unwrap();
        assert!(
            filtered
                .iter()
                .any(|f| f.relative_path.ends_with("main.rs")),
            "src/main.rs should pass"
        );
        assert!(
            !filtered
                .iter()
                .any(|f| f.relative_path.display().to_string().contains("vendor")),
            "vendor paths should be skipped"
        );
        assert!(
            !filtered.iter().any(|f| f
                .relative_path
                .display()
                .to_string()
                .contains("node_modules")),
            "node_modules paths should be skipped"
        );
    }

    #[test]
    fn test_language_detection() {
        assert_eq!(Language::from_extension("rs"), Language::Rust);
        assert_eq!(Language::from_extension("py"), Language::Python);
        assert_eq!(Language::from_extension("JS"), Language::JavaScript);
        assert_eq!(Language::from_extension("tsx"), Language::TypeScript);
        assert_eq!(Language::from_extension("png"), Language::Unknown);
    }
}
