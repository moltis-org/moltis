//! Skill reading: primary body, sidecar files, and sidecar listing.

use std::{path::Path, sync::Arc};

use {
    async_trait::async_trait,
    moltis_agents::tool_registry::AgentTool,
    moltis_skills::{discover::SkillDiscoverer, types::SkillSource},
    serde_json::{Value, json},
};

use {
    super::{
        MAX_SIDECAR_FILES_PER_CALL, MAX_SIDECAR_FILES_PER_SUBDIR, MAX_SKILL_BODY_BYTES,
        helpers::normalize_relative_skill_file_path,
    },
    crate::error::Error,
};

/// Sidecar subdirectories walked for the primary-read linked-files listing.
const SIDECAR_SUBDIRS: &[&str] = moltis_skills::SIDECAR_SUBDIRS;

// ── ReadSkillTool ───────────────────────────────────────────

/// Tool that reads a skill's body (and optionally a sidecar file) using the
/// same discoverer that the `<available_skills>` prompt block was built from.
///
/// This is the read-side mirror of [`super::write_ops::WriteSkillFilesTool`]
/// and replaces the previous expectation that the model would use an external
/// filesystem MCP server to load `SKILL.md` by absolute path.
pub struct ReadSkillTool {
    discoverer: Arc<dyn SkillDiscoverer>,
    #[cfg(feature = "bundled-skills")]
    bundled_store: Option<Arc<moltis_skills::bundled::BundledSkillStore>>,
}

impl ReadSkillTool {
    /// Construct a `ReadSkillTool` backed by the given discoverer.
    #[must_use]
    pub fn new(discoverer: Arc<dyn SkillDiscoverer>) -> Self {
        Self {
            discoverer,
            #[cfg(feature = "bundled-skills")]
            bundled_store: None,
        }
    }

    /// Construct a `ReadSkillTool` with bundled skill support.
    #[cfg(feature = "bundled-skills")]
    #[must_use]
    pub fn with_bundled(
        discoverer: Arc<dyn SkillDiscoverer>,
        bundled_store: Arc<moltis_skills::bundled::BundledSkillStore>,
    ) -> Self {
        Self {
            discoverer,
            bundled_store: Some(bundled_store),
        }
    }

    /// Convenience constructor that uses default filesystem paths.
    #[must_use]
    pub fn with_default_paths() -> Self {
        use moltis_skills::discover::FsSkillDiscoverer;
        let discoverer = Arc::new(FsSkillDiscoverer::new(FsSkillDiscoverer::default_paths()));
        Self {
            discoverer,
            #[cfg(feature = "bundled-skills")]
            bundled_store: None,
        }
    }
}

#[async_trait]
impl AgentTool for ReadSkillTool {
    fn name(&self) -> &str {
        "read_skill"
    }

    fn description(&self) -> &str {
        "Load a skill's full content or access its linked files (references, \
         templates, assets, scripts). The primary call (with just 'name') \
         returns the SKILL.md body plus a list of available sidecar files \
         under references/, templates/, assets/, and scripts/. To read those, \
         call again with the file_path argument \
         (e.g. file_path=\"references/api.md\"). Nested file_paths such as \
         \"references/subdir/deep.md\" are supported even if the listing only \
         shows the first level. Binary files return a structured response \
         with { is_binary: true, bytes }. Use the skill names listed in the \
         <available_skills> system-prompt block."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["name"],
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Skill name (use the names from <available_skills>)"
                },
                "file_path": {
                    "type": "string",
                    "description": "Optional: relative path to a sidecar file inside the skill directory (e.g. 'references/api.md'). Omit to read the main SKILL.md body."
                }
            }
        })
    }

    async fn execute(&self, params: Value) -> anyhow::Result<Value> {
        let name = params
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::message("missing 'name'"))?;
        // Treat empty string the same as absent — models often send ""
        // instead of omitting the field.
        let file_path = params
            .get("file_path")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty());

        let skills = self.discoverer.discover().await?;
        let meta = skills.iter().find(|s| s.name == name).ok_or_else(|| {
            let available: Vec<&str> = skills.iter().map(|s| s.name.as_str()).collect();
            let hint = if available.is_empty() {
                "no skills are currently available".to_string()
            } else {
                format!("available skills: {}", available.join(", "))
            };
            Error::message(format!(
                "skill '{name}' not found ({hint}). Use one of the names listed \
                 in <available_skills>."
            ))
        })?;

        // Check requirements and provide install instructions if missing.
        let install_note = if file_path.is_none() {
            auto_install_requirements(meta).await
        } else {
            None
        };

        // Bundled skills are served from the embedded store, not the filesystem.
        #[cfg(feature = "bundled-skills")]
        if meta.source.as_ref() == Some(&SkillSource::Bundled)
            && let Some(ref store) = self.bundled_store
        {
            let mut result = read_bundled(name, meta, store, file_path)?;
            inject_install_note(&mut result, &install_note);
            return Ok(result);
        }

        if let Some(rel) = file_path {
            if meta.source.as_ref() == Some(&SkillSource::Plugin)
                && tokio::fs::metadata(&meta.path)
                    .await
                    .map(|m| m.is_file())
                    .unwrap_or(false)
            {
                return Err(Error::message(format!(
                    "plugin skill '{name}' is a single .md file and has no \
                     sidecar directory; omit file_path to read the body"
                ))
                .into());
            }
            return read_sidecar(name, &meta.path, rel).await;
        }

        let mut result = read_primary(name, meta).await?;
        inject_install_note(&mut result, &install_note);
        Ok(result)
    }
}

// ── read_primary ────────────────────────────────────────────

/// Read the main SKILL.md body (or the plugin's `.md` file) plus the list of
/// sidecar files available in `references/`, `templates/`, `assets/`, and
/// `scripts/`.
async fn read_primary(
    name: &str,
    meta: &moltis_skills::types::SkillMetadata,
) -> anyhow::Result<Value> {
    let is_plugin = meta.source.as_ref() == Some(&SkillSource::Plugin);

    match tokio::fs::symlink_metadata(&meta.path).await {
        Ok(m) if m.file_type().is_symlink() => {
            return Err(
                Error::message(format!("skill '{name}' directory must not be a symlink")).into(),
            );
        },
        Ok(_) => {},
        Err(e) => {
            return Err(Error::message(format!("skill '{name}' path not accessible: {e}")).into());
        },
    }

    let plugin_as_file = is_plugin
        && tokio::fs::metadata(&meta.path)
            .await
            .map(|m| m.is_file())
            .unwrap_or(false);

    let (loaded_meta, body, linked_files, effective_dir) = if plugin_as_file {
        let file_meta = tokio::fs::metadata(&meta.path).await.map_err(|e| {
            Error::message(format!(
                "failed to stat plugin skill '{name}' at {}: {e}",
                meta.path.display()
            ))
        })?;
        if file_meta.len() > MAX_SKILL_BODY_BYTES as u64 {
            return Err(Error::message(format!(
                "plugin skill '{name}' body exceeds maximum size of \
                 {MAX_SKILL_BODY_BYTES} bytes ({} bytes on disk)",
                file_meta.len()
            ))
            .into());
        }
        let raw = tokio::fs::read_to_string(&meta.path).await.map_err(|e| {
            Error::message(format!(
                "failed to read plugin skill '{name}' at {}: {e}",
                meta.path.display()
            ))
        })?;
        let body = moltis_skills::parse::strip_optional_frontmatter(&raw).to_string();
        let effective_dir = meta
            .path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| meta.path.clone());
        (meta.clone(), body, Vec::new(), effective_dir)
    } else {
        let canonical_skill_dir = tokio::fs::canonicalize(&meta.path).await.map_err(|e| {
            Error::message(format!("skill directory not accessible for '{name}': {e}"))
        })?;

        let skill_md_path = canonical_skill_dir.join("SKILL.md");
        if let Ok(m) = tokio::fs::metadata(&skill_md_path).await
            && m.len() > MAX_SKILL_BODY_BYTES as u64
        {
            return Err(Error::message(format!(
                "skill '{name}' SKILL.md exceeds maximum size of \
                 {MAX_SKILL_BODY_BYTES} bytes ({} bytes on disk)",
                m.len()
            ))
            .into());
        }

        let content = moltis_skills::registry::load_skill_from_path(&canonical_skill_dir)
            .await
            .map_err(|e| Error::message(format!("failed to load skill '{name}': {e}")))?;
        let linked = list_skill_sidecar_files(&canonical_skill_dir).await?;
        (content.metadata, content.body, linked, canonical_skill_dir)
    };

    let hits = moltis_skills::safety::scan_skill_body(name, &body);
    if !hits.is_empty() {
        tracing::warn!(
            skill = %name,
            patterns = ?hits,
            "skill body contains potential prompt-injection patterns"
        );
    }

    let source_label = match meta.source.as_ref() {
        Some(SkillSource::Project) => "project",
        Some(SkillSource::Personal) => "personal",
        Some(SkillSource::Plugin) => "plugin",
        Some(SkillSource::Registry) => "registry",
        Some(SkillSource::Bundled) => "bundled",
        None => "unknown",
    };

    let mut response = serde_json::Map::new();
    response.insert("name".into(), json!(name));
    response.insert("description".into(), json!(loaded_meta.description));
    response.insert("source".into(), json!(source_label));
    response.insert("body".into(), json!(body));
    response.insert("bytes".into(), json!(body.len()));
    response.insert(
        "skill_dir_name".into(),
        json!(
            effective_dir
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
        ),
    );

    if let Some(display_name) = &loaded_meta.display_name {
        response.insert("display_name".into(), json!(display_name));
    }
    if let Some(license) = &loaded_meta.license {
        response.insert("license".into(), json!(license));
    }
    if let Some(homepage) = &loaded_meta.homepage {
        response.insert("homepage".into(), json!(homepage));
    }
    if let Some(compatibility) = &loaded_meta.compatibility {
        response.insert("compatibility".into(), json!(compatibility));
    }
    if !loaded_meta.allowed_tools.is_empty() {
        response.insert("allowed_tools".into(), json!(loaded_meta.allowed_tools));
    }
    if !linked_files.is_empty() {
        response.insert(
            "usage_hint".into(),
            json!(
                "To view a linked file, call read_skill again with file_path \
                 set to one of the paths in linked_files (e.g. \
                 file_path=\"references/api.md\"). Nested paths inside those \
                 directories are also supported."
            ),
        );
    }
    response.insert("linked_files".into(), json!(linked_files));

    Ok(Value::Object(response))
}

// ── read_sidecar ────────────────────────────────────────────

/// Read a single sidecar file inside a skill directory.
async fn read_sidecar(name: &str, skill_dir: &Path, rel: &str) -> anyhow::Result<Value> {
    let relative = normalize_relative_skill_file_path(rel)?;

    match tokio::fs::symlink_metadata(skill_dir).await {
        Ok(meta) if meta.file_type().is_symlink() => {
            return Err(
                Error::message(format!("skill '{name}' directory must not be a symlink")).into(),
            );
        },
        Ok(_) => {},
        Err(e) => {
            return Err(Error::message(format!(
                "skill directory not accessible for '{name}': {e}"
            ))
            .into());
        },
    }

    let canonical_skill_dir = tokio::fs::canonicalize(skill_dir)
        .await
        .map_err(|e| Error::message(format!("skill directory not accessible for '{name}': {e}")))?;

    let target = canonical_skill_dir.join(&relative);

    match tokio::fs::symlink_metadata(&target).await {
        Ok(_) => {},
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            let available = list_skill_sidecar_files(&canonical_skill_dir).await?;
            return Err(Error::message(format!(
                "sidecar file '{}' not found in skill '{name}'. \
                 Available sidecar files: {}",
                relative.display(),
                if available.is_empty() {
                    "(none)".to_string()
                } else {
                    available
                        .iter()
                        .filter_map(|v| v.get("path").and_then(|p| p.as_str()))
                        .collect::<Vec<_>>()
                        .join(", ")
                }
            ))
            .into());
        },
        Err(e) => {
            return Err(Error::message(format!(
                "sidecar file '{}' not accessible: {e}",
                relative.display()
            ))
            .into());
        },
    }

    let canonical_target = tokio::fs::canonicalize(&target).await.map_err(|e| {
        Error::message(format!(
            "sidecar file '{}' not accessible: {e}",
            relative.display()
        ))
    })?;

    if !canonical_target.starts_with(&canonical_skill_dir) {
        return Err(Error::message(format!(
            "sidecar file '{}' is outside the skill directory",
            relative.display()
        ))
        .into());
    }

    let metadata = tokio::fs::metadata(&canonical_target).await?;
    if !metadata.is_file() {
        return Err(Error::message(format!(
            "sidecar path '{}' is not a regular file",
            relative.display()
        ))
        .into());
    }
    if metadata.len() > super::MAX_SIDECAR_FILE_BYTES as u64 {
        return Err(Error::message(format!(
            "sidecar file '{}' exceeds maximum size of {} bytes",
            relative.display(),
            super::MAX_SIDECAR_FILE_BYTES
        ))
        .into());
    }

    let raw = tokio::fs::read(&canonical_target).await.map_err(|e| {
        Error::message(format!(
            "failed to read sidecar file '{}': {e}",
            relative.display()
        ))
    })?;

    match std::str::from_utf8(&raw) {
        Ok(text) => Ok(json!({
            "name": name,
            "file_path": relative.display().to_string(),
            "bytes": metadata.len(),
            "content": text,
            "is_binary": false,
        })),
        Err(_) => {
            let file_type = canonical_target
                .extension()
                .and_then(|e| e.to_str())
                .map(|s| format!(".{s}"))
                .unwrap_or_default();
            Ok(json!({
                "name": name,
                "file_path": relative.display().to_string(),
                "bytes": metadata.len(),
                "is_binary": true,
                "file_type": file_type,
                "note": format!(
                    "Binary file ({} bytes). Contents omitted — the model \
                     cannot consume binary data directly.",
                    metadata.len()
                ),
            }))
        },
    }
}

// ── Sidecar listing ─────────────────────────────────────────

#[derive(Debug, Clone)]
struct SidecarEntry {
    relative_path: String,
    bytes: u64,
}

impl From<&SidecarEntry> for Value {
    fn from(entry: &SidecarEntry) -> Self {
        json!({
            "path": entry.relative_path,
            "bytes": entry.bytes,
        })
    }
}

/// One-level-deep walk of `<skill_dir>/{references,templates,assets,scripts}`.
pub(super) async fn list_skill_sidecar_files(skill_dir: &Path) -> crate::Result<Vec<Value>> {
    let mut entries = collect_sidecar_entries(skill_dir).await?;
    entries.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    Ok(entries.iter().map(Value::from).collect())
}

async fn collect_sidecar_entries(skill_dir: &Path) -> crate::Result<Vec<SidecarEntry>> {
    let mut out: Vec<SidecarEntry> = Vec::new();

    for sub in SIDECAR_SUBDIRS {
        if out.len() >= MAX_SIDECAR_FILES_PER_CALL {
            break;
        }
        let dir = skill_dir.join(sub);
        let mut entries = match tokio::fs::read_dir(&dir).await {
            Ok(e) => e,
            Err(_) => continue,
        };
        let mut this_subdir = 0usize;
        while let Some(entry) = entries.next_entry().await? {
            if this_subdir >= MAX_SIDECAR_FILES_PER_SUBDIR
                || out.len() >= MAX_SIDECAR_FILES_PER_CALL
            {
                break;
            }
            let file_type = match entry.file_type().await {
                Ok(ft) => ft,
                Err(_) => continue,
            };
            if !file_type.is_file() {
                continue;
            }
            let meta = match entry.metadata().await {
                Ok(m) => m,
                Err(_) => continue,
            };
            let file_name = match entry.file_name().into_string() {
                Ok(name) => name,
                Err(_) => continue,
            };
            if file_name.starts_with('.') {
                continue;
            }
            out.push(SidecarEntry {
                relative_path: format!("{sub}/{file_name}"),
                bytes: meta.len(),
            });
            this_subdir += 1;
        }
    }

    Ok(out)
}

// ── Bundled skill reading ──────────────────────────────────────────────────

/// Read a bundled skill from the embedded store (no filesystem I/O).
#[cfg(feature = "bundled-skills")]
fn read_bundled(
    name: &str,
    meta: &moltis_skills::types::SkillMetadata,
    store: &moltis_skills::bundled::BundledSkillStore,
    file_path: Option<&str>,
) -> anyhow::Result<Value> {
    if let Some(rel) = file_path {
        let rel_normalized = normalize_relative_skill_file_path(rel)
            .map_err(|e| Error::message(format!("invalid file_path: {e}")))?;
        let rel = rel_normalized.to_str().unwrap_or(rel);

        return match store.read_sidecar(name, rel) {
            Some((bytes, true)) => {
                let text = String::from_utf8_lossy(&bytes);
                Ok(json!({
                    "name": name,
                    "file_path": rel,
                    "bytes": bytes.len(),
                    "content": text,
                    "is_binary": false,
                }))
            },
            Some((bytes, false)) => Ok(json!({
                "name": name,
                "file_path": rel,
                "bytes": bytes.len(),
                "is_binary": true,
                "note": format!("Binary file ({} bytes). Contents omitted.", bytes.len()),
            })),
            None => {
                let available = store.list_sidecars(name);
                let hint = if available.is_empty() {
                    "(none)".to_string()
                } else {
                    available
                        .iter()
                        .map(|(p, _)| p.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                };
                Err(Error::message(format!(
                    "sidecar file '{rel}' not found in bundled skill '{name}'. \
                     Available sidecar files: {hint}"
                ))
                .into())
            },
        };
    }

    let body = store
        .read_skill(name)
        .ok_or_else(|| Error::message(format!("bundled skill '{name}' body not readable")))?;

    let linked: Vec<Value> = store
        .list_sidecars(name)
        .into_iter()
        .map(|(path, bytes)| json!({"path": path, "bytes": bytes}))
        .collect();

    // Materialize sidecar files to disk so scripts can be executed.
    let skill_dir = store.materialize_sidecars(name);

    let mut response = serde_json::Map::new();
    response.insert("name".into(), json!(name));
    response.insert("description".into(), json!(meta.description));
    response.insert("source".into(), json!("bundled"));
    if let Some(ref cat) = meta.category {
        response.insert("category".into(), json!(cat));
    }
    response.insert("body".into(), json!(body));
    response.insert("bytes".into(), json!(body.len()));
    if let Some(ref dir) = skill_dir {
        response.insert("skill_dir".into(), json!(dir.display().to_string()));
    }

    if let Some(display_name) = &meta.display_name {
        response.insert("display_name".into(), json!(display_name));
    }
    if let Some(license) = &meta.license {
        response.insert("license".into(), json!(license));
    }
    if let Some(homepage) = &meta.homepage {
        response.insert("homepage".into(), json!(homepage));
    }
    if let Some(origin) = &meta.origin {
        response.insert("origin".into(), json!(origin));
    }
    if !meta.allowed_tools.is_empty() {
        response.insert("allowed_tools".into(), json!(meta.allowed_tools));
    }
    if !linked.is_empty() {
        let has_scripts = skill_dir.is_some()
            && linked.iter().any(|v| {
                v.get("path")
                    .and_then(|p| p.as_str())
                    .is_some_and(|p| p.starts_with("scripts/"))
            });
        let hint = if has_scripts {
            "This skill includes executable scripts. Use the `skill_dir` path \
             to resolve script references in the body. To view a linked file, \
             call read_skill again with file_path set to one of the paths in \
             linked_files."
        } else {
            "To view a linked file, call read_skill again with file_path \
             set to one of the paths in linked_files."
        };
        response.insert("usage_hint".into(), json!(hint));
    }
    response.insert("linked_files".into(), json!(linked));

    Ok(Value::Object(response))
}

// ── Install requirements ───────────────────────────────────────────────────

/// Check skill requirements and return install instructions if binaries are
/// missing. Does NOT run the install — the agent should run the commands via
/// `exec` so they execute in the correct environment (sandbox or host).
async fn auto_install_requirements(meta: &moltis_skills::types::SkillMetadata) -> Option<String> {
    use moltis_skills::requirements::{check_requirements, install_command_preview};

    let elig = check_requirements(meta);
    if elig.eligible || elig.install_options.is_empty() {
        return None;
    }

    let commands: Vec<String> = elig
        .install_options
        .iter()
        .filter_map(|spec| install_command_preview(spec).ok())
        .collect();

    if commands.is_empty() {
        return Some(format!(
            "Missing binaries: {}. No install instructions available.",
            elig.missing_bins.join(", ")
        ));
    }

    Some(format!(
        "Missing binaries: {}. Install with: {}",
        elig.missing_bins.join(", "),
        commands.join(" OR ")
    ))
}

/// Inject an install note into a skill read response.
fn inject_install_note(result: &mut Value, note: &Option<String>) {
    if let Some(msg) = note
        && let Some(obj) = result.as_object_mut()
    {
        obj.insert("install_note".into(), json!(msg));
    }
}
