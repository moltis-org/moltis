//! Agent tools for creating, updating, and deleting personal skills at runtime.
//! Skills are written to `<data_dir>/skills/<name>/SKILL.md` (Personal source).

use std::path::{Path, PathBuf};

use {
    anyhow::{Result, bail},
    async_trait::async_trait,
    moltis_agents::tool_registry::AgentTool,
    serde_json::{Value, json},
};

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
        "Create a new personal skill. Writes a SKILL.md file to ~/skills/<name>/. \
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

    async fn execute(&self, params: Value) -> Result<Value> {
        let name = params
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing 'name'"))?;
        let description = params
            .get("description")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing 'description'"))?;
        let body = params
            .get("body")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing 'body'"))?;
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
            bail!("invalid skill name '{name}': must be 1-64 lowercase alphanumeric/hyphen chars");
        }

        let skill_dir = self.skills_dir().join(name);
        if skill_dir.exists() {
            bail!("skill '{name}' already exists; use update_skill to modify it");
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

    async fn execute(&self, params: Value) -> Result<Value> {
        let name = params
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing 'name'"))?;
        let description = params
            .get("description")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing 'description'"))?;
        let body = params
            .get("body")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing 'body'"))?;
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
            bail!("invalid skill name '{name}': must be 1-64 lowercase alphanumeric/hyphen chars");
        }

        let skill_dir = self.skills_dir().join(name);
        if !skill_dir.exists() {
            bail!("skill '{name}' does not exist; use create_skill first");
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
        "Delete a personal skill. Only works for skills in ~/skills/."
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

    async fn execute(&self, params: Value) -> Result<Value> {
        let name = params
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing 'name'"))?;

        if !moltis_skills::parse::validate_name(name) {
            bail!("invalid skill name '{name}'");
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
            bail!("can only delete personal skills");
        }

        if !skill_dir.exists() {
            bail!("skill '{name}' not found");
        }

        tokio::fs::remove_dir_all(&skill_dir).await?;

        Ok(json!({ "deleted": true }))
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

async fn write_skill(skill_dir: &Path, content: &str) -> Result<()> {
    tokio::fs::create_dir_all(skill_dir).await?;
    tokio::fs::write(skill_dir.join("SKILL.md"), content).await?;
    Ok(())
}

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
}
