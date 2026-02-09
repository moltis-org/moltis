use std::{fs, path::PathBuf};

/// Return directory completions for a partial path string.
///
/// Given a partial path like `/home/user/pro`, returns directories matching
/// that prefix (e.g. `/home/user/projects/`, `/home/user/proto/`).
pub fn complete_path(partial: &str) -> Vec<PathBuf> {
    if partial.is_empty() {
        return Vec::new();
    }

    let path = PathBuf::from(partial);

    // If partial ends with '/', list children of that directory
    if partial.ends_with('/') || partial.ends_with(std::path::MAIN_SEPARATOR) {
        return list_subdirs(&path);
    }

    // Otherwise, list siblings that match the prefix
    let parent = match path.parent() {
        Some(p) if p.as_os_str().is_empty() => return Vec::new(),
        Some(p) => p,
        None => return Vec::new(),
    };

    let prefix = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_lowercase();

    let Ok(entries) = fs::read_dir(parent) else {
        return Vec::new();
    };

    let mut results: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .to_lowercase()
                .starts_with(&prefix)
        })
        .map(|e| e.path())
        .collect();

    results.sort();
    results
}

fn list_subdirs(dir: &PathBuf) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut dirs: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
        .map(|e| e.path())
        .collect();
    dirs.sort();
    dirs
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_complete_empty() {
        assert!(complete_path("").is_empty());
    }

    #[test]
    fn test_complete_lists_subdirs() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("alpha")).unwrap();
        std::fs::create_dir(dir.path().join("beta")).unwrap();
        std::fs::create_dir(dir.path().join(".hidden")).unwrap();

        let path_str = format!("{}/", dir.path().display());
        let results = complete_path(&path_str);
        assert_eq!(results.len(), 2); // excludes .hidden
    }

    #[test]
    fn test_complete_prefix_match() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("alpha")).unwrap();
        std::fs::create_dir(dir.path().join("alpine")).unwrap();
        std::fs::create_dir(dir.path().join("beta")).unwrap();

        let partial = format!("{}/al", dir.path().display());
        let results = complete_path(&partial);
        assert_eq!(results.len(), 2);
    }
}
