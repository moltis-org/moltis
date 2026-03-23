//! Agent tools for creating, updating, and deleting personal skills at runtime.
//! Skills are written to `<data_dir>/skills/<name>/SKILL.md` (Personal source).

use std::{
    collections::HashSet,
    path::{Component, Path, PathBuf},
};

use {
    async_trait::async_trait,
    moltis_agents::tool_registry::AgentTool,
    serde_json::{Value, json},
};

use crate::error::Error;

const MAX_SIDECAR_FILES_PER_CALL: usize = 16;
const MAX_SIDECAR_FILE_BYTES: usize = 128 * 1024;
const MAX_SIDECAR_TOTAL_BYTES: usize = 512 * 1024;

/// Tool that creates a new personal skill in `<data_dir>/skills/`.
pub struct CreateSkillTool {
    data_dir: PathBuf,
}

impl CreateSkillTool {
    pub fn new(data_dir: PathBuf) -> Self {
        Self { data_dir }
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

        let content = build_skill_md(name, description, body, &allowed_tools);
        write_skill(&skill_dir, &content).await?;

        Ok(json!({
            "created": true,
            "path": skill_dir.display().to_string()
        }))
    }
}

/// Tool that updates an existing personal skill in `<data_dir>/skills/`.
pub struct UpdateSkillTool {
    data_dir: PathBuf,
}

impl UpdateSkillTool {
    pub fn new(data_dir: PathBuf) -> Self {
        Self { data_dir }
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

        let content = build_skill_md(name, description, body, &allowed_tools);
        write_skill(&skill_dir, &content).await?;

        Ok(json!({
            "updated": true,
            "path": skill_dir.display().to_string()
        }))
    }
}

/// Tool that deletes a personal skill from `<data_dir>/skills/`.
pub struct DeleteSkillTool {
    data_dir: PathBuf,
}

impl DeleteSkillTool {
    pub fn new(data_dir: PathBuf) -> Self {
        Self { data_dir }
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

        tokio::fs::remove_dir_all(&skill_dir).await?;

        Ok(json!({ "deleted": true }))
    }
}

/// Tool that writes supplementary text files inside an existing personal skill.
pub struct WriteSkillFilesTool {
    data_dir: PathBuf,
}

impl WriteSkillFilesTool {
    pub fn new(data_dir: PathBuf) -> Self {
        Self { data_dir }
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

        write_sidecar_files(&skill_dir, &validated).await?;
        audit_sidecar_file_write(&self.data_dir, name, &validated);

        Ok(json!({
            "written": true,
            "path": skill_dir.display().to_string(),
            "files_written": validated.len(),
            "files": validated.iter().map(|file| file.relative_path.display().to_string()).collect::<Vec<_>>(),
        }))
    }
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

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_create_skill() {
        let tmp = tempfile::tempdir().unwrap();
        let tool = CreateSkillTool::new(tmp.path().to_path_buf());

        let result = tool
            .execute(json!({
                "name": "my-skill",
                "description": "A test skill",
                "body": "Do something useful."
            }))
            .await
            .unwrap();
        assert!(result["created"].as_bool().unwrap());

        let skill_md = tmp.path().join("skills/my-skill/SKILL.md");
        assert!(skill_md.exists());
        let content = std::fs::read_to_string(&skill_md).unwrap();
        assert!(content.contains("name: my-skill"));
        assert!(content.contains("Do something useful."));
    }

    #[tokio::test]
    async fn test_create_with_allowed_tools() {
        let tmp = tempfile::tempdir().unwrap();
        let tool = CreateSkillTool::new(tmp.path().to_path_buf());

        tool.execute(json!({
            "name": "git-skill",
            "description": "Git helper",
            "body": "Help with git.",
            "allowed_tools": ["Bash(git:*)", "Read"]
        }))
        .await
        .unwrap();

        let content =
            std::fs::read_to_string(tmp.path().join("skills/git-skill/SKILL.md")).unwrap();
        assert!(content.contains("allowed_tools:"));
        assert!(content.contains("Bash(git:*)"));
    }

    #[tokio::test]
    async fn test_create_invalid_name() {
        let tmp = tempfile::tempdir().unwrap();
        let tool = CreateSkillTool::new(tmp.path().to_path_buf());

        let result = tool
            .execute(json!({
                "name": "Bad Name",
                "description": "test",
                "body": "body"
            }))
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_create_duplicate_fails() {
        let tmp = tempfile::tempdir().unwrap();
        let tool = CreateSkillTool::new(tmp.path().to_path_buf());

        tool.execute(json!({
            "name": "my-skill",
            "description": "test",
            "body": "body"
        }))
        .await
        .unwrap();

        let result = tool
            .execute(json!({
                "name": "my-skill",
                "description": "test2",
                "body": "body2"
            }))
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_update_skill() {
        let tmp = tempfile::tempdir().unwrap();
        let create = CreateSkillTool::new(tmp.path().to_path_buf());
        let update = UpdateSkillTool::new(tmp.path().to_path_buf());

        create
            .execute(json!({
                "name": "my-skill",
                "description": "original",
                "body": "original body"
            }))
            .await
            .unwrap();

        update
            .execute(json!({
                "name": "my-skill",
                "description": "updated",
                "body": "new body"
            }))
            .await
            .unwrap();

        let content = std::fs::read_to_string(tmp.path().join("skills/my-skill/SKILL.md")).unwrap();
        assert!(content.contains("description: updated"));
        assert!(content.contains("new body"));
    }

    #[tokio::test]
    async fn test_update_nonexistent_fails() {
        let tmp = tempfile::tempdir().unwrap();
        let tool = UpdateSkillTool::new(tmp.path().to_path_buf());

        let result = tool
            .execute(json!({
                "name": "nope",
                "description": "test",
                "body": "body"
            }))
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_delete_skill() {
        let tmp = tempfile::tempdir().unwrap();
        let create = CreateSkillTool::new(tmp.path().to_path_buf());
        let delete = DeleteSkillTool::new(tmp.path().to_path_buf());

        create
            .execute(json!({
                "name": "my-skill",
                "description": "test",
                "body": "body"
            }))
            .await
            .unwrap();

        let result = delete.execute(json!({ "name": "my-skill" })).await.unwrap();
        assert!(result["deleted"].as_bool().unwrap());
        assert!(!tmp.path().join("skills/my-skill").exists());
    }

    #[tokio::test]
    async fn test_delete_nonexistent_fails() {
        let tmp = tempfile::tempdir().unwrap();
        let tool = DeleteSkillTool::new(tmp.path().to_path_buf());

        let result = tool.execute(json!({ "name": "nope" })).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_write_skill_files_writes_sidecars_and_audits() {
        let tmp = tempfile::tempdir().unwrap();
        let create = CreateSkillTool::new(tmp.path().to_path_buf());
        let write = WriteSkillFilesTool::new(tmp.path().to_path_buf());

        create
            .execute(json!({
                "name": "my-skill",
                "description": "test",
                "body": "body"
            }))
            .await
            .unwrap();

        let result = write
            .execute(json!({
                "name": "my-skill",
                "files": [
                    { "path": "script.sh", "content": "#!/usr/bin/env bash\necho hi\n" },
                    { "path": "templates/prompt.txt", "content": "hello\n" },
                    { "path": "_meta.json", "content": "{\"owner\":\"me\"}\n" }
                ]
            }))
            .await
            .unwrap();

        assert!(result["written"].as_bool().unwrap());
        assert_eq!(result["files_written"].as_u64().unwrap(), 3);
        assert_eq!(
            std::fs::read_to_string(tmp.path().join("skills/my-skill/script.sh")).unwrap(),
            "#!/usr/bin/env bash\necho hi\n"
        );
        assert_eq!(
            std::fs::read_to_string(tmp.path().join("skills/my-skill/templates/prompt.txt"))
                .unwrap(),
            "hello\n"
        );

        let audit_log =
            std::fs::read_to_string(tmp.path().join("logs/security-audit.jsonl")).unwrap();
        assert!(audit_log.contains("\"event\":\"skills.sidecar_files.write\""));
        assert!(audit_log.contains("\"path\":\"script.sh\""));
    }

    #[tokio::test]
    async fn test_write_skill_files_requires_existing_skill() {
        let tmp = tempfile::tempdir().unwrap();
        let write = WriteSkillFilesTool::new(tmp.path().to_path_buf());

        let result = write
            .execute(json!({
                "name": "missing-skill",
                "files": [{ "path": "script.sh", "content": "echo hi\n" }]
            }))
            .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_write_skill_files_rejects_path_traversal() {
        let tmp = tempfile::tempdir().unwrap();
        let create = CreateSkillTool::new(tmp.path().to_path_buf());
        let write = WriteSkillFilesTool::new(tmp.path().to_path_buf());

        create
            .execute(json!({
                "name": "my-skill",
                "description": "test",
                "body": "body"
            }))
            .await
            .unwrap();

        let result = write
            .execute(json!({
                "name": "my-skill",
                "files": [{ "path": "../escape.sh", "content": "echo nope\n" }]
            }))
            .await;

        assert!(result.is_err());
        assert!(!tmp.path().join("skills/escape.sh").exists());
    }

    #[tokio::test]
    async fn test_write_skill_files_rejects_reserved_skill_md() {
        let tmp = tempfile::tempdir().unwrap();
        let create = CreateSkillTool::new(tmp.path().to_path_buf());
        let write = WriteSkillFilesTool::new(tmp.path().to_path_buf());

        create
            .execute(json!({
                "name": "my-skill",
                "description": "test",
                "body": "body"
            }))
            .await
            .unwrap();

        let result = write
            .execute(json!({
                "name": "my-skill",
                "files": [{ "path": "SKILL.md", "content": "nope\n" }]
            }))
            .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_write_skill_files_rejects_hidden_paths() {
        let tmp = tempfile::tempdir().unwrap();
        let create = CreateSkillTool::new(tmp.path().to_path_buf());
        let write = WriteSkillFilesTool::new(tmp.path().to_path_buf());

        create
            .execute(json!({
                "name": "my-skill",
                "description": "test",
                "body": "body"
            }))
            .await
            .unwrap();

        let result = write
            .execute(json!({
                "name": "my-skill",
                "files": [{ "path": ".secret", "content": "nope\n" }]
            }))
            .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_write_skill_files_rejects_duplicate_paths() {
        let tmp = tempfile::tempdir().unwrap();
        let create = CreateSkillTool::new(tmp.path().to_path_buf());
        let write = WriteSkillFilesTool::new(tmp.path().to_path_buf());

        create
            .execute(json!({
                "name": "my-skill",
                "description": "test",
                "body": "body"
            }))
            .await
            .unwrap();

        let result = write
            .execute(json!({
                "name": "my-skill",
                "files": [
                    { "path": "script.sh", "content": "echo one\n" },
                    { "path": "script.sh", "content": "echo two\n" }
                ]
            }))
            .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_write_skill_files_rejects_oversize_file() {
        let tmp = tempfile::tempdir().unwrap();
        let create = CreateSkillTool::new(tmp.path().to_path_buf());
        let write = WriteSkillFilesTool::new(tmp.path().to_path_buf());

        create
            .execute(json!({
                "name": "my-skill",
                "description": "test",
                "body": "body"
            }))
            .await
            .unwrap();

        let result = write
            .execute(json!({
                "name": "my-skill",
                "files": [{
                    "path": "huge.txt",
                    "content": "x".repeat(MAX_SIDECAR_FILE_BYTES + 1)
                }]
            }))
            .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_delete_skill_removes_sidecar_files() {
        let tmp = tempfile::tempdir().unwrap();
        let create = CreateSkillTool::new(tmp.path().to_path_buf());
        let write = WriteSkillFilesTool::new(tmp.path().to_path_buf());
        let delete = DeleteSkillTool::new(tmp.path().to_path_buf());

        create
            .execute(json!({
                "name": "my-skill",
                "description": "test",
                "body": "body"
            }))
            .await
            .unwrap();
        write
            .execute(json!({
                "name": "my-skill",
                "files": [{ "path": "script.sh", "content": "echo hi\n" }]
            }))
            .await
            .unwrap();

        delete.execute(json!({ "name": "my-skill" })).await.unwrap();
        assert!(!tmp.path().join("skills/my-skill").exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_write_skill_files_rejects_symlink_escape() {
        use std::os::unix::fs::symlink;

        let tmp = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let create = CreateSkillTool::new(tmp.path().to_path_buf());
        let write = WriteSkillFilesTool::new(tmp.path().to_path_buf());

        create
            .execute(json!({
                "name": "my-skill",
                "description": "test",
                "body": "body"
            }))
            .await
            .unwrap();

        symlink(outside.path(), tmp.path().join("skills/my-skill/link")).unwrap();

        let result = write
            .execute(json!({
                "name": "my-skill",
                "files": [{ "path": "link/escape.sh", "content": "echo nope\n" }]
            }))
            .await;

        assert!(result.is_err());
        assert!(!outside.path().join("escape.sh").exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_write_skill_files_rejects_symlinked_skill_root() {
        use std::os::unix::fs::symlink;

        let tmp = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();

        // Create a real skill directory outside the skills tree, then symlink
        // the skill name to it.  The confinement check must reject this.
        let skills_dir = tmp.path().join("skills");
        std::fs::create_dir_all(&skills_dir).unwrap();
        let real_dir = outside.path().join("real-skill");
        std::fs::create_dir_all(&real_dir).unwrap();
        std::fs::write(real_dir.join("SKILL.md"), "---\nname: evil\n---\n").unwrap();
        symlink(&real_dir, skills_dir.join("evil")).unwrap();

        let write = WriteSkillFilesTool::new(tmp.path().to_path_buf());
        let result = write
            .execute(json!({
                "name": "evil",
                "files": [{ "path": "payload.sh", "content": "echo pwned\n" }]
            }))
            .await;

        assert!(result.is_err());
        assert!(!real_dir.join("payload.sh").exists());
    }

    #[tokio::test]
    async fn test_write_skill_files_rollback_on_error() {
        let tmp = tempfile::tempdir().unwrap();
        let create = CreateSkillTool::new(tmp.path().to_path_buf());
        let write = WriteSkillFilesTool::new(tmp.path().to_path_buf());

        create
            .execute(json!({
                "name": "my-skill",
                "description": "test",
                "body": "body"
            }))
            .await
            .unwrap();

        // Create a directory where the second file should be written,
        // which will trigger the "target is a directory" error.
        let collision_dir = tmp.path().join("skills/my-skill/collision");
        std::fs::create_dir_all(&collision_dir).unwrap();

        let result = write
            .execute(json!({
                "name": "my-skill",
                "files": [
                    { "path": "first.txt", "content": "ok\n" },
                    { "path": "collision", "content": "boom\n" }
                ]
            }))
            .await;

        assert!(result.is_err());
        // The first file should have been rolled back.
        assert!(
            !tmp.path().join("skills/my-skill/first.txt").exists(),
            "first.txt should be rolled back after batch failure"
        );
    }
}
