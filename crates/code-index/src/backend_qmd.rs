//! QMD backend for code indexing.
//!
//! Creates [`QmdManagerConfig`] and [`QmdCollection`] entries scoped to
//! a single project, using the code-index extension allowlist as the QMD
//! glob mask.

#[cfg(feature = "qmd")]
use moltis_qmd::{QmdCollection, QmdManagerConfig};

use crate::config::CodeIndexConfig;

/// Build a QMD collection configuration for a project.
///
/// Creates a QMD collection that targets the project directory,
/// filtered by the code index config's extension allowlist.
#[cfg(feature = "qmd")]
pub fn project_collection_config(
    project_dir: &std::path::Path,
    _project_id: &str,
    config: &CodeIndexConfig,
) -> QmdCollection {
    let mut globs = Vec::new();
    for ext in &config.extensions {
        globs.push(format!("**/*.{ext}"));
    }

    QmdCollection {
        path: project_dir.to_path_buf(),
        glob: globs.join(","),
    }
}

/// Build a [`QmdManagerConfig`] for code indexing.
///
/// Registers a single collection keyed by `project_id` and sets the
/// QMD index name to `code-{project_id}`.
#[cfg(feature = "qmd")]
pub fn qmd_config_for_project(
    project_dir: &std::path::Path,
    project_id: &str,
    config: &CodeIndexConfig,
) -> QmdManagerConfig {
    let mut collections = std::collections::HashMap::new();
    collections.insert(
        project_id.to_string(),
        project_collection_config(project_dir, project_id, config),
    );

    QmdManagerConfig {
        collections,
        index_name: format!("code-{project_id}"),
        ..QmdManagerConfig::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_project_collection_config_includes_extensions() {
        let config = CodeIndexConfig {
            extensions: vec!["rs".into(), "py".into()],
            ..CodeIndexConfig::default()
        };
        let coll = project_collection_config(
            std::path::Path::new("/tmp/test-repo"),
            "test-project",
            &config,
        );
        // The glob should contain patterns for rs and py.
        assert!(coll.glob.contains("*.rs"));
        assert!(coll.glob.contains("*.py"));
        assert_eq!(coll.path, std::path::PathBuf::from("/tmp/test-repo"));
    }
}
