use std::path::{Component, Path, PathBuf};

/// Resolve a user-supplied path component relative to a data directory,
/// rejecting absolute paths, empty strings, and `..` traversal.
///
/// Rust's [`Path::join`] silently discards the base when given an absolute
/// component (`base.join("/etc/shadow") == "/etc/shadow"`) and does not
/// normalize `..` segments. Both would allow a config-driven `db_path` field
/// to create zvec collection and cache files outside the intended data
/// directory.
///
/// This function validates the component string before joining and returns
/// the resolved path only when it stays logically within `data_dir`.
pub fn resolve_data_subpath(data_dir: &Path, component: &str) -> anyhow::Result<PathBuf> {
    let component_path = Path::new(component);

    anyhow::ensure!(
        !component.is_empty(),
        "memory path component must not be empty"
    );
    anyhow::ensure!(
        !component_path.is_absolute(),
        "memory path must be relative, got {component:?}"
    );
    anyhow::ensure!(
        !component_path
            .components()
            .any(|c| matches!(c, Component::ParentDir)),
        "memory path must not contain .. traversal: {component:?}"
    );

    Ok(data_dir.join(component_path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absolute_components_are_rejected() {
        match resolve_data_subpath(Path::new("/data"), "/etc/shadow") {
            Err(e) => assert!(e.to_string().contains("relative")),
            Ok(p) => panic!("expected error for absolute path, got {p:?}"),
        }
    }

    #[test]
    fn empty_components_are_rejected() {
        assert!(resolve_data_subpath(Path::new("/data"), "").is_err());
    }

    #[test]
    fn parent_dir_traversal_is_rejected() {
        match resolve_data_subpath(Path::new("/data"), "../.ssh/authorized_keys") {
            Err(e) => assert!(e.to_string().contains("..")),
            Ok(p) => panic!("expected error for .. path, got {p:?}"),
        }
    }

    #[test]
    fn internal_parent_dir_traversal_is_rejected() {
        match resolve_data_subpath(Path::new("/data"), "foo/../../etc") {
            Err(e) => assert!(e.to_string().contains("..")),
            Ok(p) => panic!("expected error for ../.. path, got {p:?}"),
        }
    }

    #[test]
    fn relative_sub_path_is_allowed() {
        match resolve_data_subpath(Path::new("/data"), "memory.zvec") {
            Ok(resolved) => assert_eq!(resolved, Path::new("/data/memory.zvec")),
            Err(e) => panic!("expected ok for relative path, got {e}"),
        }
    }

    #[test]
    fn nested_relative_sub_path_is_allowed() {
        match resolve_data_subpath(Path::new("/data"), "collections/memory.zvec") {
            Ok(resolved) => assert_eq!(resolved, Path::new("/data/collections/memory.zvec")),
            Err(e) => panic!("expected ok for nested path, got {e}"),
        }
    }
}
