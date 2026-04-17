//! QMD backend for code indexing.
//!
//! Creates [`QmdCollection`] entries scoped to a single project, using the
//! code-index extension allowlist as the QMD glob mask.
//!
//! QMD's `--mask` flag accepts a single glob pattern per collection, so
//! we emit one [`QmdCollection`] per extension.

#[cfg(feature = "qmd")]
use moltis_qmd::{QmdCollection, QmdManagerConfig};

use crate::config::CodeIndexConfig;

// P2 will use Error, Result, and IndexStatus when wiring the full backend.
#[allow(unused_imports)]
use crate::error::Result;
#[allow(unused_imports)]
use crate::types::IndexStatus;

/// Build QMD collection configurations for a project.
///
/// Creates one [`QmdCollection`] per extension in the allowlist, each
/// targeting the project directory. Multiple collections are needed
/// because QMD's `--mask` flag accepts a single glob pattern per
/// collection.
#[cfg(feature = "qmd")]
pub fn project_collections(
    project_dir: &std::path::Path,
    _project_id: &str,
    config: &CodeIndexConfig,
) -> Vec<QmdCollection> {
    config
        .extensions
        .iter()
        .map(|ext| QmdCollection {
            path: project_dir.to_path_buf(),
            glob: format!("**/*.{ext}"),
        })
        .collect()
}

/// Build a [`QmdManagerConfig`] for code indexing.
///
/// Registers one collection per extension keyed by
/// `{project_id}-{extension}` and sets the QMD index name to
/// `code-{project_id}`.
#[cfg(feature = "qmd")]
pub fn qmd_config_for_project(
    project_dir: &std::path::Path,
    work_dir: &std::path::Path,
    project_id: &str,
    config: &CodeIndexConfig,
) -> QmdManagerConfig {
    let mut collections = std::collections::HashMap::new();

    for collection in project_collections(project_dir, project_id, config) {
        // Derive a stable key from the glob pattern (e.g. "project-rs").
        let ext_key = collection
            .glob
            .strip_prefix("**/*.")
            .unwrap_or(&collection.glob);
        let key = format!("{project_id}-{ext_key}");
        collections.insert(key, collection);
    }

    QmdManagerConfig {
        collections,
        index_name: format!("code-{project_id}"),
        work_dir: work_dir.to_path_buf(),
        ..QmdManagerConfig::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_project_collections_emits_per_extension() {
        let config = CodeIndexConfig {
            extensions: vec!["rs".into(), "py".into()],
            ..CodeIndexConfig::default()
        };
        let colls = project_collections(
            std::path::Path::new("/tmp/test-repo"),
            "test-project",
            &config,
        );
        assert_eq!(colls.len(), 2);
        assert_eq!(colls[0].glob, "**/*.rs");
        assert_eq!(colls[1].glob, "**/*.py");
        assert_eq!(colls[0].path, std::path::PathBuf::from("/tmp/test-repo"));
    }

    #[test]
    fn test_qmd_config_for_project_sets_work_dir() {
        let config = CodeIndexConfig {
            extensions: vec!["rs".into()],
            ..CodeIndexConfig::default()
        };
        let qmc = qmd_config_for_project(
            std::path::Path::new("/tmp/test-repo"),
            std::path::Path::new("/tmp/work"),
            "test-project",
            &config,
        );
        assert_eq!(qmc.work_dir, std::path::PathBuf::from("/tmp/work"));
        assert_eq!(qmc.index_name, "code-test-project");
        assert!(qmc.collections.contains_key("test-project-rs"));
    }
}
