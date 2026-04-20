//! Agent tools for creating, updating, and deleting personal skills at runtime.
//! Skills are written to `<data_dir>/skills/<name>/SKILL.md` (Personal source).

use std::{
    collections::HashSet,
    path::{Component, Path, PathBuf},
    sync::Arc,
};

use {
    async_trait::async_trait,
    moltis_agents::tool_registry::AgentTool,
    moltis_skills::{discover::SkillDiscoverer, types::SkillSource},
    serde_json::{Value, json},
};

use crate::{checkpoints::CheckpointManager, error::Error};

const MAX_SIDECAR_FILES_PER_CALL: usize = 32;
/// Per-sidecar-subdirectory cap used by the read path's listing. The previous
/// implementation enforced only a single global cap, which meant a
/// `references/` directory containing 32 files would silently swallow the
/// entire quota before `templates/`, `assets/`, or `scripts/` ever got a
/// chance to contribute entries. Enforcing a per-subdir quota guarantees
/// every populated subdirectory shows up in the listing.
const MAX_SIDECAR_FILES_PER_SUBDIR: usize = 8;
const MAX_SIDECAR_FILE_BYTES: usize = 128 * 1024;
const MAX_SIDECAR_TOTAL_BYTES: usize = 512 * 1024;

/// Cap on the size of a single skill body (SKILL.md or a plugin's `.md` file)
/// we'll hand back to the model. This is a defensive ceiling — real skills
/// are typically 5-50 KB — used to prevent a rogue file from filling the
/// agent's context or eating the sidecar size budget by proxy.
const MAX_SKILL_BODY_BYTES: usize = 256 * 1024;

/// Tool that creates a new personal skill in `<data_dir>/skills/`.
pub struct CreateSkillTool {
    data_dir: PathBuf,
    checkpoints: CheckpointManager,
}

impl CreateSkillTool {
    pub fn new(data_dir: PathBuf) -> Self {
        let checkpoints = CheckpointManager::new(data_dir.clone());
        Self {
            data_dir,
            checkpoints,
        }
    }

    fn skills_dir(&self) -> PathBuf {
        self.data_dir.join("skills")
    }
}

#[async_trait]
impl AgentTool for CreateSkillTool {
    fn name(&self) -> &str {
        "create_skill"
    }

    fn description(&self) -> &str {
        "Create a new personal skill. Writes a SKILL.md file to <data_dir>/skills/<name>/. \
         This is persistent workspace storage (not sandbox ~/skills). \
         The skill will be available on the next message automatically."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["name", "description", "body"],
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Skill name (lowercase, hyphens, 1-64 chars)"
                },
                "description": {
                    "type": "string",
                    "description": "Short human-readable description"
                },
                "body": {
                    "type": "string",
                    "description": "Markdown instructions for the skill"
                },
                "allowed_tools": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional list of tools this skill may use"
                }
            }
        })
    }

    async fn execute(&self, params: Value) -> anyhow::Result<Value> {
        let name = params
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::message("missing 'name'"))?;
        let description = params
            .get("description")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::message("missing 'description'"))?;
        let body = params
            .get("body")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::message("missing 'body'"))?;
        let allowed_tools: Vec<String> = params
            .get("allowed_tools")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        if !moltis_skills::parse::validate_name(name) {
            return Err(Error::message(format!(
                "invalid skill name '{name}': must be 1-64 lowercase alphanumeric/hyphen chars"
            ))
            .into());
        }

        let skill_dir = self.skills_dir().join(name);
        if skill_dir.exists() {
            return Err(Error::message(format!(
                "skill '{name}' already exists; use update_skill to modify it"
            ))
            .into());
        }

        let checkpoint = self
            .checkpoints
            .checkpoint_path(&skill_dir, "create_skill")
            .await?;
        let content = build_skill_md(name, description, body, &allowed_tools);
        write_skill(&skill_dir, &content).await?;

        Ok(json!({
            "created": true,
            "path": skill_dir.display().to_string(),
            "checkpointId": checkpoint.id,
        }))
    }
}

/// Tool that updates an existing personal skill in `<data_dir>/skills/`.
pub struct UpdateSkillTool {
    data_dir: PathBuf,
    checkpoints: CheckpointManager,
}

impl UpdateSkillTool {
    pub fn new(data_dir: PathBuf) -> Self {
        let checkpoints = CheckpointManager::new(data_dir.clone());
        Self {
            data_dir,
            checkpoints,
        }
    }

    fn skills_dir(&self) -> PathBuf {
        self.data_dir.join("skills")
    }
}

#[async_trait]
impl AgentTool for UpdateSkillTool {
    fn name(&self) -> &str {
        "update_skill"
    }

    fn description(&self) -> &str {
        "Update an existing personal skill. Overwrites the SKILL.md file."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["name", "description", "body"],
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Skill name to update"
                },
                "description": {
                    "type": "string",
                    "description": "New short description"
                },
                "body": {
                    "type": "string",
                    "description": "New markdown instructions"
                },
                "allowed_tools": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional new list of allowed tools"
                }
            }
        })
    }

    async fn execute(&self, params: Value) -> anyhow::Result<Value> {
        let name = params
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::message("missing 'name'"))?;
        let description = params
            .get("description")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::message("missing 'description'"))?;
        let body = params
            .get("body")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::message("missing 'body'"))?;
        let allowed_tools: Vec<String> = params
            .get("allowed_tools")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        if !moltis_skills::parse::validate_name(name) {
            return Err(Error::message(format!(
                "invalid skill name '{name}': must be 1-64 lowercase alphanumeric/hyphen chars"
            ))
            .into());
        }

        let skill_dir = self.skills_dir().join(name);
        if !skill_dir.exists() {
            return Err(Error::message(format!(
                "skill '{name}' does not exist; use create_skill first"
            ))
            .into());
        }

        let checkpoint = self
            .checkpoints
            .checkpoint_path(&skill_dir, "update_skill")
            .await?;
        let content = build_skill_md(name, description, body, &allowed_tools);
        write_skill(&skill_dir, &content).await?;

        Ok(json!({
            "updated": true,
            "path": skill_dir.display().to_string(),
            "checkpointId": checkpoint.id,
        }))
    }
}

/// Tool that deletes a personal skill from `<data_dir>/skills/`.
pub struct DeleteSkillTool {
    data_dir: PathBuf,
    checkpoints: CheckpointManager,
}

impl DeleteSkillTool {
    pub fn new(data_dir: PathBuf) -> Self {
        let checkpoints = CheckpointManager::new(data_dir.clone());
        Self {
            data_dir,
            checkpoints,
        }
    }

    fn skills_dir(&self) -> PathBuf {
        self.data_dir.join("skills")
    }
}

#[async_trait]
impl AgentTool for DeleteSkillTool {
    fn name(&self) -> &str {
        "delete_skill"
    }

    fn description(&self) -> &str {
        "Delete a personal skill. Removes the full skill directory, including supplementary files."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["name"],
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Skill name to delete"
                }
            }
        })
    }

    async fn execute(&self, params: Value) -> anyhow::Result<Value> {
        let name = params
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::message("missing 'name'"))?;

        if !moltis_skills::parse::validate_name(name) {
            return Err(Error::message(format!("invalid skill name '{name}'")).into());
        }

        let skill_dir = self.skills_dir().join(name);

        // Only allow deleting from the personal skills directory.
        let canonical_base = self
            .skills_dir()
            .canonicalize()
            .unwrap_or_else(|_| self.skills_dir().clone());
        let canonical_target = skill_dir
            .canonicalize()
            .unwrap_or_else(|_| skill_dir.clone());
        if !canonical_target.starts_with(&canonical_base) {
            return Err(Error::message("can only delete personal skills").into());
        }

        if !skill_dir.exists() {
            return Err(Error::message(format!("skill '{name}' not found")).into());
        }

        let checkpoint = self
            .checkpoints
            .checkpoint_path(&skill_dir, "delete_skill")
            .await?;
        tokio::fs::remove_dir_all(&skill_dir).await?;

        Ok(json!({
            "deleted": true,
            "checkpointId": checkpoint.id,
        }))
    }
}

/// Tool that reads a skill's body (and optionally a sidecar file) using the
/// same discoverer that the `<available_skills>` prompt block was built from.
///
/// This is the read-side mirror of [`WriteSkillFilesTool`] and replaces the
/// previous expectation that the model would use an external filesystem MCP
/// server to load `SKILL.md` by absolute path.
pub struct ReadSkillTool {
    discoverer: Arc<dyn SkillDiscoverer>,
}

impl ReadSkillTool {
    /// Construct a `ReadSkillTool` backed by the given discoverer.
    ///
    /// The discoverer should be the same one used to build the
    /// `<available_skills>` prompt block so names listed there always resolve.
    #[must_use]
    pub fn new(discoverer: Arc<dyn SkillDiscoverer>) -> Self {
        Self { discoverer }
    }

    /// Convenience constructor that uses
    /// [`FsSkillDiscoverer::default_paths`](moltis_skills::discover::FsSkillDiscoverer::default_paths).
    ///
    /// Useful for tests and for call sites that already rely on the default
    /// filesystem layout.
    #[must_use]
    pub fn with_default_paths() -> Self {
        use moltis_skills::discover::FsSkillDiscoverer;
        let discoverer = Arc::new(FsSkillDiscoverer::new(FsSkillDiscoverer::default_paths()));
        Self { discoverer }
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
        let file_path = params.get("file_path").and_then(|v| v.as_str());

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

        if let Some(rel) = file_path {
            // Plugin-backed skills can be a single `.md` file rather than
            // a directory containing SKILL.md. Reject sidecar requests on
            // such skills with a clear error — otherwise `read_sidecar`
            // would canonicalise the `.md` file and join the relative
            // path, producing nonsense like `/plugin/demo.md/references/api.md`
            // that would fail with an opaque I/O error.
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

        read_primary(name, meta).await
    }
}

/// Read the main SKILL.md body (or the plugin's `.md` file) plus the list of
/// sidecar files available in `references/`, `templates/`, `assets/`, and
/// `scripts/`. The response also surfaces frontmatter metadata fields
/// (`license`, `homepage`, `compatibility`, `allowed_tools`, `display_name`)
/// so the agent can make informed activation decisions without a second call.
async fn read_primary(
    name: &str,
    meta: &moltis_skills::types::SkillMetadata,
) -> anyhow::Result<Value> {
    let is_plugin = meta.source.as_ref() == Some(&SkillSource::Plugin);

    // Reject a symlinked skill root the same way `read_sidecar` and
    // `write_sidecar_files` do. Without this guard, a symlink like
    // `~/.moltis/skills/malicious -> /etc` would canonicalise silently
    // and the rest of the read path would serve whatever the target
    // resolves to. Defence in depth: the discoverer should not hand us a
    // symlinked root, but the tool enforces the invariant regardless.
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

    // Detect whether a plugin-backed skill is a single `.md` file (rather
    // than a SKILL.md-in-a-directory) via async metadata so the read path
    // stays fully non-blocking — no synchronous `Path::is_file` inside an
    // async function.
    let plugin_as_file = is_plugin
        && tokio::fs::metadata(&meta.path)
            .await
            .map(|m| m.is_file())
            .unwrap_or(false);

    // Plugin skills can be backed by a single `.md` file rather than a
    // directory containing SKILL.md (see `prompt_gen.rs`). Handle both shapes.
    let (loaded_meta, body, linked_files, effective_dir) = if plugin_as_file {
        // Size check *before* reading the whole file so we never buffer a
        // multi-megabyte `.md` into memory only to reject it. Mirrors the
        // defence-in-depth posture `read_sidecar` uses for sidecar files.
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
        // Strip any optional YAML frontmatter so the model sees clean
        // markdown — without this, plugin-backed skills that follow the
        // SKILL.md format return `---\nname: ...\n---` noise in the body.
        // Mirrors what `load_skill_from_path` does for directory-backed
        // skills via `parse::parse_skill`.
        let body = moltis_skills::parse::strip_optional_frontmatter(&raw).to_string();
        let effective_dir = meta
            .path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| meta.path.clone());
        // Plugin .md files don't carry full frontmatter in the discovered
        // metadata — fall back to the discoverer's stub metadata.
        (meta.clone(), body, Vec::new(), effective_dir)
    } else {
        let canonical_skill_dir = tokio::fs::canonicalize(&meta.path).await.map_err(|e| {
            Error::message(format!("skill directory not accessible for '{name}': {e}"))
        })?;

        // Apply the same defensive ceiling to directory-backed SKILL.md
        // files so the plugin and directory paths have symmetric limits.
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

    // Warn on injection patterns (do not block).
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
        None => "unknown",
    };

    // Build the response as a Map directly to avoid the
    // `as_object_mut().expect()` pattern that workspace clippy lints on.
    // Optional metadata fields are only included when set so the agent
    // doesn't wade through empty keys.
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
    // `linked_files` goes last so tools that pretty-print the response
    // surface metadata first.
    response.insert("linked_files".into(), json!(linked_files));

    Ok(Value::Object(response))
}

/// Read a single sidecar file inside a skill directory.
///
/// Supports arbitrary-depth `Component::Normal`-only relative paths (e.g.
/// `references/subdir/deep.md`). Binary files return a structured
/// `{ is_binary: true, bytes }` response instead of failing on UTF-8 decode.
/// If the file doesn't exist, returns a helpful listing of the sidecar files
/// that do exist under this skill.
async fn read_sidecar(name: &str, skill_dir: &Path, rel: &str) -> anyhow::Result<Value> {
    let relative = normalize_relative_skill_file_path(rel)?;

    // Reject a symlinked skill directory to stay consistent with
    // `write_sidecar_files`. Without this, a symlinked skill root
    // (e.g. `~/.moltis/skills/malicious -> /etc`) would pass the later
    // canonical-prefix check because `canonicalize` resolves the symlink
    // and the subsequent `starts_with` comparison succeeds against the
    // resolved target.
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

    // Check existence before canonicalising so we can return a helpful
    // listing instead of an opaque I/O error when the file is missing.
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
    if metadata.len() > MAX_SIDECAR_FILE_BYTES as u64 {
        return Err(Error::message(format!(
            "sidecar file '{}' exceeds maximum size of {MAX_SIDECAR_FILE_BYTES} bytes",
            relative.display()
        ))
        .into());
    }

    // Try UTF-8; fall back to a structured "binary" response on decode
    // failure (mirrors hermes-agent's `skill_view` behavior).
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

/// Sidecar subdirectories that are walked for the primary-read linked-files
/// listing. Re-exported from [`moltis_skills::SIDECAR_SUBDIRS`] so the prompt
/// generator and the read-side walker stay in lockstep — adding a new entry
/// in the skills crate automatically propagates to both the activation
/// instruction and this walker, eliminating a whole class of drift bugs.
const SIDECAR_SUBDIRS: &[&str] = moltis_skills::SIDECAR_SUBDIRS;

/// Entry returned by [`list_skill_sidecar_files`]. Sorted-for-determinism and
/// kept as a typed struct so both the primary read path and the sidecar
/// "file not found" error path can reuse it.
#[derive(Debug, Clone)]
struct SidecarEntry {
    /// Path relative to the skill directory, e.g. `references/api.md`.
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
///
/// Returns a sorted (by relative path) list of entries, capped at
/// [`MAX_SIDECAR_FILES_PER_CALL`]. Directory entries, symlinks, hidden files
/// (dotfiles), and unreadable entries are skipped silently so the listing
/// only shows real in-skill files the agent can actually consume.
async fn list_skill_sidecar_files(skill_dir: &Path) -> crate::Result<Vec<Value>> {
    let mut entries = collect_sidecar_entries(skill_dir).await?;
    // Sort for deterministic output — makes tests stable and the agent's
    // reasoning traces reproducible across runs.
    entries.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    Ok(entries.iter().map(Value::from).collect())
}

async fn collect_sidecar_entries(skill_dir: &Path) -> crate::Result<Vec<SidecarEntry>> {
    let mut out: Vec<SidecarEntry> = Vec::new();

    for sub in SIDECAR_SUBDIRS {
        // Stop early if the global cap is already exhausted so we don't
        // over-report, but *do* enter each subdir as long as it has free
        // budget — the per-subdir cap below guarantees every populated
        // subdirectory gets its fair share even when one dir contains
        // hundreds of files.
        if out.len() >= MAX_SIDECAR_FILES_PER_CALL {
            break;
        }
        let dir = skill_dir.join(sub);
        // Use `tokio::fs::read_dir` directly and treat a missing or
        // unreadable subdirectory as "no entries" — avoids a synchronous
        // `Path::is_dir()` stat inside this async function.
        let mut entries = match tokio::fs::read_dir(&dir).await {
            Ok(e) => e,
            Err(_) => continue,
        };
        let mut this_subdir = 0usize;
        while let Some(entry) = entries.next_entry().await? {
            // Enforce both the per-subdir cap (so `references/` can't
            // swallow the entire listing and hide `templates/` or
            // `scripts/`) and the global cap (so a pathological skill
            // can't return thousands of entries).
            if this_subdir >= MAX_SIDECAR_FILES_PER_SUBDIR
                || out.len() >= MAX_SIDECAR_FILES_PER_CALL
            {
                break;
            }
            // Reject symlinks so the listing only shows real in-skill files.
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

/// Tool that writes supplementary text files inside an existing personal skill.
pub struct WriteSkillFilesTool {
    data_dir: PathBuf,
    checkpoints: CheckpointManager,
}

impl WriteSkillFilesTool {
    pub fn new(data_dir: PathBuf) -> Self {
        let checkpoints = CheckpointManager::new(data_dir.clone());
        Self {
            data_dir,
            checkpoints,
        }
    }

    fn skills_dir(&self) -> PathBuf {
        self.data_dir.join("skills")
    }
}

#[derive(Debug, Clone)]
struct ValidatedSkillFile {
    relative_path: PathBuf,
    content: String,
}

#[async_trait]
impl AgentTool for WriteSkillFilesTool {
    fn name(&self) -> &str {
        "write_skill_files"
    }

    fn description(&self) -> &str {
        "Write supplementary UTF-8 text files inside an existing personal skill directory. \
         This tool is disabled by default and only appears when skills.enable_agent_sidecar_files is enabled."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["name", "files"],
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Existing skill name to update"
                },
                "files": {
                    "type": "array",
                    "description": "Supplementary text files to write inside the skill directory",
                    "minItems": 1,
                    "maxItems": MAX_SIDECAR_FILES_PER_CALL,
                    "items": {
                        "type": "object",
                        "required": ["path", "content"],
                        "properties": {
                            "path": {
                                "type": "string",
                                "description": "Relative path inside the skill directory"
                            },
                            "content": {
                                "type": "string",
                                "description": "UTF-8 text content to write"
                            }
                        }
                    }
                }
            }
        })
    }

    async fn execute(&self, params: Value) -> anyhow::Result<Value> {
        let name = params
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::message("missing 'name'"))?;

        if !moltis_skills::parse::validate_name(name) {
            return Err(Error::message(format!(
                "invalid skill name '{name}': must be 1-64 lowercase alphanumeric/hyphen chars"
            ))
            .into());
        }

        let files = params
            .get("files")
            .and_then(|v| v.as_array())
            .ok_or_else(|| Error::message("missing 'files'"))?;
        let validated = validate_sidecar_files(files)?;

        let skill_dir = self.skills_dir().join(name);
        if !skill_dir.exists() {
            return Err(Error::message(format!(
                "skill '{name}' does not exist; use create_skill first"
            ))
            .into());
        }

        let checkpoint = self
            .checkpoints
            .checkpoint_path(&skill_dir, "write_skill_files")
            .await?;
        write_sidecar_files(&skill_dir, &validated).await?;
        audit_sidecar_file_write(&self.data_dir, name, &validated);

        Ok(json!({
            "written": true,
            "path": skill_dir.display().to_string(),
            "checkpointId": checkpoint.id,
            "files_written": validated.len(),
            "files": validated.iter().map(|file| file.relative_path.display().to_string()).collect::<Vec<_>>(),
        }))
    }
}

/// Tool that applies surgical find/replace patches to an existing personal skill.
///
/// Unlike [`UpdateSkillTool`], which requires a full SKILL.md rewrite, this tool
/// applies one or more exact-string replacements. This reduces hallucination risk
/// and token cost when making incremental fixes.
pub struct PatchSkillTool {
    data_dir: PathBuf,
    checkpoints: CheckpointManager,
}

impl PatchSkillTool {
    pub fn new(data_dir: PathBuf) -> Self {
        let checkpoints = CheckpointManager::new(data_dir.clone());
        Self {
            data_dir,
            checkpoints,
        }
    }

    fn skills_dir(&self) -> PathBuf {
        self.data_dir.join("skills")
    }
}

/// Maximum number of patches per call.
const MAX_PATCHES_PER_CALL: usize = 10;

#[async_trait]
impl AgentTool for PatchSkillTool {
    fn name(&self) -> &str {
        "patch_skill"
    }

    fn description(&self) -> &str {
        "Apply surgical find/replace patches to an existing personal skill's SKILL.md. \
         More efficient than update_skill when fixing a few lines — avoids regenerating \
         the entire body."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["name", "patches"],
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Skill name to patch"
                },
                "patches": {
                    "type": "array",
                    "description": "Ordered list of find/replace operations applied sequentially",
                    "minItems": 1,
                    "maxItems": MAX_PATCHES_PER_CALL,
                    "items": {
                        "type": "object",
                        "required": ["find", "replace"],
                        "properties": {
                            "find": {
                                "type": "string",
                                "description": "Exact string to find in the skill body"
                            },
                            "replace": {
                                "type": "string",
                                "description": "Replacement string"
                            }
                        }
                    }
                },
                "description": {
                    "type": "string",
                    "description": "Optional: update the frontmatter description"
                }
            }
        })
    }

    async fn execute(&self, params: Value) -> anyhow::Result<Value> {
        let name = params
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::message("missing 'name'"))?;
        let patches = params
            .get("patches")
            .and_then(|v| v.as_array())
            .ok_or_else(|| Error::message("missing 'patches'"))?;
        let new_description = params.get("description").and_then(|v| v.as_str());

        if !moltis_skills::parse::validate_name(name) {
            return Err(Error::message(format!(
                "invalid skill name '{name}': must be 1-64 lowercase alphanumeric/hyphen chars"
            ))
            .into());
        }
        if patches.is_empty() {
            return Err(Error::message("at least one patch is required").into());
        }
        if patches.len() > MAX_PATCHES_PER_CALL {
            return Err(Error::message(format!(
                "too many patches: maximum is {MAX_PATCHES_PER_CALL}"
            ))
            .into());
        }

        let skill_dir = self.skills_dir().join(name);
        if !skill_dir.exists() {
            return Err(Error::message(format!(
                "skill '{name}' does not exist; use create_skill first"
            ))
            .into());
        }

        // Reject symlinked skill directories (same check as DeleteSkillTool).
        let canonical_base = self
            .skills_dir()
            .canonicalize()
            .unwrap_or_else(|_| self.skills_dir().clone());
        let canonical_target = skill_dir
            .canonicalize()
            .unwrap_or_else(|_| skill_dir.clone());
        if !canonical_target.starts_with(&canonical_base) {
            return Err(Error::message("can only patch personal skills").into());
        }
        match tokio::fs::symlink_metadata(&skill_dir).await {
            Ok(meta) if meta.file_type().is_symlink() => {
                return Err(Error::message(format!(
                    "skill '{name}' directory must not be a symlink"
                ))
                .into());
            },
            Ok(_) => {},
            Err(e) => {
                return Err(
                    Error::message(format!("skill '{name}' path not accessible: {e}")).into(),
                );
            },
        }

        let skill_md_path = skill_dir.join("SKILL.md");
        let raw = tokio::fs::read_to_string(&skill_md_path)
            .await
            .map_err(|e| Error::message(format!("failed to read skill '{name}': {e}")))?;

        // Parse frontmatter + body.
        let (frontmatter_block, body) = split_frontmatter_body(&raw);

        // Apply patches sequentially to the body.
        let mut patched_body = body.to_string();
        let mut applied = 0usize;
        for (i, patch) in patches.iter().enumerate() {
            let find = patch
                .get("find")
                .and_then(|v| v.as_str())
                .ok_or_else(|| Error::message(format!("patch[{i}]: missing 'find'")))?;
            let replace = patch
                .get("replace")
                .and_then(|v| v.as_str())
                .ok_or_else(|| Error::message(format!("patch[{i}]: missing 'replace'")))?;

            if find.is_empty() {
                return Err(Error::message(format!("patch[{i}]: 'find' must not be empty")).into());
            }
            if !patched_body.contains(find) {
                return Err(Error::message(format!(
                    "patch[{i}]: string not found in skill body: {find:?}"
                ))
                .into());
            }

            patched_body = patched_body.replacen(find, replace, 1);
            applied += 1;
        }

        // Optionally update the frontmatter description.
        let final_content = if let Some(desc) = new_description {
            let updated_fm = update_frontmatter_description(frontmatter_block, desc);
            format!("{updated_fm}{patched_body}")
        } else {
            format!("{frontmatter_block}{patched_body}")
        };

        // Ensure trailing newline.
        let final_content = if final_content.ends_with('\n') {
            final_content
        } else {
            format!("{final_content}\n")
        };

        let checkpoint = self
            .checkpoints
            .checkpoint_path(&skill_dir, "patch_skill")
            .await?;

        tokio::fs::write(&skill_md_path, &final_content).await?;

        // Run injection scan on the result.
        let hits = moltis_skills::safety::scan_skill_body(name, &patched_body);
        let warning = if !hits.is_empty() {
            tracing::warn!(
                skill = %name,
                patterns = ?hits,
                "patched skill body contains potential prompt-injection patterns"
            );
            Some(format!(
                "Warning: patched body matches injection patterns: {}",
                hits.join(", ")
            ))
        } else {
            None
        };

        let mut response = json!({
            "patched": true,
            "patches_applied": applied,
            "checkpointId": checkpoint.id,
        });
        if let Some(warn_msg) = warning
            && let Some(m) = response.as_object_mut()
        {
            m.insert("warning".into(), json!(warn_msg));
        }

        Ok(response)
    }
}

/// Split a SKILL.md file into its frontmatter block (including delimiters and
/// trailing newline) and the body. If there is no frontmatter, frontmatter_block
/// is empty.
fn split_frontmatter_body(raw: &str) -> (&str, &str) {
    if !raw.starts_with("---") {
        return ("", raw);
    }
    // Find the closing `---` after the opening one.
    if let Some(end_idx) = raw[3..].find("\n---") {
        // end_idx is relative to raw[3..], so the closing `---` starts at 3 + end_idx + 1
        let closing_end = 3 + end_idx + 1 + 3; // skip the `---`
        // Include any trailing newline(s) after the closing `---`.
        let after_closing = &raw[closing_end..];
        let body_start = if after_closing.starts_with("\n\n") {
            closing_end + 2
        } else if after_closing.starts_with('\n') {
            closing_end + 1
        } else {
            closing_end
        };
        (&raw[..body_start], &raw[body_start..])
    } else {
        ("", raw)
    }
}

/// Replace the `description: ...` line in a frontmatter block.
///
/// The value is YAML-quoted to prevent indicator characters (`{`, `[`, `>`, `|`,
/// etc.) from being misinterpreted as YAML structure.
fn update_frontmatter_description(frontmatter: &str, new_desc: &str) -> String {
    let mut result = String::with_capacity(frontmatter.len() + new_desc.len());
    let mut found = false;
    for line in frontmatter.lines() {
        if line.starts_with("description:") && !found {
            let quoted = yaml_quote(new_desc);
            result.push_str(&format!("description: {quoted}"));
            found = true;
        } else {
            result.push_str(line);
        }
        result.push('\n');
    }
    // Preserve the blank line between frontmatter and body if the original had one.
    if frontmatter.ends_with("\n\n") && !result.ends_with("\n\n") {
        result.push('\n');
    }
    result
}

/// Quote a string for safe YAML scalar emission. Plain scalars starting with
/// YAML indicator characters (`{`, `[`, `>`, `|`, `*`, `&`, `!`, `%`, `@`, `` ` ``)
/// or containing `: ` would be misinterpreted. Always double-quoting is safe
/// and avoids edge-case surprises.
fn yaml_quote(s: &str) -> String {
    let escaped = s.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

fn build_skill_md(name: &str, description: &str, body: &str, allowed_tools: &[String]) -> String {
    let mut frontmatter = format!("---\nname: {name}\ndescription: {description}\n");
    if !allowed_tools.is_empty() {
        frontmatter.push_str("allowed_tools:\n");
        for tool in allowed_tools {
            frontmatter.push_str(&format!("  - {tool}\n"));
        }
    }
    frontmatter.push_str("---\n\n");
    frontmatter.push_str(body);
    if !body.ends_with('\n') {
        frontmatter.push('\n');
    }
    frontmatter
}

async fn write_skill(skill_dir: &Path, content: &str) -> crate::Result<()> {
    tokio::fs::create_dir_all(skill_dir).await?;
    tokio::fs::write(skill_dir.join("SKILL.md"), content).await?;
    Ok(())
}

fn validate_sidecar_files(files: &[Value]) -> anyhow::Result<Vec<ValidatedSkillFile>> {
    if files.is_empty() {
        return Err(Error::message("at least one file is required").into());
    }
    if files.len() > MAX_SIDECAR_FILES_PER_CALL {
        return Err(Error::message(format!(
            "too many files: maximum is {MAX_SIDECAR_FILES_PER_CALL}"
        ))
        .into());
    }

    let mut total_bytes = 0usize;
    let mut seen_paths = HashSet::new();
    let mut validated = Vec::with_capacity(files.len());

    for file in files {
        let path = file
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::message("each file needs a string 'path'"))?;
        let content = file
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::message("each file needs a string 'content'"))?;

        let relative_path = normalize_relative_skill_file_path(path)?;
        if !seen_paths.insert(relative_path.clone()) {
            return Err(Error::message(format!(
                "duplicate file path '{}'",
                relative_path.display()
            ))
            .into());
        }

        let file_bytes = content.len();
        if file_bytes > MAX_SIDECAR_FILE_BYTES {
            return Err(Error::message(format!(
                "file '{}' exceeds maximum size of {MAX_SIDECAR_FILE_BYTES} bytes",
                relative_path.display()
            ))
            .into());
        }

        total_bytes += file_bytes;
        if total_bytes > MAX_SIDECAR_TOTAL_BYTES {
            return Err(Error::message(format!(
                "total file content exceeds maximum size of {MAX_SIDECAR_TOTAL_BYTES} bytes"
            ))
            .into());
        }

        validated.push(ValidatedSkillFile {
            relative_path,
            content: content.to_string(),
        });
    }

    Ok(validated)
}

fn normalize_relative_skill_file_path(path: &str) -> anyhow::Result<PathBuf> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err(Error::message("file path must not be empty").into());
    }

    let candidate = Path::new(trimmed);
    if candidate.is_absolute() {
        return Err(Error::message("file path must be relative").into());
    }

    let mut normalized = PathBuf::new();
    for component in candidate.components() {
        match component {
            Component::Normal(segment) => {
                let Some(segment_str) = segment.to_str() else {
                    return Err(Error::message("file path must be valid UTF-8").into());
                };
                if segment_str.starts_with('.') {
                    return Err(Error::message(format!(
                        "hidden path components are not allowed: '{trimmed}'"
                    ))
                    .into());
                }
                normalized.push(segment);
            },
            Component::CurDir => {},
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(Error::message("path traversal is not allowed").into());
            },
        }
    }

    let Some(file_name) = normalized.file_name().and_then(|name| name.to_str()) else {
        return Err(Error::message("file path must name a file").into());
    };

    if file_name.eq_ignore_ascii_case("SKILL.md") {
        return Err(
            Error::message("SKILL.md must be managed with create_skill/update_skill").into(),
        );
    }

    Ok(normalized)
}

async fn write_sidecar_files(skill_dir: &Path, files: &[ValidatedSkillFile]) -> crate::Result<()> {
    // Anchor the confinement check to the canonical *skills root*, not the
    // skill directory itself.  If `<data_dir>/skills/<name>` were a symlink
    // pointing outside the tree, using `canonicalize(skill_dir)` as the base
    // would silently accept writes to the symlink target.
    let skills_root = skill_dir
        .parent()
        .ok_or_else(|| Error::message("invalid skill directory"))?;
    let canonical_skills_root = tokio::fs::canonicalize(skills_root).await?;

    // Reject a skill directory that is itself a symlink.
    let skill_meta = tokio::fs::symlink_metadata(skill_dir).await?;
    if skill_meta.file_type().is_symlink() {
        return Err(Error::message("skill directory must not be a symlink"));
    }

    let canonical_base = tokio::fs::canonicalize(skill_dir).await?;
    if !canonical_base.starts_with(&canonical_skills_root) {
        return Err(Error::message("skill directory is outside the skills root"));
    }

    let mut written_paths: Vec<PathBuf> = Vec::new();

    for file in files {
        let target = skill_dir.join(&file.relative_path);
        let parent = target
            .parent()
            .ok_or_else(|| Error::message("invalid file path"))?;

        // Validate path ancestry *before* creating directories so a symlinked
        // intermediate cannot cause out-of-tree directory creation.
        validate_no_symlinks_in_ancestry(skill_dir, &file.relative_path).await?;

        tokio::fs::create_dir_all(parent).await?;

        let canonical_parent = tokio::fs::canonicalize(parent).await?;
        if !canonical_parent.starts_with(&canonical_base) {
            rollback_written_files(&written_paths).await;
            return Err(Error::message(
                "can only write inside the personal skill directory",
            ));
        }

        if let Ok(metadata) = tokio::fs::symlink_metadata(&target).await {
            if metadata.file_type().is_symlink() {
                rollback_written_files(&written_paths).await;
                return Err(Error::message(format!(
                    "refusing to write through symlink '{}'",
                    file.relative_path.display()
                )));
            }
            if metadata.is_dir() {
                rollback_written_files(&written_paths).await;
                return Err(Error::message(format!(
                    "target '{}' is a directory",
                    file.relative_path.display()
                )));
            }
        }

        let Some(file_name) = file
            .relative_path
            .file_name()
            .and_then(|value| value.to_str())
        else {
            rollback_written_files(&written_paths).await;
            return Err(Error::message("invalid file name"));
        };
        let temp_name = format!(".{file_name}.moltis-tmp-{}", uuid::Uuid::new_v4());
        let temp_path = parent.join(temp_name);

        tokio::fs::write(&temp_path, &file.content).await?;
        if let Err(error) = tokio::fs::rename(&temp_path, &target).await {
            let _ = tokio::fs::remove_file(&temp_path).await;
            rollback_written_files(&written_paths).await;
            return Err(error.into());
        }
        written_paths.push(target);
    }

    Ok(())
}

/// Walk from `base` through the existing intermediate components of
/// `relative_path` (excluding the final file component) and reject any
/// symlink.  This prevents `create_dir_all` from following a symlinked
/// intermediate and creating directories outside the skill tree.
async fn validate_no_symlinks_in_ancestry(base: &Path, relative_path: &Path) -> crate::Result<()> {
    let components: Vec<_> = relative_path.components().collect();
    // Only check parent components — the last component is the file itself.
    let parent_components = components.len().saturating_sub(1);
    let mut current = base.to_path_buf();
    for component in components.iter().take(parent_components) {
        if let Component::Normal(segment) = component {
            current.push(segment);
            match tokio::fs::symlink_metadata(&current).await {
                Ok(meta) if meta.file_type().is_symlink() => {
                    return Err(Error::message(format!(
                        "refusing to traverse symlink at '{}'",
                        current.display()
                    )));
                },
                Ok(_) => {},
                // Path doesn't exist yet — safe to stop; create_dir_all will
                // create it as a real directory.
                Err(_) => break,
            }
        }
    }
    Ok(())
}

/// Best-effort removal of already-written files when a batch fails mid-way.
async fn rollback_written_files(paths: &[PathBuf]) {
    for path in paths.iter().rev() {
        let _ = tokio::fs::remove_file(path).await;
    }
}

fn audit_sidecar_file_write(data_dir: &Path, skill_name: &str, files: &[ValidatedSkillFile]) {
    let dir = data_dir.join("logs");
    let path = dir.join("security-audit.jsonl");
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let line = serde_json::json!({
        "ts": now_ms,
        "event": "skills.sidecar_files.write",
        "details": {
            "skill": skill_name,
            "files": files.iter().map(|file| {
                serde_json::json!({
                    "path": file.relative_path.display().to_string(),
                    "bytes": file.content.len(),
                })
            }).collect::<Vec<_>>(),
        },
    })
    .to_string();

    if let Err(err) = (|| -> std::io::Result<()> {
        std::fs::create_dir_all(&dir)?;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        use std::io::Write as _;
        writeln!(file, "{line}")?;
        Ok(())
    })() {
        tracing::warn!(
            error = %err,
            skill = skill_name,
            "failed to write sidecar-file audit entry"
        );
    }
}

#[cfg(test)]
mod tests;
