//! Moltis documentation exposed to agent prompts.

use std::{
    fs,
    path::{Component, Path, PathBuf},
};

use {
    include_dir::{Dir, DirEntry, include_dir},
    tracing::{debug, warn},
};

pub const MOLTIS_DOCS_URL: &str = "https://docs.moltis.org";
pub const BUNDLED_DOCS_RELATIVE_DIR: &str = "docs/moltis";
pub const CONFIG_TEMPLATE_DOC: &str = "config-template.md";

static BUNDLED_DOCS: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/../../docs/src");

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MoltisDocsReference {
    pub docs_dir: PathBuf,
    pub config_template_path: PathBuf,
}

#[must_use]
pub fn is_usable_moltis_docs_dir(path: &Path) -> bool {
    path.join("SUMMARY.md").is_file()
}

pub fn resolve_moltis_docs_reference(data_dir: &Path, port: u16) -> Option<MoltisDocsReference> {
    let config_template_path = write_config_template(data_dir, port);

    if let Some(dir) = std::env::var("MOLTIS_DOCS_DIR")
        .ok()
        .map(PathBuf::from)
        .filter(|dir| is_usable_moltis_docs_dir(dir))
    {
        debug!(path = %dir.display(), "using Moltis docs from MOLTIS_DOCS_DIR");
        return Some(MoltisDocsReference {
            docs_dir: dir,
            config_template_path,
        });
    }

    if let Some(dir) = moltis_config::share_dir()
        .map(|share| share.join("docs"))
        .filter(|dir| is_usable_moltis_docs_dir(dir))
    {
        debug!(path = %dir.display(), "using Moltis docs from external share dir");
        return Some(MoltisDocsReference {
            docs_dir: dir,
            config_template_path,
        });
    }

    let source_docs = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/src");
    if is_usable_moltis_docs_dir(&source_docs) {
        debug!(path = %source_docs.display(), "using Moltis docs from source tree");
        return Some(MoltisDocsReference {
            docs_dir: source_docs,
            config_template_path,
        });
    }

    ensure_embedded_moltis_docs_dir(data_dir).map(|docs_dir| MoltisDocsReference {
        docs_dir,
        config_template_path,
    })
}

fn write_config_template(data_dir: &Path, port: u16) -> PathBuf {
    let path = data_dir
        .join(BUNDLED_DOCS_RELATIVE_DIR)
        .join(CONFIG_TEMPLATE_DOC);
    let config_template = format!(
        "# Moltis configuration template\n\n```toml\n{}\n```\n",
        moltis_config::template::default_config_template(port)
    );
    let mut updated = 0usize;
    let mut errors = 0usize;
    write_if_changed(&path, config_template.as_bytes(), &mut updated, &mut errors);
    path
}

fn ensure_embedded_moltis_docs_dir(data_dir: &Path) -> Option<PathBuf> {
    let docs_dir = data_dir.join(BUNDLED_DOCS_RELATIVE_DIR);
    if let Err(error) = fs::create_dir_all(&docs_dir) {
        warn!(path = %docs_dir.display(), error = %error, "failed to create bundled Moltis docs directory");
        return None;
    }

    let mut updated = 0usize;
    let mut errors = 0usize;
    write_dir_entries(&docs_dir, BUNDLED_DOCS.entries(), &mut updated, &mut errors);

    if errors > 0 {
        warn!(path = %docs_dir.display(), errors, "failed to write some bundled Moltis docs");
    } else if updated > 0 {
        debug!(path = %docs_dir.display(), updated, "bundled Moltis docs refreshed");
    }

    is_usable_moltis_docs_dir(&docs_dir).then_some(docs_dir)
}

fn write_dir_entries(
    target_root: &Path,
    entries: &[DirEntry<'_>],
    updated: &mut usize,
    errors: &mut usize,
) {
    for entry in entries {
        match entry {
            DirEntry::Dir(dir) => write_dir_entries(target_root, dir.entries(), updated, errors),
            DirEntry::File(file) => {
                let Some(relative_path) = safe_relative_path(file.path()) else {
                    *errors += 1;
                    warn!(path = %file.path().display(), "skipping bundled doc with unsafe path");
                    continue;
                };
                write_if_changed(
                    &target_root.join(relative_path),
                    file.contents(),
                    updated,
                    errors,
                );
            },
        }
    }
}

fn safe_relative_path(path: &Path) -> Option<PathBuf> {
    let mut cleaned = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => cleaned.push(part),
            Component::CurDir => {},
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    (!cleaned.as_os_str().is_empty()).then_some(cleaned)
}

fn write_if_changed(path: &Path, content: &[u8], updated: &mut usize, errors: &mut usize) {
    if fs::read(path).is_ok_and(|existing| existing == content) {
        return;
    }
    if let Some(parent) = path.parent()
        && let Err(error) = fs::create_dir_all(parent)
    {
        *errors += 1;
        warn!(path = %parent.display(), error = %error, "failed to create bundled doc parent directory");
        return;
    }
    match fs::write(path, content) {
        Ok(()) => *updated += 1,
        Err(error) => {
            *errors += 1;
            warn!(path = %path.display(), error = %error, "failed to write bundled Moltis doc");
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn materializes_bundled_docs_and_config_template() {
        let temp = tempfile::tempdir().unwrap_or_else(|error| panic!("tempdir failed: {error}"));

        let reference = resolve_moltis_docs_reference(temp.path(), 18789)
            .unwrap_or_else(|| panic!("docs reference was not resolved"));

        assert!(reference.docs_dir.join("SUMMARY.md").is_file());
        let config_template = fs::read_to_string(reference.config_template_path)
            .unwrap_or_else(|error| panic!("read config template failed: {error}"));
        assert!(config_template.contains("# Moltis configuration template"));
        assert!(config_template.contains("port = 18789"));
    }

    #[test]
    fn prefers_external_docs_dir_when_available() {
        let data = tempfile::tempdir().unwrap_or_else(|error| panic!("tempdir failed: {error}"));
        let share = tempfile::tempdir().unwrap_or_else(|error| panic!("tempdir failed: {error}"));
        let docs = share.path().join("docs");
        fs::create_dir_all(&docs).unwrap_or_else(|error| panic!("create docs failed: {error}"));
        fs::write(docs.join("SUMMARY.md"), "# Summary\n")
            .unwrap_or_else(|error| panic!("write summary failed: {error}"));

        moltis_config::set_share_dir(share.path().to_path_buf());
        let reference = resolve_moltis_docs_reference(data.path(), 18789)
            .unwrap_or_else(|| panic!("docs reference was not resolved"));
        moltis_config::clear_share_dir();

        assert_eq!(reference.docs_dir, docs);
        assert!(reference.config_template_path.starts_with(data.path()));
    }

    #[test]
    fn usable_docs_dir_requires_summary() {
        let temp = tempfile::tempdir().unwrap_or_else(|error| panic!("tempdir failed: {error}"));
        assert!(!is_usable_moltis_docs_dir(temp.path()));

        fs::write(temp.path().join("SUMMARY.md"), "# Summary\n")
            .unwrap_or_else(|error| panic!("write summary failed: {error}"));
        assert!(is_usable_moltis_docs_dir(temp.path()));
    }
}
