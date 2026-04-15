//! End-to-end test for the `read_skill` agent tool, mirroring the live
//! gateway's skill-tool registration.
//!
//! The gateway registers `ReadSkillTool` alongside the other skill management
//! tools in `server.rs`. This test bootstraps a matching `ToolRegistry`
//! independently (using the same discoverer type the gateway does), seeds a
//! temporary skill tree, and exercises the registered tool end-to-end:
//!
//!   1. `read_skill` is present in the registry's tool list.
//!   2. `{ name: "test-skill" }` returns the seeded body and lists sidecars.
//!   3. `{ name: "test-skill", file_path: "references/notes.md" }` returns
//!      the sidecar content.
//!   4. An unknown skill name returns a friendly error.
//!   5. A path-traversal `file_path` is rejected.
//!
//! This is the test that would have caught the original bug where the model
//! silently fell back to a filesystem MCP server (or fabricated a body) when
//! no native read tool was registered.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{path::Path, sync::Arc};

use {
    moltis_agents::tool_registry::{AgentTool, ToolRegistry},
    moltis_skills::{
        discover::{FsSkillDiscoverer, SkillDiscoverer},
        types::SkillSource,
    },
    serde_json::json,
    tempfile::TempDir,
};

/// Seed a personal-source skill at `<data_dir>/skills/<name>` with a known
/// SKILL.md body, and create a `references/notes.md` sidecar. Returns the
/// `(data_dir guard, skill_dir_path)` pair.
fn seed_test_skill(name: &str, body: &str) -> (TempDir, std::path::PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let skill_dir = tmp.path().join("skills").join(name);
    std::fs::create_dir_all(&skill_dir).unwrap();
    std::fs::write(
        skill_dir.join("SKILL.md"),
        format!("---\nname: {name}\ndescription: an integration-test skill\n---\n{body}"),
    )
    .unwrap();
    std::fs::create_dir_all(skill_dir.join("references")).unwrap();
    std::fs::write(
        skill_dir.join("references/notes.md"),
        "# Notes\n\nSome reference material.\n",
    )
    .unwrap();
    (tmp, skill_dir)
}

/// Hermetic discoverer that only sees `<data_dir>/skills` as a Personal
/// source — avoids hitting the user's real `~/.moltis` during tests.
fn hermetic_discoverer(data_dir: &Path) -> Arc<dyn SkillDiscoverer> {
    Arc::new(FsSkillDiscoverer::new(vec![(
        data_dir.join("skills"),
        SkillSource::Personal,
    )]))
}

/// Build a `ToolRegistry` populated the same way `server.rs` does for
/// skill-management tools, but scoped to a single temporary data directory.
fn registry_for(data_dir: &Path) -> ToolRegistry {
    let mut registry = ToolRegistry::new();

    // Mirror the gateway's skill tool registration block from `server.rs`.
    registry.register(Box::new(moltis_tools::skill_tools::CreateSkillTool::new(
        data_dir.to_path_buf(),
    )));
    registry.register(Box::new(moltis_tools::skill_tools::UpdateSkillTool::new(
        data_dir.to_path_buf(),
    )));
    registry.register(Box::new(moltis_tools::skill_tools::DeleteSkillTool::new(
        data_dir.to_path_buf(),
    )));
    // `WriteSkillFilesTool` is gated behind `config.skills.enable_agent_sidecar_files`
    // in production; the integration suite always registers it so we can
    // exercise the write-then-read round trip.
    registry.register(Box::new(
        moltis_tools::skill_tools::WriteSkillFilesTool::new(data_dir.to_path_buf()),
    ));

    // `ReadSkillTool` uses a discoverer pointed at the same personal-skills
    // directory the other tools write to. This intentionally does not include
    // the global default paths — the test must be hermetic.
    registry.register(Box::new(moltis_tools::skill_tools::ReadSkillTool::new(
        hermetic_discoverer(data_dir),
    )));

    registry
}

#[tokio::test]
async fn read_skill_tool_is_registered() {
    let (tmp, _skill_dir) = seed_test_skill("test-skill", "# Test\nHello.\n");
    let registry = registry_for(tmp.path());
    let names = registry.list_names();
    assert!(
        names.iter().any(|name| name == "read_skill"),
        "read_skill must be registered alongside the other skill tools, got: {names:?}"
    );
    // Sanity check: the create/update/delete tools are still there too, so
    // the registration block is intact.
    assert!(names.iter().any(|name| name == "create_skill"));
    assert!(names.iter().any(|name| name == "update_skill"));
    assert!(names.iter().any(|name| name == "delete_skill"));
}

#[tokio::test]
async fn read_skill_primary_call_returns_body_and_sidecar_listing() {
    let (tmp, _skill_dir) = seed_test_skill("test-skill", "# Test\n\nHello world.\n");
    let registry = registry_for(tmp.path());
    let tool = registry
        .get("read_skill")
        .expect("read_skill is registered");

    let result = tool
        .execute(json!({ "name": "test-skill" }))
        .await
        .expect("reading a seeded skill must succeed");

    assert_eq!(result["name"], "test-skill");
    assert_eq!(result["source"], "personal");
    assert_eq!(result["description"], "an integration-test skill");
    let body = result["body"].as_str().expect("body is a string");
    assert!(
        body.contains("Hello world"),
        "body should contain seeded text: {body:?}"
    );

    let linked: Vec<String> = result["linked_files"]
        .as_array()
        .expect("linked_files is an array")
        .iter()
        .map(|v| v["path"].as_str().unwrap().to_string())
        .collect();
    assert!(
        linked.contains(&"references/notes.md".to_string()),
        "linked_files should list the seeded sidecar: {linked:?}"
    );

    // The serialized response must not leak the absolute tmp path.
    let serialized = serde_json::to_string(&result).unwrap();
    assert!(
        !serialized.contains(tmp.path().to_string_lossy().as_ref()),
        "response must not leak the absolute data_dir path: {serialized}"
    );
}

#[tokio::test]
async fn read_skill_sidecar_call_returns_file_content() {
    let (tmp, _skill_dir) = seed_test_skill("test-skill", "# Test\n");
    let registry = registry_for(tmp.path());
    let tool = registry
        .get("read_skill")
        .expect("read_skill is registered");

    let result = tool
        .execute(json!({
            "name": "test-skill",
            "file_path": "references/notes.md"
        }))
        .await
        .expect("reading the sidecar must succeed");

    assert_eq!(result["name"], "test-skill");
    assert_eq!(result["file_path"], "references/notes.md");
    assert_eq!(
        result["content"].as_str().unwrap(),
        "# Notes\n\nSome reference material.\n"
    );
}

#[tokio::test]
async fn read_skill_unknown_name_returns_error() {
    let (tmp, _skill_dir) = seed_test_skill("test-skill", "# Test\n");
    let registry = registry_for(tmp.path());
    let tool = registry
        .get("read_skill")
        .expect("read_skill is registered");

    let result = tool.execute(json!({ "name": "does-not-exist" })).await;
    let err = result.expect_err("unknown skill name must return an error");
    let msg = format!("{err}");
    assert!(
        msg.contains("'does-not-exist'"),
        "error should name the skill: {msg}"
    );
    assert!(
        msg.contains("test-skill"),
        "error should hint at available names: {msg}"
    );
}

#[tokio::test]
async fn read_skill_rejects_path_traversal_in_file_path() {
    let (tmp, _skill_dir) = seed_test_skill("test-skill", "# Test\n");
    // Create a sibling file that traversal would try to reach.
    std::fs::write(tmp.path().join("skills/secret.txt"), "top secret\n").unwrap();
    let registry = registry_for(tmp.path());
    let tool = registry
        .get("read_skill")
        .expect("read_skill is registered");

    let result = tool
        .execute(json!({
            "name": "test-skill",
            "file_path": "../../etc/passwd"
        }))
        .await;
    assert!(result.is_err(), "path traversal must be rejected");
}

#[tokio::test]
async fn read_skill_lists_assets_directory_end_to_end() {
    // Regression coverage for the agentskills.io `assets/` standard.
    let (tmp, skill_dir) = seed_test_skill("test-skill", "# Test\n");
    std::fs::create_dir_all(skill_dir.join("assets")).unwrap();
    std::fs::write(skill_dir.join("assets/logo.txt"), "logo\n").unwrap();
    let registry = registry_for(tmp.path());
    let tool = registry
        .get("read_skill")
        .expect("read_skill is registered");

    let result = tool.execute(json!({ "name": "test-skill" })).await.unwrap();
    let linked: Vec<String> = result["linked_files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v["path"].as_str().unwrap().to_string())
        .collect();
    assert!(
        linked.contains(&"assets/logo.txt".to_string()),
        "assets/ files must appear in the end-to-end linked_files output: {linked:?}"
    );
}

#[tokio::test]
async fn read_skill_missing_sidecar_returns_helpful_listing_end_to_end() {
    let (tmp, skill_dir) = seed_test_skill("test-skill", "# Test\n");
    std::fs::write(skill_dir.join("references/other.md"), "other\n").unwrap();
    let registry = registry_for(tmp.path());
    let tool = registry
        .get("read_skill")
        .expect("read_skill is registered");

    let result = tool
        .execute(json!({
            "name": "test-skill",
            "file_path": "references/does-not-exist.md"
        }))
        .await;
    let err = result.expect_err("missing sidecar must error");
    let msg = format!("{err}");
    assert!(
        msg.contains("references/notes.md"),
        "should list the seeded notes.md: {msg}"
    );
    assert!(
        msg.contains("references/other.md"),
        "should list the seeded other.md: {msg}"
    );
}

#[tokio::test]
async fn read_skill_binary_sidecar_returns_structured_response_end_to_end() {
    let (tmp, skill_dir) = seed_test_skill("test-skill", "# Test\n");
    std::fs::create_dir_all(skill_dir.join("assets")).unwrap();
    let bytes: &[u8] = &[0xff, 0xfe, 0x00, 0x01];
    std::fs::write(skill_dir.join("assets/payload.bin"), bytes).unwrap();
    let registry = registry_for(tmp.path());
    let tool = registry
        .get("read_skill")
        .expect("read_skill is registered");

    let result = tool
        .execute(json!({
            "name": "test-skill",
            "file_path": "assets/payload.bin"
        }))
        .await
        .unwrap();
    assert_eq!(result["is_binary"], true);
    assert_eq!(result["bytes"].as_u64().unwrap(), bytes.len() as u64);
    assert!(
        result.get("content").is_none(),
        "binary response must omit `content`: {result}"
    );
}

// ── Parity invariants ──────────────────────────────────────────────────────
//
// The following tests verify the *cross-file* invariants that, if silently
// broken by a future refactor, would reintroduce the original fabrication
// bug. Every test in this section should be treated as a trip-wire: if it
// starts failing, the immediate question is "did we just regress skills?".

/// The tool registered under the name `read_skill` must match the tool name
/// the prompt generator advertises. If someone renames the tool or the
/// prompt instruction without updating the other side, the model will never
/// find the tool the prompt tells it to use — reproducing the original bug.
#[tokio::test]
async fn read_skill_tool_name_matches_prompt_instruction() {
    let (tmp, _skill_dir) = seed_test_skill("test-skill", "# Test\n");
    let registry = registry_for(tmp.path());

    // 1. The registered tool's `name()` must equal the shared constant
    //    the prompt generator references.
    let tool = registry
        .get(moltis_skills::prompt_gen::READ_SKILL_TOOL_NAME)
        .expect("tool name in prompt must resolve in the registry");
    assert_eq!(tool.name(), moltis_skills::prompt_gen::READ_SKILL_TOOL_NAME);

    // 2. Generating a prompt must mention the exact tool name.
    let skills = vec![moltis_skills::types::SkillMetadata {
        name: "test-skill".into(),
        description: "an integration-test skill".into(),
        path: tmp.path().join("skills/test-skill"),
        source: Some(SkillSource::Personal),
        ..Default::default()
    }];
    let prompt = moltis_skills::prompt_gen::generate_skills_prompt(&skills);
    assert!(
        prompt.contains(moltis_skills::prompt_gen::READ_SKILL_TOOL_NAME),
        "activation instruction must name `read_skill`: {prompt}"
    );

    // 3. The registered tool's schema must accept the shape the prompt
    //    advertises — `{ "name": "..." }` is all the primary call needs.
    let schema = tool.parameters_schema();
    assert_eq!(schema["type"], "object");
    let required: Vec<&str> = schema["required"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert!(
        required.contains(&"name"),
        "primary read must require `name`: {schema}"
    );
    assert!(
        schema["properties"]["file_path"].is_object(),
        "sidecar read must accept optional `file_path`: {schema}"
    );
}

/// Every skill name the prompt advertises must resolve through the tool.
/// This is the parity invariant the whole PR is built on — the prompt and
/// the tool share a discoverer, so what the model sees listed is what the
/// model can actually load. Without this test, a future refactor that
/// accidentally pointed them at different discoverers would silently break
/// skills again.
#[tokio::test]
async fn read_skill_resolves_every_name_listed_in_prompt() {
    let tmp = tempfile::tempdir().unwrap();

    // Seed three distinct skills so the prompt has more than one entry
    // to parse. Each lives in the hermetic `<data_dir>/skills/` tree the
    // discoverer was configured to walk.
    for (name, body) in [
        ("alpha", "# Alpha body\n"),
        ("beta", "# Beta body\n"),
        ("gamma", "# Gamma body\n"),
    ] {
        let skill_dir = tmp.path().join("skills").join(name);
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: test {name}\n---\n{body}"),
        )
        .unwrap();
    }

    // Share ONE discoverer between the prompt builder and the tool —
    // this is the parity invariant in production too.
    let discoverer = hermetic_discoverer(tmp.path());

    // Build the prompt from the discoverer's snapshot.
    let skills = discoverer.discover().await.unwrap();
    let prompt = moltis_skills::prompt_gen::generate_skills_prompt(&skills);

    // Extract every `name="..."` token from the prompt. The prompt is
    // generated by our own code, so a naïve substring scan is fine.
    let mut listed_names = Vec::new();
    let mut rest = prompt.as_str();
    while let Some(idx) = rest.find("<skill name=\"") {
        rest = &rest[idx + "<skill name=\"".len()..];
        if let Some(end) = rest.find('"') {
            listed_names.push(rest[..end].to_string());
            rest = &rest[end..];
        }
    }
    assert_eq!(
        listed_names.len(),
        3,
        "prompt should list all three seeded skills, got {listed_names:?}"
    );

    // For each name the prompt lists, the tool must resolve it — no
    // fabrication, no MCP fallback, just the discoverer's view.
    let tool = moltis_tools::skill_tools::ReadSkillTool::new(discoverer);
    for name in &listed_names {
        let result = tool
            .execute(json!({ "name": name }))
            .await
            .unwrap_or_else(|e| panic!("name '{name}' listed in prompt must resolve: {e}"));
        assert_eq!(result["name"], name.as_str());
        assert!(
            result["body"]
                .as_str()
                .unwrap()
                .contains(&format!("{} body", capitalize(name))),
            "body for '{name}' must match seeded content: {result}"
        );
    }
}

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

// ── Round-trip: write side → read side ────────────────────────────────────
//
// These tests prove that the create/update/write_files tools and the read
// tool agree on the on-disk layout. If someone changes the skills directory
// path in one place but not the other, these round-trips break immediately.

/// Use `create_skill` to write a new personal skill, then `read_skill` to
/// load it back. The two tools must share the same `<data_dir>/skills/<name>`
/// layout — if they ever disagree, the agent can write a skill it can never
/// read back, which is its own silent-failure mode.
#[tokio::test]
async fn create_skill_then_read_skill_round_trip() {
    let tmp = tempfile::tempdir().unwrap();
    let registry = registry_for(tmp.path());

    let create = registry
        .get("create_skill")
        .expect("create_skill is registered");
    create
        .execute(json!({
            "name": "round-trip",
            "description": "created via CreateSkillTool",
            "body": "# Round-trip body\n\nHello from create_skill.\n"
        }))
        .await
        .expect("create must succeed");

    let read = registry
        .get("read_skill")
        .expect("read_skill is registered");
    let result = read
        .execute(json!({ "name": "round-trip" }))
        .await
        .expect("read must succeed for a just-created skill");
    assert_eq!(result["name"], "round-trip");
    assert_eq!(result["description"], "created via CreateSkillTool");
    assert!(
        result["body"].as_str().unwrap().contains("Round-trip body"),
        "body must reflect create_skill output: {result}"
    );
    assert_eq!(result["source"], "personal");
}

/// Create a skill via `create_skill`, add a sidecar via `write_skill_files`,
/// and read the sidecar back via `read_skill`. This verifies the full write
/// side is reachable from the read side with no layout skew.
#[tokio::test]
async fn write_skill_files_then_read_skill_sidecar_round_trip() {
    let tmp = tempfile::tempdir().unwrap();
    let registry = registry_for(tmp.path());

    registry
        .get("create_skill")
        .unwrap()
        .execute(json!({
            "name": "with-sidecars",
            "description": "sidecar round-trip",
            "body": "# Body\n"
        }))
        .await
        .unwrap();

    registry
        .get("write_skill_files")
        .unwrap()
        .execute(json!({
            "name": "with-sidecars",
            "files": [
                { "path": "references/api.md", "content": "# API notes\n" },
                { "path": "templates/prompt.txt", "content": "hello\n" },
                { "path": "assets/config.yaml", "content": "k: v\n" }
            ]
        }))
        .await
        .expect("write_skill_files must succeed");

    // Primary read should list all three sidecars.
    let read = registry.get("read_skill").unwrap();
    let primary = read
        .execute(json!({ "name": "with-sidecars" }))
        .await
        .unwrap();
    let linked: Vec<String> = primary["linked_files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v["path"].as_str().unwrap().to_string())
        .collect();
    assert!(linked.contains(&"references/api.md".to_string()));
    assert!(linked.contains(&"templates/prompt.txt".to_string()));
    assert!(linked.contains(&"assets/config.yaml".to_string()));

    // Each sidecar must be readable by its advertised path.
    for (path, expected) in [
        ("references/api.md", "# API notes\n"),
        ("templates/prompt.txt", "hello\n"),
        ("assets/config.yaml", "k: v\n"),
    ] {
        let result = read
            .execute(json!({ "name": "with-sidecars", "file_path": path }))
            .await
            .unwrap_or_else(|e| panic!("sidecar {path} must be readable: {e}"));
        assert_eq!(
            result["content"].as_str().unwrap(),
            expected,
            "sidecar {path} content must round-trip"
        );
    }
}

/// Update → read round-trip: after `update_skill` replaces the body, the
/// next `read_skill` call must reflect the new content. Because the
/// discoverer re-runs on every `execute()`, this works without any
/// explicit cache invalidation.
#[tokio::test]
async fn update_skill_then_read_skill_reflects_new_body() {
    let tmp = tempfile::tempdir().unwrap();
    let registry = registry_for(tmp.path());

    registry
        .get("create_skill")
        .unwrap()
        .execute(json!({
            "name": "mutable",
            "description": "v1",
            "body": "# Version 1\n"
        }))
        .await
        .unwrap();

    let read = registry.get("read_skill").unwrap();
    let before = read.execute(json!({ "name": "mutable" })).await.unwrap();
    assert!(before["body"].as_str().unwrap().contains("Version 1"));
    assert_eq!(before["description"], "v1");

    registry
        .get("update_skill")
        .unwrap()
        .execute(json!({
            "name": "mutable",
            "description": "v2",
            "body": "# Version 2\n"
        }))
        .await
        .unwrap();

    let after = read.execute(json!({ "name": "mutable" })).await.unwrap();
    assert!(
        after["body"].as_str().unwrap().contains("Version 2"),
        "read after update must reflect new body: {after}"
    );
    assert_eq!(after["description"], "v2");
}

/// After `delete_skill`, the read tool must return a not-found error with
/// the hint listing whatever skills remain.
#[tokio::test]
async fn delete_skill_then_read_skill_returns_not_found() {
    let tmp = tempfile::tempdir().unwrap();
    let registry = registry_for(tmp.path());

    for name in ["gone-soon", "still-here"] {
        registry
            .get("create_skill")
            .unwrap()
            .execute(json!({
                "name": name,
                "description": "t",
                "body": "# body\n"
            }))
            .await
            .unwrap();
    }
    registry
        .get("delete_skill")
        .unwrap()
        .execute(json!({ "name": "gone-soon" }))
        .await
        .unwrap();

    let err = registry
        .get("read_skill")
        .unwrap()
        .execute(json!({ "name": "gone-soon" }))
        .await
        .expect_err("deleted skill must not be readable");
    let msg = format!("{err}");
    assert!(
        msg.contains("'gone-soon'"),
        "error should name the skill: {msg}"
    );
    assert!(
        msg.contains("still-here"),
        "error hint should list remaining skills: {msg}"
    );
}
