use {
    super::*,
    moltis_config::{MoltisConfig, validate::Diagnostic},
};

#[test]
fn status_labels() {
    assert_eq!(Status::Ok.label(), "ok");
    assert_eq!(Status::Warn.label(), "warn");
    assert_eq!(Status::Fail.label(), "fail");
    assert_eq!(Status::Skip.label(), "skip");
    assert_eq!(Status::Info.label(), "info");
}

#[test]
fn section_push_counts() {
    let mut section = Section::new("test");
    section.push(Status::Ok, "good");
    section.push(Status::Warn, "attention");
    section.push(Status::Fail, "bad");
    assert_eq!(section.items.len(), 3);
    assert_eq!(section.items[0].status, Status::Ok);
    assert_eq!(section.items[1].status, Status::Warn);
    assert_eq!(section.items[2].status, Status::Fail);
}

#[test]
fn print_report_counts_errors_and_warnings() {
    let mut section = Section::new("test");
    section.push(Status::Ok, "fine");
    section.push(Status::Warn, "caution");
    section.push(Status::Warn, "caution2");
    section.push(Status::Fail, "broken");
    section.push(Status::Info, "note");

    let (errors, warnings) = print_report(&[section]);
    assert_eq!(errors, 1);
    assert_eq!(warnings, 2);
}

#[test]
fn config_validation_status_warns_for_deprecated_field() {
    let diagnostic = Diagnostic {
        severity: Severity::Warning,
        category: "deprecated-field",
        path: "memory.embedding_provider".into(),
        message: "deprecated field; use \"memory.provider\" instead".into(),
    };

    assert_eq!(config_validation_status(&diagnostic), Some(Status::Warn));
}

#[test]
fn check_providers_empty_config() {
    let config = MoltisConfig::default();
    let section = check_providers(&config);
    assert_eq!(section.items.len(), 1);
    assert_eq!(section.items[0].status, Status::Info);
    assert!(section.items[0].message.contains("No providers configured"));
}

#[test]
fn check_providers_with_config_key() {
    let mut config = MoltisConfig::default();
    let entry = moltis_config::schema::ProviderEntry {
        api_key: Some(secrecy::Secret::new("sk-test-fake".to_string())),
        ..Default::default()
    };
    config
        .providers
        .providers
        .insert("anthropic".to_string(), entry);

    let section = check_providers(&config);
    let anthropic_item = section
        .items
        .iter()
        .find(|i| i.message.contains("anthropic"));
    assert!(anthropic_item.is_some());
    assert_eq!(anthropic_item.unwrap().status, Status::Ok);
}

#[test]
fn check_providers_missing_key_warns() {
    let mut config = MoltisConfig::default();
    config.providers.providers.insert(
        "minimax".to_string(),
        moltis_config::schema::ProviderEntry::default(),
    );

    if std::env::var("MINIMAX_API_KEY").is_err() {
        let section = check_providers(&config);
        let item = section.items.iter().find(|i| i.message.contains("minimax"));
        assert!(item.is_some());
        assert_eq!(item.unwrap().status, Status::Warn);
    }
}

#[test]
fn check_providers_ollama_optional() {
    let mut config = MoltisConfig::default();
    config.providers.providers.insert(
        "ollama".to_string(),
        moltis_config::schema::ProviderEntry::default(),
    );

    let section = check_providers(&config);
    let ollama_item = section.items.iter().find(|i| i.message.contains("ollama"));
    assert!(ollama_item.is_some());
    let status = ollama_item.unwrap().status;
    assert!(
        status == Status::Info || status == Status::Ok,
        "ollama should be Info or Ok, got {status:?}",
    );
}

#[test]
fn check_providers_disabled_skipped() {
    let mut config = MoltisConfig::default();
    let entry = moltis_config::schema::ProviderEntry {
        enabled: false,
        ..Default::default()
    };
    config
        .providers
        .providers
        .insert("openai".to_string(), entry);

    let section = check_providers(&config);
    let openai_item = section.items.iter().find(|i| i.message.contains("openai"));
    assert!(openai_item.is_some());
    assert_eq!(openai_item.unwrap().status, Status::Skip);
}

#[test]
fn check_providers_oauth_skipped() {
    let mut config = MoltisConfig::default();
    config.providers.providers.insert(
        "github-copilot".to_string(),
        moltis_config::schema::ProviderEntry::default(),
    );

    let section = check_providers(&config);
    let gh_item = section
        .items
        .iter()
        .find(|i| i.message.contains("github-copilot"));
    assert!(gh_item.is_some());
    assert_eq!(gh_item.unwrap().status, Status::Skip);
}

#[test]
fn check_mcp_servers_empty() {
    let config = MoltisConfig::default();
    let section = check_mcp_servers(&config);
    assert_eq!(section.items.len(), 1);
    assert_eq!(section.items[0].status, Status::Info);
}

#[test]
fn check_mcp_servers_disabled_skipped() {
    let mut config = MoltisConfig::default();
    let entry = moltis_config::schema::McpServerEntry {
        command: "node".to_string(),
        args: vec![],
        env: Default::default(),
        headers: Default::default(),
        enabled: false,
        transport: String::new(),
        url: None,
        oauth: None,
        display_name: None,
        request_timeout_secs: None,
    };
    config.mcp.servers.insert("test".to_string(), entry);

    let section = check_mcp_servers(&config);
    let test_item = section.items.iter().find(|i| i.message.contains("test"));
    assert!(test_item.is_some());
    assert_eq!(test_item.unwrap().status, Status::Skip);
}

#[test]
fn check_mcp_servers_missing_command_fails() {
    let mut config = MoltisConfig::default();
    let entry = moltis_config::schema::McpServerEntry {
        command: String::new(),
        args: vec![],
        env: Default::default(),
        headers: Default::default(),
        enabled: true,
        transport: String::new(),
        url: None,
        oauth: None,
        display_name: None,
        request_timeout_secs: None,
    };
    config.mcp.servers.insert("broken".to_string(), entry);

    let section = check_mcp_servers(&config);
    let broken_item = section.items.iter().find(|i| i.message.contains("broken"));
    assert!(broken_item.is_some());
    assert_eq!(broken_item.unwrap().status, Status::Fail);
}

#[test]
fn check_mcp_servers_sse_with_url_ok() {
    let mut config = MoltisConfig::default();
    let entry = moltis_config::schema::McpServerEntry {
        command: String::new(),
        args: vec![],
        env: Default::default(),
        headers: Default::default(),
        enabled: true,
        transport: "sse".to_string(),
        url: Some("http://localhost:3000/sse".to_string()),
        oauth: None,
        display_name: None,
        request_timeout_secs: None,
    };
    config.mcp.servers.insert("remote".to_string(), entry);

    let section = check_mcp_servers(&config);
    let remote_item = section.items.iter().find(|i| i.message.contains("remote"));
    assert!(remote_item.is_some());
    assert_eq!(remote_item.unwrap().status, Status::Ok);
}

#[test]
fn check_mcp_servers_sse_without_url_fails() {
    let mut config = MoltisConfig::default();
    let entry = moltis_config::schema::McpServerEntry {
        command: String::new(),
        args: vec![],
        env: Default::default(),
        headers: Default::default(),
        enabled: true,
        transport: "sse".to_string(),
        url: None,
        oauth: None,
        display_name: None,
        request_timeout_secs: None,
    };
    config.mcp.servers.insert("broken-sse".to_string(), entry);

    let section = check_mcp_servers(&config);
    let item = section
        .items
        .iter()
        .find(|i| i.message.contains("broken-sse"));
    assert!(item.is_some());
    assert_eq!(item.unwrap().status, Status::Fail);
}

#[test]
fn check_mcp_servers_nonexistent_command_fails() {
    let mut config = MoltisConfig::default();
    let entry = moltis_config::schema::McpServerEntry {
        command: "definitely-not-a-real-command-xyz123".to_string(),
        args: vec![],
        env: Default::default(),
        headers: Default::default(),
        enabled: true,
        transport: String::new(),
        url: None,
        oauth: None,
        display_name: None,
        request_timeout_secs: None,
    };
    config.mcp.servers.insert("bad".to_string(), entry);

    let section = check_mcp_servers(&config);
    let item = section.items.iter().find(|i| i.message.contains("bad"));
    assert!(item.is_some());
    assert_eq!(item.unwrap().status, Status::Fail);
}

#[test]
fn check_directories_with_temp_dirs() {
    let temp = tempfile::TempDir::new().unwrap();
    let config_dir = temp.path().join("config");
    let data_dir = temp.path().join("data");
    std::fs::create_dir_all(&config_dir).unwrap();
    std::fs::create_dir_all(&data_dir).unwrap();

    let section = check_directories(Some(&config_dir), &data_dir);

    let ok_count = section
        .items
        .iter()
        .filter(|i| i.status == Status::Ok)
        .count();
    assert!(
        ok_count >= 2,
        "expected at least 2 OK items, got {ok_count}"
    );
}

#[test]
fn check_directories_missing_config_dir() {
    let temp = tempfile::TempDir::new().unwrap();
    let missing = temp.path().join("nonexistent");
    let data_dir = temp.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();

    let section = check_directories(Some(&missing), &data_dir);

    let fail_item = section
        .items
        .iter()
        .find(|i| i.status == Status::Fail && i.message.contains("Config directory missing"));
    assert!(fail_item.is_some());
}

#[tokio::test]
async fn check_database_missing_file() {
    let temp = tempfile::TempDir::new().unwrap();
    let section = check_database(temp.path()).await;
    assert_eq!(section.items.len(), 1);
    assert_eq!(section.items[0].status, Status::Skip);
}

#[tokio::test]
async fn check_database_valid_db() {
    let temp = tempfile::TempDir::new().unwrap();
    let db_path = temp.path().join("moltis.db");
    let db_url = format!("sqlite:{}?mode=rwc", db_path.display());
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&db_url)
        .await
        .unwrap();
    pool.close().await;

    let section = check_database(temp.path()).await;
    let ok_item = section.items.iter().find(|i| i.status == Status::Ok);
    assert!(
        ok_item.is_some(),
        "expected OK for valid db, got: {:?}",
        section
            .items
            .iter()
            .map(|i| (&i.status, &i.message))
            .collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn read_remote_exec_inventory_reports_pinned_defaults() {
    let temp = tempfile::TempDir::new().unwrap();
    let db_path = temp.path().join("moltis.db");
    let db_url = format!("sqlite:{}?mode=rwc", db_path.display());
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&db_url)
        .await
        .unwrap();

    sqlx::query(
        "CREATE TABLE ssh_keys (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL UNIQUE,
            private_key TEXT NOT NULL,
            public_key TEXT NOT NULL,
            fingerprint TEXT NOT NULL,
            encrypted INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "CREATE TABLE ssh_targets (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            label TEXT NOT NULL UNIQUE,
            target TEXT NOT NULL,
            port INTEGER,
            known_host TEXT,
            auth_mode TEXT NOT NULL DEFAULT 'system',
            key_id INTEGER,
            is_default INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO ssh_keys (name, private_key, public_key, fingerprint, encrypted)
         VALUES ('prod-key', 'PRIVATE', 'ssh-ed25519 AAAA...', 'SHA256:test', 1)",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO ssh_targets (label, target, known_host, auth_mode, key_id, is_default)
         VALUES ('prod', 'deploy@example.com', 'prod.example.com ssh-ed25519 AAAA...', 'managed', 1, 1)",
    )
    .execute(&pool)
    .await
    .unwrap();
    pool.close().await;

    let inventory = read_remote_exec_inventory(temp.path())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(inventory.managed_key_count, 1);
    assert_eq!(inventory.encrypted_key_count, 1);
    assert_eq!(inventory.managed_target_count, 1);
    assert_eq!(inventory.pinned_target_count, 1);
    assert_eq!(inventory.default_target_label.as_deref(), Some("prod"));
    assert!(inventory.default_target_is_pinned);
}

#[tokio::test]
async fn check_remote_exec_warns_for_unpinned_active_target() {
    let temp = tempfile::TempDir::new().unwrap();
    let db_path = temp.path().join("moltis.db");
    let db_url = format!("sqlite:{}?mode=rwc", db_path.display());
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&db_url)
        .await
        .unwrap();
    sqlx::query(
        "CREATE TABLE ssh_keys (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL UNIQUE,
            private_key TEXT NOT NULL,
            public_key TEXT NOT NULL,
            fingerprint TEXT NOT NULL,
            encrypted INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "CREATE TABLE ssh_targets (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            label TEXT NOT NULL UNIQUE,
            target TEXT NOT NULL,
            port INTEGER,
            known_host TEXT,
            auth_mode TEXT NOT NULL DEFAULT 'system',
            key_id INTEGER,
            is_default INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO ssh_targets (label, target, auth_mode, is_default)
         VALUES ('prod', 'deploy@example.com', 'system', 1)",
    )
    .execute(&pool)
    .await
    .unwrap();
    pool.close().await;

    let mut config = MoltisConfig::default();
    config.tools.exec.host = "ssh".to_string();
    let section = check_remote_exec(&config, temp.path()).await;
    assert!(
        section.items.iter().any(|item| {
            item.status == Status::Warn && item.message.contains("not host-pinned")
        })
    );
}

#[test]
fn check_security_no_api_keys_in_config() {
    let config = MoltisConfig::default();
    let temp = tempfile::TempDir::new().unwrap();
    let section = check_security(&config, Some(temp.path()), temp.path());

    let ok_item = section
        .items
        .iter()
        .find(|i| i.message.contains("No API keys in config file"));
    assert!(ok_item.is_some());
    assert_eq!(ok_item.unwrap().status, Status::Ok);
}

#[test]
fn check_security_api_keys_in_config_warns() {
    let mut config = MoltisConfig::default();
    let entry = moltis_config::schema::ProviderEntry {
        api_key: Some(secrecy::Secret::new("sk-test".to_string())),
        ..Default::default()
    };
    config
        .providers
        .providers
        .insert("anthropic".to_string(), entry);

    let temp = tempfile::TempDir::new().unwrap();
    let section = check_security(&config, Some(temp.path()), temp.path());

    let warn_item = section
        .items
        .iter()
        .find(|i| i.message.contains("API keys found in config"));
    assert!(warn_item.is_some());
    assert_eq!(warn_item.unwrap().status, Status::Warn);
}
