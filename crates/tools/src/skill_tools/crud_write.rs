#![allow(clippy::unwrap_used, clippy::expect_used)]
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
    assert!(result["checkpointId"].as_str().is_some());

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

    let content = std::fs::read_to_string(tmp.path().join("skills/git-skill/SKILL.md")).unwrap();
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

    let result = update
        .execute(json!({
            "name": "my-skill",
            "description": "updated",
            "body": "new body"
        }))
        .await
        .unwrap();
    assert!(result["checkpointId"].as_str().is_some());

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
    assert!(result["checkpointId"].as_str().is_some());
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
    assert!(result["checkpointId"].as_str().is_some());
    assert_eq!(result["files_written"].as_u64().unwrap(), 3);
    assert_eq!(
        std::fs::read_to_string(tmp.path().join("skills/my-skill/script.sh")).unwrap(),
        "#!/usr/bin/env bash\necho hi\n"
    );
    assert_eq!(
        std::fs::read_to_string(tmp.path().join("skills/my-skill/templates/prompt.txt")).unwrap(),
        "hello\n"
    );

    let audit_log = std::fs::read_to_string(tmp.path().join("logs/security-audit.jsonl")).unwrap();
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

#[tokio::test]
async fn test_update_skill_checkpoint_can_restore_previous_state() {
    let tmp = tempfile::tempdir().unwrap();
    let create = CreateSkillTool::new(tmp.path().to_path_buf());
    let update = UpdateSkillTool::new(tmp.path().to_path_buf());
    let checkpoints = CheckpointManager::new(tmp.path().to_path_buf());

    create
        .execute(json!({
            "name": "my-skill",
            "description": "original",
            "body": "original body"
        }))
        .await
        .unwrap();

    let result = update
        .execute(json!({
            "name": "my-skill",
            "description": "updated",
            "body": "new body"
        }))
        .await
        .unwrap();
    let checkpoint_id = result["checkpointId"].as_str().unwrap();

    checkpoints.restore(checkpoint_id).await.unwrap();

    let content = std::fs::read_to_string(tmp.path().join("skills/my-skill/SKILL.md")).unwrap();
    assert!(content.contains("description: original"));
    assert!(content.contains("original body"));
}

#[tokio::test]
async fn test_delete_skill_checkpoint_can_restore_deleted_skill() {
    let tmp = tempfile::tempdir().unwrap();
    let create = CreateSkillTool::new(tmp.path().to_path_buf());
    let delete = DeleteSkillTool::new(tmp.path().to_path_buf());
    let checkpoints = CheckpointManager::new(tmp.path().to_path_buf());

    create
        .execute(json!({
            "name": "my-skill",
            "description": "test",
            "body": "body"
        }))
        .await
        .unwrap();

    let result = delete.execute(json!({ "name": "my-skill" })).await.unwrap();
    let checkpoint_id = result["checkpointId"].as_str().unwrap();

    checkpoints.restore(checkpoint_id).await.unwrap();

    assert!(tmp.path().join("skills/my-skill/SKILL.md").exists());
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

// ── PatchSkillTool tests ────────────────────────────────────────────────

#[tokio::test]
async fn test_patch_skill_single_patch() {
    let tmp = tempfile::tempdir().unwrap();
    let create = CreateSkillTool::new(tmp.path().to_path_buf());
    let patch = PatchSkillTool::new(tmp.path().to_path_buf());

    create
        .execute(json!({
            "name": "my-skill",
            "description": "A test skill",
            "body": "Step 1: Do X\nStep 2: Do Y\nStep 3: Do Z"
        }))
        .await
        .unwrap();

    let result = patch
        .execute(json!({
            "name": "my-skill",
            "patches": [
                { "find": "Do Y", "replace": "Do B" }
            ]
        }))
        .await
        .unwrap();

    assert!(result["patched"].as_bool().unwrap());
    assert_eq!(result["patches_applied"].as_u64().unwrap(), 1);

    let content = std::fs::read_to_string(tmp.path().join("skills/my-skill/SKILL.md")).unwrap();
    assert!(content.contains("Do B"));
    assert!(content.contains("Do X"));
    assert!(content.contains("Do Z"));
    // Frontmatter should be preserved.
    assert!(content.contains("name: my-skill"));
    assert!(content.contains("description: A test skill"));
}

#[tokio::test]
async fn test_patch_skill_multiple_patches_in_order() {
    let tmp = tempfile::tempdir().unwrap();
    let create = CreateSkillTool::new(tmp.path().to_path_buf());
    let patch = PatchSkillTool::new(tmp.path().to_path_buf());

    create
        .execute(json!({
            "name": "my-skill",
            "description": "test",
            "body": "AAA BBB CCC"
        }))
        .await
        .unwrap();

    let result = patch
        .execute(json!({
            "name": "my-skill",
            "patches": [
                { "find": "AAA", "replace": "111" },
                { "find": "BBB", "replace": "222" },
                { "find": "CCC", "replace": "333" }
            ]
        }))
        .await
        .unwrap();

    assert_eq!(result["patches_applied"].as_u64().unwrap(), 3);
    let content = std::fs::read_to_string(tmp.path().join("skills/my-skill/SKILL.md")).unwrap();
    assert!(content.contains("111 222 333"));
}

#[tokio::test]
async fn test_patch_skill_find_not_found_fails() {
    let tmp = tempfile::tempdir().unwrap();
    let create = CreateSkillTool::new(tmp.path().to_path_buf());
    let patch = PatchSkillTool::new(tmp.path().to_path_buf());

    create
        .execute(json!({
            "name": "my-skill",
            "description": "test",
            "body": "Hello world"
        }))
        .await
        .unwrap();

    let result = patch
        .execute(json!({
            "name": "my-skill",
            "patches": [
                { "find": "NOTFOUND", "replace": "oops" }
            ]
        }))
        .await;

    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(err_msg.contains("not found"), "error: {err_msg}");
}

#[tokio::test]
async fn test_patch_skill_nonexistent_fails() {
    let tmp = tempfile::tempdir().unwrap();
    let patch = PatchSkillTool::new(tmp.path().to_path_buf());

    let result = patch
        .execute(json!({
            "name": "nope",
            "patches": [{ "find": "a", "replace": "b" }]
        }))
        .await;

    assert!(result.is_err());
}

#[tokio::test]
async fn test_patch_skill_invalid_name_fails() {
    let tmp = tempfile::tempdir().unwrap();
    let patch = PatchSkillTool::new(tmp.path().to_path_buf());

    let result = patch
        .execute(json!({
            "name": "Bad Name",
            "patches": [{ "find": "a", "replace": "b" }]
        }))
        .await;

    assert!(result.is_err());
}

#[tokio::test]
async fn test_patch_skill_empty_patches_fails() {
    let tmp = tempfile::tempdir().unwrap();
    let create = CreateSkillTool::new(tmp.path().to_path_buf());
    let patch = PatchSkillTool::new(tmp.path().to_path_buf());

    create
        .execute(json!({
            "name": "my-skill",
            "description": "test",
            "body": "body"
        }))
        .await
        .unwrap();

    let result = patch
        .execute(json!({
            "name": "my-skill",
            "patches": []
        }))
        .await;

    assert!(result.is_err());
}

#[tokio::test]
async fn test_patch_skill_updates_description() {
    let tmp = tempfile::tempdir().unwrap();
    let create = CreateSkillTool::new(tmp.path().to_path_buf());
    let patch = PatchSkillTool::new(tmp.path().to_path_buf());

    create
        .execute(json!({
            "name": "my-skill",
            "description": "old desc",
            "body": "Hello world"
        }))
        .await
        .unwrap();

    patch
        .execute(json!({
            "name": "my-skill",
            "patches": [{ "find": "Hello", "replace": "Goodbye" }],
            "description": "new desc"
        }))
        .await
        .unwrap();

    let content = std::fs::read_to_string(tmp.path().join("skills/my-skill/SKILL.md")).unwrap();
    assert!(
        content.contains("description: \"new desc\""),
        "patched description should be YAML-quoted: {content}"
    );
    assert!(content.contains("Goodbye world"));
}

#[tokio::test]
async fn test_patch_skill_creates_checkpoint() {
    let tmp = tempfile::tempdir().unwrap();
    let create = CreateSkillTool::new(tmp.path().to_path_buf());
    let patch = PatchSkillTool::new(tmp.path().to_path_buf());
    let checkpoints = CheckpointManager::new(tmp.path().to_path_buf());

    create
        .execute(json!({
            "name": "my-skill",
            "description": "test",
            "body": "AAA BBB"
        }))
        .await
        .unwrap();

    let result = patch
        .execute(json!({
            "name": "my-skill",
            "patches": [{ "find": "AAA", "replace": "XXX" }]
        }))
        .await
        .unwrap();

    let checkpoint_id = result["checkpointId"].as_str().unwrap();
    checkpoints.restore(checkpoint_id).await.unwrap();

    let content = std::fs::read_to_string(tmp.path().join("skills/my-skill/SKILL.md")).unwrap();
    assert!(
        content.contains("AAA BBB"),
        "checkpoint should restore original"
    );
}

#[tokio::test]
async fn test_patch_skill_empty_find_fails() {
    let tmp = tempfile::tempdir().unwrap();
    let create = CreateSkillTool::new(tmp.path().to_path_buf());
    let patch = PatchSkillTool::new(tmp.path().to_path_buf());

    create
        .execute(json!({
            "name": "my-skill",
            "description": "test",
            "body": "body"
        }))
        .await
        .unwrap();

    let result = patch
        .execute(json!({
            "name": "my-skill",
            "patches": [{ "find": "", "replace": "oops" }]
        }))
        .await;

    assert!(result.is_err());
}

// ── split_frontmatter_body / update_frontmatter_description tests ───────

#[test]
fn test_split_frontmatter_body_with_frontmatter() {
    let raw = "---\nname: foo\ndescription: bar\n---\n\nBody text here.";
    let (fm, body) = split_frontmatter_body(raw);
    assert!(fm.starts_with("---"));
    assert!(fm.contains("name: foo"));
    assert_eq!(body, "Body text here.");
}

#[test]
fn test_split_frontmatter_body_no_frontmatter() {
    let raw = "Just a body.";
    let (fm, body) = split_frontmatter_body(raw);
    assert_eq!(fm, "");
    assert_eq!(body, "Just a body.");
}

#[test]
fn test_split_frontmatter_body_unclosed_frontmatter() {
    let raw = "---\nname: foo\nno closing delimiter";
    let (fm, body) = split_frontmatter_body(raw);
    // Unclosed frontmatter is treated as no frontmatter.
    assert_eq!(fm, "");
    assert_eq!(body, raw);
}

#[test]
fn test_split_frontmatter_body_empty_frontmatter() {
    let raw = "---\n---\nBody after empty frontmatter.";
    let (fm, body) = split_frontmatter_body(raw);
    assert!(fm.contains("---\n---"));
    assert_eq!(body, "Body after empty frontmatter.");
}

#[test]
fn test_split_frontmatter_body_no_trailing_newline() {
    let raw = "---\nname: x\n---";
    let (fm, body) = split_frontmatter_body(raw);
    assert!(fm.contains("---\nname: x\n---"));
    assert_eq!(body, "");
}

#[test]
fn test_update_frontmatter_description_replaces() {
    let fm = "---\nname: foo\ndescription: old\n---\n\n";
    let result = update_frontmatter_description(fm, "new desc");
    assert!(
        result.contains("description: \"new desc\""),
        "description should be YAML-quoted: {result}"
    );
    assert!(!result.contains("description: old"));
    assert!(result.contains("name: foo"));
}

#[test]
fn test_update_frontmatter_description_missing_field() {
    let fm = "---\nname: foo\n---\n\n";
    let result = update_frontmatter_description(fm, "new desc");
    // No description line to replace — should preserve original.
    assert!(!result.contains("new desc"));
    assert!(result.contains("name: foo"));
}

#[test]
fn test_update_frontmatter_description_with_yaml_special_chars() {
    let fm = "---\nname: foo\ndescription: old\n---\n\n";
    let result = update_frontmatter_description(fm, "has: colons and # hashes");
    // Value should be double-quoted to prevent YAML misinterpretation.
    assert!(
        result.contains(r#"description: "has: colons and # hashes""#),
        "description should be YAML-quoted: {result}"
    );
}

#[test]
fn test_update_frontmatter_description_escapes_quotes() {
    let fm = "---\nname: foo\ndescription: old\n---\n\n";
    let result = update_frontmatter_description(fm, r#"says "hello""#);
    assert!(
        result.contains(r#"description: "says \"hello\"""#),
        "internal quotes should be escaped: {result}"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn test_patch_skill_rejects_symlinked_skill_root() {
    use std::os::unix::fs::symlink;

    let tmp = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();

    let skills_dir = tmp.path().join("skills");
    std::fs::create_dir_all(&skills_dir).unwrap();
    let real_dir = outside.path().join("real-skill");
    std::fs::create_dir_all(&real_dir).unwrap();
    std::fs::write(
        real_dir.join("SKILL.md"),
        "---\nname: evil\ndescription: evil\n---\n\nevil body",
    )
    .unwrap();
    symlink(&real_dir, skills_dir.join("evil")).unwrap();

    let patch = PatchSkillTool::new(tmp.path().to_path_buf());
    let result = patch
        .execute(json!({
            "name": "evil",
            "patches": [{ "find": "evil", "replace": "good" }]
        }))
        .await;

    assert!(result.is_err());
    // Original file should not be modified.
    let content = std::fs::read_to_string(real_dir.join("SKILL.md")).unwrap();
    assert!(content.contains("evil body"));
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
