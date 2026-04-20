//! Write and patch skill files: sidecar writes and surgical find/replace.

use std::path::PathBuf;

use {
    async_trait::async_trait,
    moltis_agents::tool_registry::AgentTool,
    serde_json::{Value, json},
};

use {
    super::{
        MAX_SIDECAR_FILES_PER_CALL,
        helpers::{
            audit_sidecar_file_write, split_frontmatter_body, update_frontmatter_description,
            validate_sidecar_files, write_sidecar_files,
        },
    },
    crate::{checkpoints::CheckpointManager, error::Error},
};

// ── WriteSkillFilesTool ─────────────────────────────────────

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

// ── PatchSkillTool ──────────────────────────────────────────

/// Maximum number of patches per call.
const MAX_PATCHES_PER_CALL: usize = 10;

/// Tool that applies surgical find/replace patches to an existing personal skill.
///
/// Unlike [`super::crud::UpdateSkillTool`], which requires a full SKILL.md
/// rewrite, this tool applies one or more exact-string replacements, reducing
/// hallucination risk and token cost.
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

        // Reject symlinked skill directories.
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

        let (frontmatter_block, body) = split_frontmatter_body(&raw);

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

        let final_content = if let Some(desc) = new_description {
            let updated_fm = update_frontmatter_description(frontmatter_block, desc);
            format!("{updated_fm}{patched_body}")
        } else {
            format!("{frontmatter_block}{patched_body}")
        };

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
