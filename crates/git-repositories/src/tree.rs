use std::{
    collections::BTreeSet,
    fs::{self, File},
    io::Write,
    path::{Component, Path, PathBuf},
};

use {gix::object::tree::EntryKind, serde::Deserialize, serde_json::Value};

use crate::{Error, MaterializationLimits};

const MOLTIS_MANIFEST: &str = ".moltis/mcp-repository.json";
const MARKETPLACE_MANIFEST: &str = ".claude-plugin/marketplace.json";
const ROOT_MCP_MANIFEST: &str = ".mcp.json";
const CLAUDE_PLUGIN_MANIFEST: &str = ".claude-plugin/plugin.json";
const CODEX_PLUGIN_MANIFEST: &str = ".codex-plugin/plugin.json";
const ROOT_RUNTIME_FILES: &[&str] = &[
    "package.json",
    "package-lock.json",
    "npm-shrinkwrap.json",
    "pnpm-lock.yaml",
    "yarn.lock",
    "bun.lock",
    "bun.lockb",
    "pyproject.toml",
    "uv.lock",
    "requirements.txt",
    "Pipfile",
    "Pipfile.lock",
    "Cargo.toml",
    "Cargo.lock",
    "go.mod",
    "go.sum",
];

pub(crate) fn resolve_commit(
    repo: &gix::Repository,
    revision: &str,
) -> Result<gix::ObjectId, Error> {
    let id = repo
        .rev_parse_single(revision)
        .map_err(|error| Error::ResolveRevision {
            revision: revision.to_string(),
            message: error.to_string(),
        })?;
    let object = id
        .object()
        .map_err(|error| Error::Object(error.to_string()))?;
    let commit = object.peel_to_commit().map_err(|_| Error::NotACommit {
        revision: revision.to_string(),
    })?;
    Ok(commit.id)
}

/// Materialize only explicit MCP content selected directly from the committed Git tree.
pub(crate) fn write_explicit_mcp_tree(
    repo: &gix::Repository,
    commit_id: gix::ObjectId,
    destination: &Path,
    limits: MaterializationLimits,
) -> Result<(), Error> {
    let commit = repo
        .find_object(commit_id)
        .map_err(|error| Error::Object(error.to_string()))?
        .peel_to_commit()
        .map_err(|error| Error::Object(error.to_string()))?;
    let tree = commit
        .tree()
        .map_err(|error| Error::Object(error.to_string()))?;
    let selection = select_mcp_content(&tree, limits)?;
    let mut state = TreeState {
        entries: 0,
        bytes: 0,
        limits,
    };

    for path in &selection.files {
        write_selected_path(&tree, destination, path, &mut state)?;
    }
    for path in &selection.recursive_roots {
        if path.as_os_str().is_empty() {
            write_tree(&tree, destination, destination, 0, &mut state)?;
        } else {
            write_selected_path(&tree, destination, path, &mut state)?;
        }
    }
    Ok(())
}

#[derive(Default)]
struct Selection {
    files: BTreeSet<PathBuf>,
    recursive_roots: BTreeSet<PathBuf>,
}

impl Selection {
    fn add_recursive_root(&mut self, path: PathBuf) {
        if self
            .recursive_roots
            .iter()
            .any(|selected| path.starts_with(selected))
        {
            return;
        }
        self.recursive_roots
            .retain(|selected| !selected.starts_with(&path));
        self.files.retain(|selected| !selected.starts_with(&path));
        self.recursive_roots.insert(path);
    }

    fn add_file(&mut self, path: PathBuf) {
        if !self
            .recursive_roots
            .iter()
            .any(|selected| path.starts_with(selected))
        {
            self.files.insert(path);
        }
    }
}

struct ManifestState {
    count: usize,
    bytes: u64,
    plugin_roots: usize,
    limits: MaterializationLimits,
}

impl ManifestState {
    fn new(limits: MaterializationLimits) -> Self {
        Self {
            count: 0,
            bytes: 0,
            plugin_roots: 0,
            limits,
        }
    }

    fn record_manifest(&mut self, path: &Path, size: u64) -> Result<(), Error> {
        self.count = self.count.saturating_add(1);
        if self.count > self.limits.max_manifests {
            return Err(Error::LimitExceeded(format!(
                "repository has more than {} MCP manifests",
                self.limits.max_manifests
            )));
        }
        let max_bytes = self
            .limits
            .max_manifest_bytes
            .min(self.limits.max_blob_bytes);
        if size > max_bytes {
            return Err(Error::LimitExceeded(format!(
                "manifest {} is larger than {} bytes",
                path.display(),
                max_bytes
            )));
        }
        self.bytes = self.bytes.saturating_add(size);
        if self.bytes > self.limits.max_total_bytes {
            return Err(Error::LimitExceeded(format!(
                "MCP manifests are larger than {} bytes",
                self.limits.max_total_bytes
            )));
        }
        Ok(())
    }

    fn record_plugin_root(&mut self) -> Result<(), Error> {
        self.plugin_roots = self.plugin_roots.saturating_add(1);
        if self.plugin_roots > self.limits.max_plugin_roots {
            return Err(Error::LimitExceeded(format!(
                "repository declares more than {} plugin roots",
                self.limits.max_plugin_roots
            )));
        }
        Ok(())
    }
}

#[derive(Deserialize)]
struct MoltisRepositoryManifest {
    #[serde(default)]
    version: Option<u32>,
    #[serde(default)]
    plugins: Vec<MoltisPlugin>,
}

#[derive(Deserialize)]
struct MoltisPlugin {
    root: String,
}

#[derive(Deserialize)]
struct MarketplaceManifest {
    #[serde(default)]
    metadata: MarketplaceMetadata,
    #[serde(default)]
    plugins: Vec<MarketplacePlugin>,
}

#[derive(Default, Deserialize)]
struct MarketplaceMetadata {
    #[serde(default, rename = "pluginRoot")]
    plugin_root: Option<String>,
}

#[derive(Deserialize)]
struct MarketplacePlugin {
    source: Value,
}

#[derive(Deserialize)]
struct PluginManifest {
    #[serde(default, rename = "mcpServers")]
    mcp_servers: Option<Value>,
}

#[derive(Deserialize)]
struct RootMcpManifest {
    #[serde(default, rename = "mcpServers")]
    mcp_servers: std::collections::BTreeMap<String, RootServerConfig>,
}

#[derive(Deserialize)]
struct RootServerConfig {
    #[serde(default)]
    command: Option<String>,
    #[serde(default)]
    args: Vec<String>,
}

fn select_mcp_content(
    tree: &gix::Tree<'_>,
    limits: MaterializationLimits,
) -> Result<Selection, Error> {
    let mut state = ManifestState::new(limits);
    if tree_entry_kind(tree, Path::new(MOLTIS_MANIFEST))?.is_some() {
        return select_moltis_content(tree, &mut state);
    }
    if tree_entry_kind(tree, Path::new(MARKETPLACE_MANIFEST))?.is_some() {
        return select_marketplace_content(tree, &mut state);
    }
    if tree_entry_kind(tree, Path::new(ROOT_MCP_MANIFEST))?.is_some() {
        return select_root_mcp_content(tree, &mut state);
    }
    Err(Error::NoSupportedManifest)
}

fn select_moltis_content(
    tree: &gix::Tree<'_>,
    state: &mut ManifestState,
) -> Result<Selection, Error> {
    let path = PathBuf::from(MOLTIS_MANIFEST);
    let manifest: MoltisRepositoryManifest = read_manifest(tree, &path, state)?;
    if manifest.version.is_some_and(|version| version != 1) {
        return Err(Error::InvalidManifest {
            path,
            message: "only version 1 is supported".into(),
        });
    }

    let mut selection = Selection::default();
    selection.add_file(PathBuf::from(MOLTIS_MANIFEST));
    for plugin in manifest.plugins {
        state.record_plugin_root()?;
        let root = safe_declared_path(&plugin.root)?;
        require_tree(tree, &root)?;
        selection.add_recursive_root(root);
    }
    Ok(selection)
}

fn select_marketplace_content(
    tree: &gix::Tree<'_>,
    state: &mut ManifestState,
) -> Result<Selection, Error> {
    let manifest_path = PathBuf::from(MARKETPLACE_MANIFEST);
    let marketplace: MarketplaceManifest = read_manifest(tree, &manifest_path, state)?;
    let base_root = marketplace
        .metadata
        .plugin_root
        .as_deref()
        .map(safe_declared_path)
        .transpose()?
        .unwrap_or_default();

    let mut selection = Selection::default();
    selection.add_file(manifest_path);
    for plugin in marketplace.plugins {
        state.record_plugin_root()?;
        let Some(source) = plugin.source.as_str() else {
            continue;
        };
        let source = safe_declared_path(source)?;
        let plugin_root = join_declared_paths(&base_root, &source)?;
        if tree_entry_kind(tree, &plugin_root)? != Some(EntryKind::Tree) {
            continue;
        }

        if plugin_root_qualifies(tree, &plugin_root, state)? {
            selection.add_recursive_root(plugin_root);
        }
    }
    Ok(selection)
}

fn plugin_root_qualifies(
    tree: &gix::Tree<'_>,
    plugin_root: &Path,
    state: &mut ManifestState,
) -> Result<bool, Error> {
    if tree_entry_kind(tree, &plugin_root.join(ROOT_MCP_MANIFEST))?.is_some_and(blob_entry_kind) {
        return Ok(true);
    }
    for relative in [CLAUDE_PLUGIN_MANIFEST, CODEX_PLUGIN_MANIFEST] {
        let path = plugin_root.join(relative);
        if !tree_entry_kind(tree, &path)?.is_some_and(blob_entry_kind) {
            continue;
        }
        let manifest: PluginManifest = read_manifest(tree, &path, state)?;
        if manifest.mcp_servers.is_some() {
            return Ok(true);
        }
    }
    Ok(false)
}

fn select_root_mcp_content(
    tree: &gix::Tree<'_>,
    state: &mut ManifestState,
) -> Result<Selection, Error> {
    let manifest_path = PathBuf::from(ROOT_MCP_MANIFEST);
    let manifest: RootMcpManifest = read_manifest(tree, &manifest_path, state)?;
    let mut selection = Selection::default();
    selection.add_file(manifest_path);

    for value in manifest
        .mcp_servers
        .values()
        .flat_map(|server| server.command.iter().chain(&server.args))
    {
        for candidate in inferred_repository_paths(value) {
            match tree_entry_kind(tree, &candidate)? {
                Some(EntryKind::Tree) => selection.add_recursive_root(candidate),
                Some(_) => selection.add_file(candidate),
                None => {},
            }
        }
    }
    for file in ROOT_RUNTIME_FILES {
        let path = PathBuf::from(file);
        if tree_entry_kind(tree, &path)?.is_some_and(blob_entry_kind) {
            selection.add_file(path);
        }
    }
    Ok(selection)
}

fn inferred_repository_paths(value: &str) -> Vec<PathBuf> {
    let value = value.trim();
    if value.is_empty() || value.contains("${") || value.starts_with('$') {
        return Vec::new();
    }
    let candidate = match value.split_once('=') {
        Some((flag, candidate)) if flag.starts_with('-') => candidate,
        _ if value.starts_with('-') => return Vec::new(),
        _ => value,
    }
    .trim_matches(['\'', '"']);
    safe_inferred_path(candidate).into_iter().collect()
}

fn safe_inferred_path(value: &str) -> Option<PathBuf> {
    if value.is_empty() || value.contains("://") {
        return None;
    }
    let path = Path::new(value);
    let mut safe = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {},
            Component::Normal(segment) => safe.push(segment),
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    (!safe.as_os_str().is_empty()).then_some(safe)
}

fn safe_declared_path(value: &str) -> Result<PathBuf, Error> {
    let trimmed = value.trim();
    if looks_absolute_on_any_platform(trimmed) || trimmed.contains('\\') {
        return Err(Error::UnsafeDeclaredPath(value.to_string()));
    }
    let path = if trimmed.is_empty() {
        "."
    } else {
        trimmed
    };
    let mut safe = PathBuf::new();
    for component in Path::new(path).components() {
        match component {
            Component::CurDir => {},
            Component::Normal(segment) => safe.push(segment),
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(Error::UnsafeDeclaredPath(value.to_string()));
            },
        }
    }
    Ok(safe)
}

fn looks_absolute_on_any_platform(value: &str) -> bool {
    value.starts_with(['/', '\\'])
        || value.as_bytes().get(1) == Some(&b':')
            && value
                .as_bytes()
                .first()
                .is_some_and(u8::is_ascii_alphabetic)
}

fn join_declared_paths(base: &Path, relative: &Path) -> Result<PathBuf, Error> {
    let joined = base.join(relative);
    if joined
        .components()
        .all(|component| matches!(component, Component::CurDir | Component::Normal(_)))
    {
        Ok(joined)
    } else {
        Err(Error::UnsafeDeclaredPath(joined.display().to_string()))
    }
}

fn require_tree(tree: &gix::Tree<'_>, path: &Path) -> Result<(), Error> {
    if path.as_os_str().is_empty() || tree_entry_kind(tree, path)? == Some(EntryKind::Tree) {
        Ok(())
    } else {
        Err(Error::InvalidManifest {
            path: path.to_path_buf(),
            message: "declared plugin root is not a committed directory".into(),
        })
    }
}

fn read_manifest<T: for<'de> Deserialize<'de>>(
    tree: &gix::Tree<'_>,
    path: &Path,
    state: &mut ManifestState,
) -> Result<T, Error> {
    let Some(entry) = tree
        .lookup_entry_by_path(path)
        .map_err(|error| Error::Object(error.to_string()))?
    else {
        return Err(Error::InvalidManifest {
            path: path.to_path_buf(),
            message: "manifest is not a committed blob".into(),
        });
    };
    if !blob_entry_kind(entry.mode().kind()) {
        return Err(Error::InvalidManifest {
            path: path.to_path_buf(),
            message: "manifest is not a committed blob".into(),
        });
    }
    let size = entry
        .id()
        .header()
        .map_err(|error| Error::Object(error.to_string()))?
        .size();
    state.record_manifest(path, size)?;
    let blob = entry
        .object()
        .map_err(|error| Error::Object(error.to_string()))?
        .try_into_blob()
        .map_err(|error| Error::Object(error.to_string()))?;
    let data = &blob.data;
    let actual_size = u64::try_from(data.len())
        .map_err(|_| Error::LimitExceeded("manifest size cannot be represented".into()))?;
    if actual_size != size {
        return Err(Error::Object(format!(
            "manifest {} size differs from its Git object header",
            path.display()
        )));
    }
    serde_json::from_slice(data).map_err(|error| Error::InvalidManifest {
        path: path.to_path_buf(),
        message: error.to_string(),
    })
}

fn tree_entry_kind(tree: &gix::Tree<'_>, path: &Path) -> Result<Option<EntryKind>, Error> {
    if path.as_os_str().is_empty() {
        return Ok(Some(EntryKind::Tree));
    }
    tree.lookup_entry_by_path(path)
        .map(|entry| entry.map(|entry| entry.mode().kind()))
        .map_err(|error| Error::Object(error.to_string()))
}

fn blob_entry_kind(kind: EntryKind) -> bool {
    matches!(kind, EntryKind::Blob | EntryKind::BlobExecutable)
}

struct TreeState {
    entries: usize,
    bytes: u64,
    limits: MaterializationLimits,
}

fn write_selected_path(
    tree: &gix::Tree<'_>,
    root: &Path,
    relative: &Path,
    state: &mut TreeState,
) -> Result<(), Error> {
    let depth = relative.components().count();
    if depth > state.limits.max_tree_depth {
        return Err(Error::LimitExceeded(format!(
            "selected path is deeper than {} entries",
            state.limits.max_tree_depth
        )));
    }
    let entry = tree
        .lookup_entry_by_path(relative)
        .map_err(|error| Error::Object(error.to_string()))?
        .ok_or_else(|| {
            Error::Object(format!("selected path is missing: {}", relative.display()))
        })?;
    create_parent_directories(root, relative)?;
    let destination = root.join(relative);
    record_entry(state)?;
    match entry.mode().kind() {
        EntryKind::Tree => {
            fs::create_dir_all(&destination).map_err(|error| Error::io(&destination, error))?;
            let child = entry
                .object()
                .map_err(|error| Error::Object(error.to_string()))?
                .try_into_tree()
                .map_err(|error| Error::Object(error.to_string()))?;
            write_tree(&child, root, &destination, depth, state)
        },
        EntryKind::Blob | EntryKind::BlobExecutable | EntryKind::Link => {
            write_entry_blob(&destination, &entry, state)
        },
        EntryKind::Commit => Ok(()),
    }
}

fn write_tree(
    tree: &gix::Tree<'_>,
    root: &Path,
    directory: &Path,
    depth: usize,
    state: &mut TreeState,
) -> Result<(), Error> {
    if depth > state.limits.max_tree_depth {
        return Err(Error::LimitExceeded(format!(
            "selected tree is deeper than {} entries",
            state.limits.max_tree_depth
        )));
    }
    for entry in tree.iter() {
        let entry = entry.map_err(|error| Error::Object(error.to_string()))?;
        record_entry(state)?;
        let name = gix::path::try_from_bstr(entry.filename())
            .map_err(|error| Error::UnsafeTreeEntry(error.to_string()))?;
        validate_component(&name)?;
        let path = directory.join(name.as_ref());
        ensure_contained(root, &path)?;
        match entry.kind() {
            EntryKind::Tree => {
                fs::create_dir(&path).map_err(|error| Error::io(&path, error))?;
                let child = entry
                    .object()
                    .map_err(|error| Error::Object(error.to_string()))?
                    .try_into_tree()
                    .map_err(|error| Error::Object(error.to_string()))?;
                write_tree(&child, root, &path, depth.saturating_add(1), state)?;
            },
            EntryKind::Blob | EntryKind::BlobExecutable | EntryKind::Link => {
                write_entry_ref_blob(&path, &entry, state)?;
            },
            EntryKind::Commit => {
                // Gitlinks are intentionally omitted: managed repositories never initialize submodules.
            },
        }
    }
    Ok(())
}

fn record_entry(state: &mut TreeState) -> Result<(), Error> {
    state.entries = state.entries.saturating_add(1);
    if state.entries > state.limits.max_files {
        Err(Error::LimitExceeded(format!(
            "selected tree has more than {} entries",
            state.limits.max_files
        )))
    } else {
        Ok(())
    }
}

fn write_entry_blob(
    path: &Path,
    entry: &gix::object::tree::Entry<'_>,
    state: &mut TreeState,
) -> Result<(), Error> {
    let size = entry
        .id()
        .header()
        .map_err(|error| Error::Object(error.to_string()))?
        .size();
    record_blob_size(path, size, state)?;
    let blob = entry
        .object()
        .map_err(|error| Error::Object(error.to_string()))?
        .try_into_blob()
        .map_err(|error| Error::Object(error.to_string()))?;
    write_checked_blob(
        path,
        &blob.data,
        size,
        entry.mode().kind() == EntryKind::BlobExecutable,
    )
}

fn write_entry_ref_blob(
    path: &Path,
    entry: &gix::object::tree::EntryRef<'_, '_>,
    state: &mut TreeState,
) -> Result<(), Error> {
    let size = entry
        .id()
        .header()
        .map_err(|error| Error::Object(error.to_string()))?
        .size();
    record_blob_size(path, size, state)?;
    let blob = entry
        .object()
        .map_err(|error| Error::Object(error.to_string()))?
        .try_into_blob()
        .map_err(|error| Error::Object(error.to_string()))?;
    write_checked_blob(
        path,
        &blob.data,
        size,
        entry.kind() == EntryKind::BlobExecutable,
    )
}

fn record_blob_size(path: &Path, size: u64, state: &mut TreeState) -> Result<(), Error> {
    if size > state.limits.max_blob_bytes {
        return Err(Error::LimitExceeded(format!(
            "blob {} is larger than {} bytes",
            path.display(),
            state.limits.max_blob_bytes
        )));
    }
    state.bytes = state.bytes.saturating_add(size);
    if state.bytes > state.limits.max_total_bytes {
        return Err(Error::LimitExceeded(format!(
            "selected tree is larger than {} bytes",
            state.limits.max_total_bytes
        )));
    }
    Ok(())
}

fn write_checked_blob(
    path: &Path,
    data: &[u8],
    expected_size: u64,
    executable: bool,
) -> Result<(), Error> {
    let actual_size = u64::try_from(data.len())
        .map_err(|_| Error::LimitExceeded("blob size cannot be represented".into()))?;
    if actual_size != expected_size {
        return Err(Error::Object(format!(
            "blob {} size differs from its Git object header",
            path.display()
        )));
    }
    // Symlink blobs are written as inert text because executable is false for EntryKind::Link.
    write_blob(path, data, executable)
}

fn write_blob(path: &Path, data: &[u8], executable: bool) -> Result<(), Error> {
    let mut file = File::options()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| Error::io(path, error))?;
    file.write_all(data)
        .map_err(|error| Error::io(path, error))?;
    #[cfg(unix)]
    if executable {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o755))
            .map_err(|error| Error::io(path, error))?;
    }
    Ok(())
}

fn create_parent_directories(root: &Path, relative: &Path) -> Result<(), Error> {
    let Some(parent) = relative.parent() else {
        return Ok(());
    };
    let path = root.join(parent);
    ensure_contained(root, &path)?;
    fs::create_dir_all(&path).map_err(|error| Error::io(path, error))
}

fn validate_component(path: &Path) -> Result<(), Error> {
    let mut components = path.components();
    let safe =
        matches!(components.next(), Some(Component::Normal(_))) && components.next().is_none();
    if safe {
        return Ok(());
    }
    Err(Error::UnsafeTreeEntry(path.display().to_string()))
}

fn ensure_contained(root: &Path, path: &Path) -> Result<(), Error> {
    if path.starts_with(root) {
        Ok(())
    } else {
        Err(Error::PathEscape)
    }
}

pub(crate) fn canonicalize_published(root: &Path, path: PathBuf) -> Result<PathBuf, Error> {
    let canonical_root = fs::canonicalize(root).map_err(|error| Error::io(root, error))?;
    let canonical_path = fs::canonicalize(&path).map_err(|error| Error::io(&path, error))?;
    if canonical_path.starts_with(&canonical_root) {
        Ok(canonical_path)
    } else {
        Err(Error::PathEscape)
    }
}
