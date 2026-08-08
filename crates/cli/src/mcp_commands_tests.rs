#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{fs, process::Command};

use {clap::Parser, sqlx::SqlitePool};

use {super::*, crate::Cli};

fn git(path: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(["-c", "commit.gpgSign=false"])
        .args(args)
        .current_dir(path)
        .status()
        .unwrap();
    assert!(status.success());
}

fn source_repo(server_command: &str) -> tempfile::TempDir {
    let source = tempfile::tempdir().unwrap();
    git(source.path(), &["init", "-q", "--initial-branch=main"]);
    fs::write(
        source.path().join(".mcp.json"),
        format!(r#"{{"mcpServers":{{"demo":{{"command":"{server_command}"}}}}}}"#),
    )
    .unwrap();
    git(source.path(), &["add", ".mcp.json"]);
    git(source.path(), &[
        "-c",
        "user.name=test",
        "-c",
        "user.email=test@example.com",
        "commit",
        "-qm",
        "initial",
    ]);
    source
}

fn local_input(source: &Path, alias: &str) -> RepositoryInput {
    RepositoryInput {
        alias: alias.to_string(),
        source: SourceArgs {
            url: None,
            ssh: None,
            local: Some(source.to_path_buf()),
        },
        private: false,
        https_credential_id: None,
        ssh_target_id: None,
        requested_ref: "HEAD".to_string(),
        id: Some(format!("{alias}-id")),
        json: true,
    }
}

#[test]
fn parses_exact_repository_commands() {
    for args in [
        vec!["moltis", "mcp", "repositories", "list", "--json"],
        vec![
            "moltis",
            "mcp",
            "repositories",
            "preview",
            "--alias",
            "demo",
            "--url",
            "https://example.com/repo.git",
        ],
        vec![
            "moltis",
            "mcp",
            "repositories",
            "add",
            "--alias",
            "demo",
            "--local",
            "/tmp/repo",
            "--approve",
            "all",
            "--enable",
            "--json",
        ],
        vec![
            "moltis",
            "mcp",
            "repositories",
            "update",
            "--id",
            "demo",
            "--apply",
        ],
        vec!["moltis", "mcp", "repositories", "rollback", "--id", "demo"],
        vec!["moltis", "mcp", "repositories", "remove", "--id", "demo"],
        vec![
            "moltis",
            "mcp",
            "repositories",
            "approve",
            "--id",
            "demo",
            "--server",
            "one",
            "--server",
            "two",
            "--enable",
        ],
        vec![
            "moltis",
            "mcp",
            "credentials",
            "add",
            "--host",
            "git.example.com",
            "--username",
            "deploy-token",
            "--token-env",
            "DEPLOY_GIT_TOKEN",
            "--json",
        ],
    ] {
        assert!(Cli::try_parse_from(args).is_ok());
    }
}

#[tokio::test]
async fn credential_persistence_never_outputs_token() {
    let data = tempfile::tempdir().unwrap();
    let result = persist_credential(
        data.path(),
        "git.example.com",
        "deploy",
        Secret::new("private-token-value".into()),
    )
    .await
    .unwrap();
    let output = serde_json::to_string(&result).unwrap();
    assert!(!output.contains("private-token-value"));
    assert_eq!(result["credential"]["host"], "git.example.com");
    assert!(result["storageWarning"].is_string());
}

#[tokio::test]
async fn offline_credential_add_refuses_to_replace_encrypted_token() {
    let data = tempfile::tempdir().unwrap();
    let original = persist_credential(
        data.path(),
        "git.example.com",
        "deploy",
        Secret::new("original-token".into()),
    )
    .await
    .unwrap();
    let id = original["credential"]["id"].as_i64().unwrap();
    let pool = SqlitePool::connect(&format!(
        "sqlite:{}",
        data.path().join("moltis.db").display()
    ))
    .await
    .unwrap();
    sqlx::query(
        "UPDATE git_https_credentials SET token = 'encrypted-token', encrypted = 1 WHERE id = ?",
    )
    .bind(id)
    .execute(&pool)
    .await
    .unwrap();
    pool.close().await;

    let error = persist_credential(
        data.path(),
        "git.example.com",
        "deploy",
        Secret::new("replacement-token".into()),
    )
    .await
    .unwrap_err();

    assert!(
        error
            .to_string()
            .contains("refuses to replace it with plaintext")
    );
    let pool = SqlitePool::connect(&format!(
        "sqlite:{}",
        data.path().join("moltis.db").display()
    ))
    .await
    .unwrap();
    let row: (String, i64) =
        sqlx::query_as("SELECT token, encrypted FROM git_https_credentials WHERE id = ?")
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(row, ("encrypted-token".to_string(), 1));
}

#[tokio::test]
async fn local_add_is_idempotent_and_disabled_by_default() {
    let data = tempfile::tempdir().unwrap();
    let source = source_repo("node");
    let action = || McpAction::Repositories {
        action: RepositoryAction::Add(AddArgs {
            repository: local_input(source.path(), "demo"),
            approve: AddApproval::None,
            enable: false,
        }),
    };
    let (first, _) = execute(action(), data.path().to_path_buf()).await.unwrap();
    assert_eq!(first["status"], "installed");
    assert_eq!(first["candidates"][0]["enabled"], false);
    assert_eq!(first["candidates"][0]["approved"], false);
    let (second, _) = execute(action(), data.path().to_path_buf()).await.unwrap();
    assert_eq!(second["status"], "alreadyInstalled");
    let registry = McpRegistry::load(&data.path().join("mcp-servers.json")).unwrap();
    assert_eq!(registry.repositories.len(), 1);
    assert_eq!(registry.servers.len(), 1);
}

#[tokio::test]
async fn repeated_offline_previews_leave_no_persistent_revisions() {
    let data = tempfile::tempdir().unwrap();
    let source = source_repo("node");

    for _ in 0..3 {
        let action = McpAction::Repositories {
            action: RepositoryAction::Preview(local_input(source.path(), "demo")),
        };
        execute(action, data.path().to_path_buf()).await.unwrap();
        let revisions = data.path().join("mcp-repositories/demo-id/revisions");
        assert_eq!(
            fs::read_dir(revisions).map_or(0, |entries| entries.count()),
            0
        );
    }
}

#[tokio::test]
async fn collision_fails_and_approve_all_enables() {
    let data = tempfile::tempdir().unwrap();
    let source = source_repo("node");
    let action = McpAction::Repositories {
        action: RepositoryAction::Add(AddArgs {
            repository: local_input(source.path(), "demo"),
            approve: AddApproval::All,
            enable: true,
        }),
    };
    let (value, _) = execute(action, data.path().to_path_buf()).await.unwrap();
    assert_eq!(value["candidates"][0]["approved"], true);
    assert_eq!(value["candidates"][0]["enabled"], true);
    let other = source_repo("python");
    let conflicting = McpAction::Repositories {
        action: RepositoryAction::Add(AddArgs {
            repository: local_input(other.path(), "demo"),
            approve: AddApproval::None,
            enable: false,
        }),
    };
    assert!(
        execute(conflicting, data.path().to_path_buf())
            .await
            .unwrap_err()
            .to_string()
            .contains("conflicting")
    );
}

#[tokio::test]
async fn update_apply_invalidates_approval_and_remove_preserves_source() {
    let data = tempfile::tempdir().unwrap();
    let source = source_repo("node");
    let add = McpAction::Repositories {
        action: RepositoryAction::Add(AddArgs {
            repository: local_input(source.path(), "demo"),
            approve: AddApproval::All,
            enable: true,
        }),
    };
    execute(add, data.path().to_path_buf()).await.unwrap();
    fs::write(
        source.path().join(".mcp.json"),
        r#"{"mcpServers":{"demo":{"command":"python"}}}"#,
    )
    .unwrap();
    git(source.path(), &["add", ".mcp.json"]);
    git(source.path(), &[
        "-c",
        "user.name=test",
        "-c",
        "user.email=test@example.com",
        "commit",
        "-qm",
        "update",
    ]);
    let preview = McpAction::Repositories {
        action: RepositoryAction::Update(UpdateArgs {
            id: "demo-id".into(),
            apply: false,
            json: true,
        }),
    };
    let (value, _) = execute(preview, data.path().to_path_buf()).await.unwrap();
    assert_eq!(
        value["reconciliation"]["updated"].as_array().unwrap().len(),
        1
    );
    let apply = McpAction::Repositories {
        action: RepositoryAction::Update(UpdateArgs {
            id: "demo-id".into(),
            apply: true,
            json: true,
        }),
    };
    execute(apply, data.path().to_path_buf()).await.unwrap();
    let registry = McpRegistry::load(&data.path().join("mcp-servers.json")).unwrap();
    let config = registry.servers.values().next().unwrap();
    assert!(!config.enabled);
    assert!(
        !config
            .managed_origin
            .as_ref()
            .unwrap()
            .is_currently_approved()
    );
    fs::write(
        source.path().join(".mcp.json"),
        r#"{"mcpServers":{"demo":{"command":"ruby"}}}"#,
    )
    .unwrap();
    git(source.path(), &["add", ".mcp.json"]);
    git(source.path(), &[
        "-c",
        "user.name=test",
        "-c",
        "user.email=test@example.com",
        "commit",
        "-qm",
        "second update",
    ]);
    let apply = McpAction::Repositories {
        action: RepositoryAction::Update(UpdateArgs {
            id: "demo-id".into(),
            apply: true,
            json: true,
        }),
    };
    execute(apply, data.path().to_path_buf()).await.unwrap();
    let revisions = fs::read_dir(data.path().join("mcp-repositories/demo-id/revisions"))
        .unwrap()
        .count();
    assert_eq!(revisions, 2);
    let remove = McpAction::Repositories {
        action: RepositoryAction::Remove(IdArgs {
            id: "demo-id".into(),
            json: true,
        }),
    };
    execute(remove, data.path().to_path_buf()).await.unwrap();
    assert!(source.path().join(".git").is_dir());
    assert!(!data.path().join("mcp-repositories/demo-id").exists());
}

#[tokio::test]
async fn output_excludes_manifest_secrets_and_lock_contention_is_actionable() {
    let data = tempfile::tempdir().unwrap();
    let source = source_repo("secret-value");
    fs::write(
        source.path().join(".mcp.json"),
        r#"{"mcpServers":{"demo":{"command":"node","env":{"API_TOKEN":"secret-value"}}}}"#,
    )
    .unwrap();
    git(source.path(), &["add", ".mcp.json"]);
    git(source.path(), &[
        "-c",
        "user.name=test",
        "-c",
        "user.email=test@example.com",
        "commit",
        "-qm",
        "secret",
    ]);
    let action = McpAction::Repositories {
        action: RepositoryAction::Preview(local_input(source.path(), "demo")),
    };
    let (value, _) = execute(action, data.path().to_path_buf()).await.unwrap();
    assert!(
        !serde_json::to_string(&value)
            .unwrap()
            .contains("secret-value")
    );
    let _lock = ManagedRepositoryLock::try_acquire(data.path()).unwrap();
    let result = execute(
        McpAction::Repositories {
            action: RepositoryAction::List(OutputArgs { json: true }),
        },
        data.path().to_path_buf(),
    )
    .await;
    assert!(result.unwrap_err().to_string().contains("busy"));
}

#[tokio::test]
async fn missing_private_credential_is_actionable() {
    let data = tempfile::tempdir().unwrap();
    let input = RepositoryInput {
        alias: "private".into(),
        source: SourceArgs {
            url: Some("https://example.com/repo.git".into()),
            ssh: None,
            local: None,
        },
        private: true,
        https_credential_id: Some(7),
        ssh_target_id: None,
        requested_ref: "HEAD".into(),
        id: None,
        json: true,
    };
    let error = execute(
        McpAction::Repositories {
            action: RepositoryAction::Preview(input),
        },
        data.path().to_path_buf(),
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("credential database"));
    assert!(error.to_string().contains("preprovision"));
}
