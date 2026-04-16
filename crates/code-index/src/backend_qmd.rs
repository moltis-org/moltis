//! QMD backend for code indexing.
//!
//! Delegates indexing and search operations to the QMD sidecar process
//! via the existing [`moltis_qmd::QmdManager`].

#[cfg(feature = "qmd")]
use moltis_qmd::{QmdCollection, QmdManagerConfig};

use crate::config::CodeIndexConfig;

// P2 will use Error, Result, and IndexStatus when wiring the full backend.
#[allow(unused_imports)]
use crate::error::Result;
#[allow(unused_imports)]
use crate::types::IndexStatus;

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
/// Uses the project ID as the QMD collection name and configures
/// the manager to point at the project directory.
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