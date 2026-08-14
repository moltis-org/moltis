#![allow(clippy::unwrap_used, clippy::expect_used)]

use {
    super::*,
    moltis_connectors::ProjectionConfig,
    secrecy::{ExposeSecret, Secret},
    uuid::Uuid,
};

#[path = "connectors_tests/channel_history.rs"]
mod channel_history;

fn configured_caldav(
    name: &str,
    provider: Option<&str>,
    url: Option<&str>,
    username: &str,
    password: &str,
) -> moltis_config::CalDavConfig {
    let mut accounts = std::collections::HashMap::new();
    accounts.insert(name.to_owned(), moltis_config::CalDavAccountConfig {
        url: url.map(str::to_owned),
        username: Some(username.to_owned()),
        password: Some(Secret::new(password.to_owned())),
        provider: provider.map(str::to_owned),
        timeout_seconds: 45,
    });
    moltis_config::CalDavConfig {
        enabled: true,
        default_account: Some(name.to_owned()),
        accounts,
    }
}

fn test_password() -> String {
    Uuid::new_v4().to_string()
}

async fn manager() -> (tempfile::TempDir, ConnectorManager) {
    #[cfg(feature = "vault")]
    crate::vault_lifecycle::set_vault_encryption_runtime_enabled(false);
    let temp = tempfile::tempdir().unwrap();
    let manager = ConnectorManager::open(
        temp.path(),
        1,
        #[cfg(feature = "vault")]
        None,
    )
    .await
    .unwrap();
    (temp, manager)
}

fn create_request() -> AccountCreateRequest {
    AccountCreateRequest {
        kind: ConnectorKind::Caldav,
        channel_type: None,
        channel_account_id: None,
        himalaya_account_name: None,
        himalaya_backend: None,
        name: "Calendar".to_owned(),
        server_url: "https://calendar.example.com".to_owned(),
        username: "user@example.com".to_owned(),
        password: Secret::new(test_password()),
        timeout_seconds: 30,
        allow_insecure_http: false,
        allow_private_network: false,
        enabled: true,
    }
}

fn item(remote_id: &str, search_text: &str) -> moltis_connectors::ConnectorItemInput {
    moltis_connectors::ConnectorItemInput {
        remote_id: remote_id.to_owned(),
        kind: "event".to_owned(),
        remote_version: Some("etag-1".to_owned()),
        occurred_at: None,
        updated_at: None,
        body_json: serde_json::json!({"summary": search_text}),
        search_text: search_text.to_owned(),
        content_hash: format!("hash-{remote_id}"),
    }
}

fn observation(
    remote_id: &str,
    disposition: moltis_connectors::SourceDisposition,
    plan_revision: u64,
) -> moltis_connectors::SourceObservation {
    moltis_connectors::SourceObservation {
        remote_id: remote_id.to_owned(),
        remote_version: Some("etag-1".to_owned()),
        disposition,
        filter_reason: (disposition == moltis_connectors::SourceDisposition::Filtered)
            .then(|| "outside filter".to_owned()),
        evaluated_plan_revision: plan_revision,
    }
}

#[tokio::test]
#[cfg_attr(feature = "vault", serial_test::serial(vault_runtime))]
async fn account_views_redact_password_and_redacted_updates_preserve_it() {
    let (_temp, manager) = manager().await;
    let request = create_request();
    let password = request.password.expose_secret().clone();
    let created = manager.add_account(request).await.unwrap();
    assert!(created.has_password);
    let serialized = serde_json::to_string(&created).unwrap();
    assert!(!serialized.contains(&password));
    assert!(!serialized.contains("password"));

    let updated = manager
        .update_account(&created.id, AccountUpdateRequest {
            name: "Work calendar".to_owned(),
            server_url: created.server_url.clone(),
            username: created.username.clone(),
            password: Some(Secret::new(REDACTED_PASSWORD.to_owned())),
            timeout_seconds: created.timeout_seconds,
            allow_insecure_http: created.allow_insecure_http,
            allow_private_network: created.allow_private_network,
            enabled: created.enabled,
        })
        .await
        .unwrap();

    assert!(updated.has_password);
    let stored = manager
        .store
        .get_account(&created.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.config["password"], password);
}

#[tokio::test]
#[cfg_attr(feature = "vault", serial_test::serial(vault_runtime))]
async fn account_authority_changes_require_a_password_and_managed_fields_reject_rpc_updates() {
    let (_temp, manager) = manager().await;
    let manual = manager.add_account(create_request()).await.unwrap();
    let manual_error = manager
        .update_account(&manual.id, AccountUpdateRequest {
            name: manual.name.clone(),
            server_url: "https://other.example.com".to_owned(),
            username: manual.username.clone(),
            password: None,
            timeout_seconds: manual.timeout_seconds,
            allow_insecure_http: manual.allow_insecure_http,
            allow_private_network: manual.allow_private_network,
            enabled: manual.enabled,
        })
        .await
        .unwrap_err();
    assert!(matches!(
        manual_error,
        ConnectorManagerError::InvalidInput(_)
    ));

    let password = test_password();
    let config = configured_caldav(
        "work",
        Some("fastmail"),
        None,
        "managed@example.com",
        &password,
    );
    manager
        .reconcile_configured_caldav_accounts(&config)
        .await
        .unwrap();
    let managed = manager
        .list_accounts()
        .await
        .unwrap()
        .into_iter()
        .find(|account| account.managed)
        .unwrap();
    let managed_error = manager
        .update_account(&managed.id, AccountUpdateRequest {
            name: managed.name.clone(),
            server_url: "https://attacker.example.com".to_owned(),
            username: managed.username.clone(),
            password: None,
            timeout_seconds: managed.timeout_seconds,
            allow_insecure_http: managed.allow_insecure_http,
            allow_private_network: managed.allow_private_network,
            enabled: managed.enabled,
        })
        .await
        .unwrap_err();
    assert!(matches!(
        managed_error,
        ConnectorManagerError::InvalidInput(_)
    ));
    let runtime = manager.runtime_account_config(&managed.id).await.unwrap();
    assert_eq!(runtime.server_url, "https://caldav.fastmail.com");
    assert_eq!(runtime.password.expose_secret(), &password);
}

#[tokio::test]
#[cfg_attr(feature = "vault", serial_test::serial(vault_runtime))]
async fn configured_caldav_accounts_import_once_and_reconcile_changes() {
    let (_temp, manager) = manager().await;
    let first_password = test_password();
    let mut config = configured_caldav(
        "work",
        Some("fastmail"),
        None,
        "user@example.com",
        &first_password,
    );

    let first = manager
        .reconcile_configured_caldav_accounts(&config)
        .await
        .unwrap();
    assert_eq!(first.created, 1);
    let accounts = manager.list_accounts().await.unwrap();
    assert_eq!(accounts.len(), 1);
    assert_eq!(accounts[0].server_url, "https://caldav.fastmail.com");
    assert_eq!(accounts[0].timeout_seconds, 45);
    assert!(accounts[0].managed);
    let account_id = accounts[0].id.clone();
    let dataset = manager
        .add_dataset(DatasetCreateRequest {
            account_id: account_id.clone(),
            name: "Imported calendar".to_owned(),
            instruction: "Keep all events".to_owned(),
            config: CalDavDatasetConfigView::default().into(),
            schedule_minutes: None,
            projections: ProjectionConfig::default(),
            enabled: true,
        })
        .await
        .unwrap();
    let stored = manager
        .store
        .get_account(&account_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.source_key.as_deref(), Some("config:caldav:work"));

    let second = manager
        .reconcile_configured_caldav_accounts(&config)
        .await
        .unwrap();
    assert_eq!(second.unchanged, 1);
    assert_eq!(manager.list_accounts().await.unwrap()[0].id, account_id);

    manager
        .update_account(&account_id, AccountUpdateRequest {
            name: accounts[0].name.clone(),
            server_url: accounts[0].server_url.clone(),
            username: accounts[0].username.clone(),
            password: None,
            timeout_seconds: accounts[0].timeout_seconds,
            allow_insecure_http: false,
            allow_private_network: true,
            enabled: true,
        })
        .await
        .unwrap();

    let legacy = config.accounts.get_mut("work").unwrap();
    legacy.username = Some("calendar-user@example.com".to_owned());
    let rotated_password = test_password();
    legacy.password = Some(Secret::new(rotated_password.clone()));
    let updated = manager
        .reconcile_configured_caldav_accounts(&config)
        .await
        .unwrap();
    assert_eq!(updated.updated, 1);
    let runtime = manager.runtime_account_config(&account_id).await.unwrap();
    assert_eq!(runtime.username, "calendar-user@example.com");
    assert_eq!(runtime.password.expose_secret(), &rotated_password);
    assert!(manager.list_accounts().await.unwrap()[0].allow_private_network);

    config.accounts.clear();
    let removed = manager
        .reconcile_configured_caldav_accounts(&config)
        .await
        .unwrap();
    assert_eq!(removed.disabled, 1);
    let account = manager
        .store
        .get_account(&account_id)
        .await
        .unwrap()
        .unwrap();
    assert!(!account.enabled);
    assert_eq!(account.source_key.as_deref(), Some("config:caldav:work"));
    assert_eq!(account.config["password"], "");
    assert!(!manager.list_accounts().await.unwrap()[0].has_password);
    assert!(
        manager
            .store
            .get_dataset(&dataset.id)
            .await
            .unwrap()
            .is_some()
    );

    let restored_password = test_password();
    let restored = configured_caldav(
        "work",
        Some("fastmail"),
        None,
        "restored@example.com",
        &restored_password,
    );
    let restored_report = manager
        .reconcile_configured_caldav_accounts(&restored)
        .await
        .unwrap();
    assert_eq!(restored_report.updated, 1);
    let runtime = manager.runtime_account_config(&account_id).await.unwrap();
    assert_eq!(runtime.username, "restored@example.com");
    assert_eq!(runtime.password.expose_secret(), &restored_password);
    let remove_error = manager.remove_account(&account_id).await.unwrap_err();
    assert!(matches!(
        remove_error,
        ConnectorManagerError::InvalidInput(_)
    ));
}

#[tokio::test]
#[cfg_attr(feature = "vault", serial_test::serial(vault_runtime))]
async fn configured_caldav_adopts_one_matching_manual_account() {
    let (_temp, manager) = manager().await;
    let request = create_request();
    let password = request.password.expose_secret().clone();
    let manual = manager.add_account(request).await.unwrap();
    let config = configured_caldav(
        "work",
        Some("generic"),
        Some("https://calendar.example.com"),
        "user@example.com",
        &password,
    );

    let report = manager
        .reconcile_configured_caldav_accounts(&config)
        .await
        .unwrap();
    assert_eq!(report.adopted, 1);
    let accounts = manager.store.list_accounts().await.unwrap();
    assert_eq!(accounts.len(), 1);
    assert_eq!(accounts[0].id, manual.id);
    assert_eq!(
        accounts[0].source_key.as_deref(),
        Some("config:caldav:work")
    );
}

#[tokio::test]
#[cfg_attr(feature = "vault", serial_test::serial(vault_runtime))]
async fn configured_caldav_does_not_guess_between_duplicate_manual_accounts() {
    let (_temp, manager) = manager().await;
    let request = create_request();
    let password = request.password.expose_secret().clone();
    manager.add_account(request).await.unwrap();
    let mut duplicate = create_request();
    duplicate.password = Secret::new(password.clone());
    duplicate.name = "Duplicate".to_owned();
    manager.add_account(duplicate).await.unwrap();
    let config = configured_caldav(
        "work",
        Some("generic"),
        Some("https://calendar.example.com"),
        "user@example.com",
        &password,
    );

    let report = manager
        .reconcile_configured_caldav_accounts(&config)
        .await
        .unwrap();
    assert_eq!(report.ambiguous, 1);
    let accounts = manager.store.list_accounts().await.unwrap();
    assert_eq!(accounts.len(), 2);
    assert!(accounts.iter().all(|account| account.source_key.is_none()));
}

#[tokio::test]
#[cfg_attr(feature = "vault", serial_test::serial(vault_runtime))]
async fn invalid_configured_caldav_account_does_not_block_valid_imports() {
    let (_temp, manager) = manager().await;
    let stale_password = test_password();
    let stale = configured_caldav(
        "invalid",
        Some("generic"),
        Some("https://stale.example.com"),
        "stale@example.com",
        &stale_password,
    );
    manager
        .reconcile_configured_caldav_accounts(&stale)
        .await
        .unwrap();
    let valid_password = test_password();
    let mut config = configured_caldav(
        "valid",
        Some("icloud"),
        None,
        "user@example.com",
        &valid_password,
    );
    config.accounts.insert(
        "invalid".to_owned(),
        moltis_config::CalDavAccountConfig::default(),
    );

    let report = manager
        .reconcile_configured_caldav_accounts(&config)
        .await
        .unwrap();
    assert_eq!(report.created, 1);
    assert_eq!(report.invalid, 1);
    assert_eq!(report.disabled, 1);
    let accounts = manager.store.list_accounts().await.unwrap();
    assert_eq!(accounts.len(), 2);
    let invalid = accounts
        .iter()
        .find(|account| account.source_key.as_deref() == Some("config:caldav:invalid"))
        .unwrap();
    assert!(!invalid.enabled);
    assert_eq!(invalid.config["password"], "");
}

#[tokio::test]
#[cfg(feature = "vault")]
#[serial_test::serial(vault_runtime)]
async fn configured_caldav_never_falls_back_to_plaintext_without_vault() {
    crate::vault_lifecycle::set_vault_encryption_runtime_enabled(true);
    let temp = tempfile::tempdir().unwrap();
    let manager = ConnectorManager::open(temp.path(), 1, None).await.unwrap();
    let password = test_password();
    let config = configured_caldav(
        "work",
        Some("generic"),
        Some("https://calendar.example.com"),
        "user@example.com",
        &password,
    );

    let report = manager
        .reconcile_configured_caldav_accounts(&config)
        .await
        .unwrap();
    assert_eq!(report.deferred, 1);
    assert!(manager.store.list_accounts().await.unwrap().is_empty());
    crate::vault_lifecycle::set_vault_encryption_runtime_enabled(false);
}

#[tokio::test]
#[cfg(feature = "vault")]
#[serial_test::serial(vault_runtime)]
async fn vault_runtime_blocks_plaintext_connector_credentials_without_a_vault() {
    crate::vault_lifecycle::set_vault_encryption_runtime_enabled(false);
    let temp = tempfile::tempdir().unwrap();
    let manager = ConnectorManager::open(temp.path(), 1, None).await.unwrap();
    let account = manager.add_account(create_request()).await.unwrap();
    crate::vault_lifecycle::set_vault_encryption_runtime_enabled(true);

    let error = manager
        .runtime_account_config(&account.id)
        .await
        .unwrap_err();
    assert!(matches!(error, ConnectorManagerError::Unavailable(_)));
    crate::vault_lifecycle::set_vault_encryption_runtime_enabled(false);
}

#[tokio::test]
#[cfg_attr(feature = "vault", serial_test::serial(vault_runtime))]
async fn scheduled_dataset_gets_a_future_next_sync() {
    let (_temp, manager) = manager().await;
    let account = manager.add_account(create_request()).await.unwrap();
    let before = OffsetDateTime::now_utc();
    let dataset = manager
        .add_dataset(DatasetCreateRequest {
            account_id: account.id,
            name: "All calendars".to_owned(),
            instruction: "Keep all calendars".to_owned(),
            config: CalDavDatasetConfigView::default().into(),
            schedule_minutes: Some(15),
            projections: ProjectionConfig::default(),
            enabled: true,
        })
        .await
        .unwrap();
    assert!(dataset.next_sync_at.is_some_and(|next| next > before));
    assert_eq!(dataset.instruction.as_deref(), Some("Keep all calendars"));
    assert!(dataset.needs_sync);
}

#[test]
fn dataset_config_and_planner_contracts_use_camel_case_and_preserve_filters() {
    let config = CalDavDatasetConfigView {
        schema_version: 1,
        selection: CalendarSelectionView::Selected {
            calendar_hrefs: vec!["/calendars/work/".to_owned()],
        },
        filters: CalDavFiltersView {
            start_date: Some("2026-08-01".to_owned()),
            end_date: Some("2026-09-01".to_owned()),
            accepted_by_account: true,
        },
    };
    let domain = CalDavDatasetConfig::from(config.clone());
    assert_eq!(CalDavDatasetConfigView::from(domain), config);

    let value = serde_json::to_value(&config).unwrap();
    assert_eq!(value["schemaVersion"], 1);
    assert_eq!(value["filters"]["startDate"], "2026-08-01");
    assert_eq!(value["filters"]["endDate"], "2026-09-01");
    assert_eq!(value["filters"]["acceptedByAccount"], true);

    let request: DatasetCompileRequest = serde_json::from_value(serde_json::json!({
        "accountId": "account-1",
        "datasetId": "dataset-1",
        "instruction": "Keep accepted events",
        "overrides": {"scheduleMinutes": 30}
    }))
    .unwrap();
    assert_eq!(request.account_id, "account-1");
    assert_eq!(request.dataset_id.as_deref(), Some("dataset-1"));
    assert_eq!(request.overrides.unwrap()["scheduleMinutes"], 30);
}

#[tokio::test]
#[cfg_attr(feature = "vault", serial_test::serial(vault_runtime))]
async fn email_accounts_and_datasets_keep_provider_owned_configs() {
    let (_temp, manager) = manager().await;
    assert!(
        manager
            .available()
            .iter()
            .any(|entry| entry.kind == ConnectorKind::Gmail)
    );
    assert!(
        manager
            .available()
            .iter()
            .any(|entry| entry.kind == ConnectorKind::Himalaya)
    );

    let gmail = manager
        .add_account(AccountCreateRequest {
            kind: ConnectorKind::Gmail,
            name: "Primary Gmail".to_owned(),
            server_url: String::new(),
            username: String::new(),
            password: Secret::new(String::new()),
            channel_type: None,
            channel_account_id: None,
            himalaya_account_name: None,
            himalaya_backend: None,
            timeout_seconds: 30,
            allow_insecure_http: false,
            allow_private_network: false,
            enabled: true,
        })
        .await
        .unwrap();
    assert_eq!(gmail.kind, ConnectorKind::Gmail);
    assert_eq!(gmail.credential_source.as_deref(), Some("google_workspace"));
    let gmail_dataset = manager
        .add_dataset(DatasetCreateRequest {
            account_id: gmail.id,
            name: "Alerts".to_owned(),
            instruction: "Keep alert emails".to_owned(),
            config: ConnectorDatasetConfigView::Gmail(GmailDatasetConfig {
                schema_version: 1,
                query: "from:alerts@example.test".to_owned(),
                max_messages: 40,
                include_body: false,
            }),
            schedule_minutes: None,
            projections: ProjectionConfig::default(),
            enabled: true,
        })
        .await
        .unwrap();
    assert!(matches!(
        gmail_dataset.config,
        ConnectorDatasetConfigView::Gmail(_)
    ));

    let himalaya = manager
        .add_account(AccountCreateRequest {
            kind: ConnectorKind::Himalaya,
            name: "Archive mail".to_owned(),
            server_url: String::new(),
            username: String::new(),
            password: Secret::new(String::new()),
            channel_type: None,
            channel_account_id: None,
            himalaya_account_name: Some("archive".to_owned()),
            himalaya_backend: Some(moltis_connector_himalaya::HimalayaBackend::Jmap),
            timeout_seconds: 30,
            allow_insecure_http: false,
            allow_private_network: false,
            enabled: true,
        })
        .await
        .unwrap();
    assert_eq!(himalaya.himalaya_account_name.as_deref(), Some("archive"));
    let himalaya_dataset = manager
        .add_dataset(DatasetCreateRequest {
            account_id: himalaya.id,
            name: "Invoices".to_owned(),
            instruction: "Keep invoice emails".to_owned(),
            config: ConnectorDatasetConfigView::Himalaya(HimalayaDatasetConfig {
                schema_version: 1,
                mailbox_ids: vec!["INBOX".to_owned()],
                query: Some("subject invoice".to_owned()),
                max_messages: 125,
                include_bodies: true,
            }),
            schedule_minutes: Some(60),
            projections: ProjectionConfig::default(),
            enabled: true,
        })
        .await
        .unwrap();
    assert!(matches!(
        himalaya_dataset.config,
        ConnectorDatasetConfigView::Himalaya(_)
    ));
}

#[test]
fn dataset_requests_require_provider_owned_config() {
    assert!(
        serde_json::from_value::<DatasetCreateRequest>(serde_json::json!({
            "accountId": "account",
            "name": "name",
            "instruction": "instruction"
        }))
        .is_err()
    );
    assert!(
        serde_json::from_value::<DatasetUpdateRequest>(serde_json::json!({
            "name": "name",
            "instruction": "instruction"
        }))
        .is_err()
    );
}

#[tokio::test]
#[cfg_attr(feature = "vault", serial_test::serial(vault_runtime))]
async fn draft_validation_checks_instruction_config_and_dataset_account_scope() {
    let (_temp, manager) = manager().await;
    let account = manager.add_account(create_request()).await.unwrap();
    let other_account = manager
        .add_account(AccountCreateRequest {
            name: "Other calendar".to_owned(),
            ..create_request()
        })
        .await
        .unwrap();
    let dataset = manager
        .add_dataset(DatasetCreateRequest {
            account_id: account.id.clone(),
            name: "Work".to_owned(),
            instruction: "Keep work events".to_owned(),
            config: CalDavDatasetConfigView::default().into(),
            schedule_minutes: None,
            projections: ProjectionConfig::default(),
            enabled: true,
        })
        .await
        .unwrap();
    let draft = DatasetDraft {
        name: "Work".to_owned(),
        config: CalDavDatasetConfigView::default(),
        schedule_minutes: Some(30),
        projections: ProjectionConfig::default(),
        enabled: true,
    };

    manager
        .validate_dataset_draft(&account.id, Some(&dataset.id), "Keep work events", &draft)
        .await
        .unwrap();
    let empty = manager
        .validate_dataset_draft(&account.id, None, "  ", &draft)
        .await
        .unwrap_err();
    assert!(matches!(empty, ConnectorManagerError::InvalidInput(_)));
    let oversized = "x".repeat(MAX_INSTRUCTION_BYTES + 1);
    let oversized = manager
        .validate_dataset_draft(&account.id, None, &oversized, &draft)
        .await
        .unwrap_err();
    assert!(matches!(oversized, ConnectorManagerError::InvalidInput(_)));
    let wrong_account = manager
        .validate_dataset_draft(
            &other_account.id,
            Some(&dataset.id),
            "Keep work events",
            &draft,
        )
        .await
        .unwrap_err();
    assert!(matches!(
        wrong_account,
        ConnectorManagerError::InvalidInput(_)
    ));
}

#[tokio::test]
#[cfg_attr(feature = "vault", serial_test::serial(vault_runtime))]
async fn account_and_dataset_mutations_reject_active_syncs() {
    let (_temp, manager) = manager().await;
    let account = manager.add_account(create_request()).await.unwrap();
    let dataset = manager
        .add_dataset(DatasetCreateRequest {
            account_id: account.id.clone(),
            name: "All calendars".to_owned(),
            instruction: "Keep all calendars".to_owned(),
            config: CalDavDatasetConfigView::default().into(),
            schedule_minutes: None,
            projections: ProjectionConfig::default(),
            enabled: true,
        })
        .await
        .unwrap();
    manager.active_dataset_ids.insert(dataset.id.clone());

    let account_error = manager
        .update_account(&account.id, AccountUpdateRequest {
            name: account.name,
            server_url: account.server_url,
            username: account.username,
            password: None,
            timeout_seconds: account.timeout_seconds,
            allow_insecure_http: account.allow_insecure_http,
            allow_private_network: account.allow_private_network,
            enabled: account.enabled,
        })
        .await
        .unwrap_err();
    assert!(matches!(account_error, ConnectorManagerError::Conflict(ref id) if id == &dataset.id));

    let dataset_error = manager.remove_dataset(&dataset.id).await.unwrap_err();
    assert!(matches!(dataset_error, ConnectorManagerError::Conflict(ref id) if id == &dataset.id));
}

#[tokio::test]
#[cfg_attr(feature = "vault", serial_test::serial(vault_runtime))]
async fn mutations_remove_obsolete_projection_directories() {
    let (temp, manager) = manager().await;
    let account = manager.add_account(create_request()).await.unwrap();
    let export_root = temp.path().join("connectors").join("exports");
    let projections = ProjectionConfig {
        jsonl: true,
        markdown: true,
    };

    let renamed = manager
        .add_dataset(DatasetCreateRequest {
            account_id: account.id.clone(),
            name: "Before rename".to_owned(),
            instruction: "Keep all calendars".to_owned(),
            config: CalDavDatasetConfigView::default().into(),
            schedule_minutes: None,
            projections,
            enabled: true,
        })
        .await
        .unwrap();
    let renamed_path =
        moltis_connectors::projection_directory(&export_root, "Before rename", &renamed.id)
            .unwrap();
    tokio::fs::create_dir_all(&renamed_path).await.unwrap();
    tokio::fs::write(renamed_path.join("items.jsonl"), "sensitive")
        .await
        .unwrap();
    manager
        .update_dataset(&renamed.id, DatasetUpdateRequest {
            name: "After rename".to_owned(),
            instruction: "Keep all calendars".to_owned(),
            config: CalDavDatasetConfigView::default().into(),
            schedule_minutes: None,
            projections: ProjectionConfig::default(),
            enabled: true,
        })
        .await
        .unwrap();
    assert!(!renamed_path.exists());

    let removed = manager
        .add_dataset(DatasetCreateRequest {
            account_id: account.id.clone(),
            name: "Remove dataset".to_owned(),
            instruction: "Keep all calendars".to_owned(),
            config: CalDavDatasetConfigView::default().into(),
            schedule_minutes: None,
            projections,
            enabled: true,
        })
        .await
        .unwrap();
    let removed_path =
        moltis_connectors::projection_directory(&export_root, &removed.name, &removed.id).unwrap();
    tokio::fs::create_dir_all(&removed_path).await.unwrap();
    manager.remove_dataset(&removed.id).await.unwrap();
    assert!(!removed_path.exists());

    let cascaded = manager
        .add_dataset(DatasetCreateRequest {
            account_id: account.id.clone(),
            name: "Remove account".to_owned(),
            instruction: "Keep all calendars".to_owned(),
            config: CalDavDatasetConfigView::default().into(),
            schedule_minutes: None,
            projections,
            enabled: true,
        })
        .await
        .unwrap();
    let cascaded_path =
        moltis_connectors::projection_directory(&export_root, &cascaded.name, &cascaded.id)
            .unwrap();
    tokio::fs::create_dir_all(&cascaded_path).await.unwrap();
    manager.remove_account(&account.id).await.unwrap();
    assert!(!cascaded_path.exists());
}

#[tokio::test]
#[cfg_attr(feature = "vault", serial_test::serial(vault_runtime))]
async fn scheduled_sync_rechecks_account_eligibility_when_claimed() {
    let (_temp, manager) = manager().await;
    let account = manager.add_account(create_request()).await.unwrap();
    let dataset = manager
        .add_dataset(DatasetCreateRequest {
            account_id: account.id.clone(),
            name: "Scheduled".to_owned(),
            instruction: "Keep all calendars".to_owned(),
            config: CalDavDatasetConfigView::default().into(),
            schedule_minutes: Some(1),
            projections: ProjectionConfig::default(),
            enabled: true,
        })
        .await
        .unwrap();
    manager
        .update_account(&account.id, AccountUpdateRequest {
            name: account.name,
            server_url: account.server_url,
            username: account.username,
            password: None,
            timeout_seconds: account.timeout_seconds,
            allow_insecure_http: account.allow_insecure_http,
            allow_private_network: account.allow_private_network,
            enabled: false,
        })
        .await
        .unwrap();
    let due_at = OffsetDateTime::now_utc();
    manager
        .store
        .set_dataset_next_sync_at(&dataset.id, Some(due_at))
        .await
        .unwrap();
    let manager = Arc::new(manager);

    let result = manager
        .spawn_sync(&dataset.id, SyncTrigger::Scheduled { due_at })
        .await
        .unwrap();
    assert!(result.is_none());
    assert!(
        manager
            .store
            .list_recent_sync_runs(&dataset.id, 1)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
#[cfg_attr(feature = "vault", serial_test::serial(vault_runtime))]
async fn manual_sync_rejects_disabled_datasets_and_accounts() {
    let (_temp, manager) = manager().await;
    let account = manager.add_account(create_request()).await.unwrap();
    let dataset = manager
        .add_dataset(DatasetCreateRequest {
            account_id: account.id.clone(),
            name: "Disabled".to_owned(),
            instruction: "Keep all calendars".to_owned(),
            config: CalDavDatasetConfigView::default().into(),
            schedule_minutes: None,
            projections: ProjectionConfig::default(),
            enabled: false,
        })
        .await
        .unwrap();
    let manager = Arc::new(manager);

    let dataset_error = manager.sync_dataset(&dataset.id).await.unwrap_err();
    assert!(matches!(
        dataset_error,
        ConnectorManagerError::InvalidInput(_)
    ));
    manager
        .update_dataset(&dataset.id, DatasetUpdateRequest {
            name: dataset.name,
            instruction: dataset.instruction.unwrap(),
            config: dataset.config,
            schedule_minutes: dataset.schedule_minutes,
            projections: dataset.projections,
            enabled: true,
        })
        .await
        .unwrap();
    manager
        .update_account(&account.id, AccountUpdateRequest {
            name: account.name,
            server_url: account.server_url,
            username: account.username,
            password: None,
            timeout_seconds: account.timeout_seconds,
            allow_insecure_http: account.allow_insecure_http,
            allow_private_network: account.allow_private_network,
            enabled: false,
        })
        .await
        .unwrap();

    let account_error = manager.sync_dataset(&dataset.id).await.unwrap_err();
    assert!(matches!(
        account_error,
        ConnectorManagerError::InvalidInput(_)
    ));
    assert!(
        manager
            .store
            .list_recent_sync_runs(&dataset.id, 1)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
#[cfg_attr(feature = "vault", serial_test::serial(vault_runtime))]
async fn syncs_are_serialized_per_account() {
    let (_temp, manager) = manager().await;
    let account = manager.add_account(create_request()).await.unwrap();
    let dataset = manager
        .add_dataset(DatasetCreateRequest {
            account_id: account.id.clone(),
            name: "Second dataset".to_owned(),
            instruction: "Keep all calendars".to_owned(),
            config: CalDavDatasetConfigView::default().into(),
            schedule_minutes: None,
            projections: ProjectionConfig::default(),
            enabled: true,
        })
        .await
        .unwrap();
    manager.active_account_ids.insert(account.id);
    let manager = Arc::new(manager);

    let error = manager.sync_dataset(&dataset.id).await.unwrap_err();
    assert!(matches!(error, ConnectorManagerError::Conflict(ref id) if id == &dataset.id));
    assert!(
        manager
            .store
            .list_recent_sync_runs(&dataset.id, 1)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
#[cfg_attr(feature = "vault", serial_test::serial(vault_runtime))]
async fn shutdown_closes_sync_admission() {
    let (_temp, manager) = manager().await;
    let account = manager.add_account(create_request()).await.unwrap();
    let dataset = manager
        .add_dataset(DatasetCreateRequest {
            account_id: account.id,
            name: "Shutdown".to_owned(),
            instruction: "Keep all calendars".to_owned(),
            config: CalDavDatasetConfigView::default().into(),
            schedule_minutes: None,
            projections: ProjectionConfig::default(),
            enabled: true,
        })
        .await
        .unwrap();
    let manager = Arc::new(manager);
    manager.shutdown().await;

    let error = manager.sync_dataset(&dataset.id).await.unwrap_err();
    assert!(matches!(error, ConnectorManagerError::Unavailable(_)));
    assert!(
        manager
            .store
            .list_recent_sync_runs(&dataset.id, 1)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
#[cfg_attr(feature = "vault", serial_test::serial(vault_runtime))]
async fn reconciliation_rebuilds_a_projection_missing_after_successful_sync() {
    let (temp, manager) = manager().await;
    let account = manager.add_account(create_request()).await.unwrap();
    let dataset = manager
        .add_dataset(DatasetCreateRequest {
            account_id: account.id,
            name: "Recover projection".to_owned(),
            instruction: "Keep all calendars".to_owned(),
            config: CalDavDatasetConfigView::default().into(),
            schedule_minutes: None,
            projections: ProjectionConfig {
                jsonl: true,
                markdown: true,
            },
            enabled: true,
        })
        .await
        .unwrap();
    let run = manager.store.start_sync_run(&dataset.id).await.unwrap();
    manager
        .store
        .commit_sync_snapshot(
            &run.id,
            &dataset.id,
            &[moltis_connectors::ConnectorItemInput {
                remote_id: "event-1".to_owned(),
                kind: "event".to_owned(),
                remote_version: Some("etag-1".to_owned()),
                occurred_at: None,
                updated_at: None,
                body_json: serde_json::json!({"summary": "Recovered"}),
                search_text: "Recovered".to_owned(),
                content_hash: "hash-1".to_owned(),
            }],
            &[moltis_connectors::SourceObservation {
                remote_id: "event-1".to_owned(),
                remote_version: Some("etag-1".to_owned()),
                disposition: moltis_connectors::SourceDisposition::Included,
                filter_reason: None,
                evaluated_plan_revision: 1,
            }],
            None,
        )
        .await
        .unwrap();
    let projection_path = moltis_connectors::projection_directory(
        &temp.path().join("connectors").join("exports"),
        &dataset.name,
        &dataset.id,
    )
    .unwrap();
    assert!(!projection_path.exists());

    manager.reconcile_projection_directories().await.unwrap();
    assert!(projection_path.join("manifest.json").exists());
    assert!(projection_path.join("items.jsonl").exists());
    let manifest: ProjectionManifest = serde_json::from_slice(
        &tokio::fs::read(projection_path.join("manifest.json"))
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(manifest.source_plan_revision, Some(1));

    tokio::fs::remove_file(projection_path.join("items.jsonl"))
        .await
        .unwrap();
    manager.reconcile_projection_directories().await.unwrap();
    assert!(projection_path.join("items.jsonl").exists());

    manager
        .update_dataset(&dataset.id, DatasetUpdateRequest {
            name: dataset.name,
            instruction: "Keep all calendars".to_owned(),
            config: CalDavDatasetConfigView::default().into(),
            schedule_minutes: None,
            projections: ProjectionConfig {
                jsonl: false,
                markdown: true,
            },
            enabled: true,
        })
        .await
        .unwrap();
    assert!(!projection_path.join("items.jsonl").exists());
    assert!(
        projection_path
            .join("items")
            .read_dir()
            .unwrap()
            .next()
            .is_some()
    );
}

#[tokio::test]
#[cfg_attr(feature = "vault", serial_test::serial(vault_runtime))]
async fn search_get_and_sync_revisions_remain_dataset_scoped() {
    let (_temp, manager) = manager().await;
    let account = manager.add_account(create_request()).await.unwrap();
    let dataset = manager
        .add_dataset(DatasetCreateRequest {
            account_id: account.id.clone(),
            name: "Searchable".to_owned(),
            instruction: "Keep planning events".to_owned(),
            config: CalDavDatasetConfigView::default().into(),
            schedule_minutes: None,
            projections: ProjectionConfig::default(),
            enabled: true,
        })
        .await
        .unwrap();
    let other = manager
        .add_dataset(DatasetCreateRequest {
            account_id: account.id,
            name: "Other".to_owned(),
            instruction: "Keep other events".to_owned(),
            config: CalDavDatasetConfigView::default().into(),
            schedule_minutes: None,
            projections: ProjectionConfig::default(),
            enabled: true,
        })
        .await
        .unwrap();

    let stored = manager.dataset_required(&dataset.id).await.unwrap();
    assert_eq!(stored.plan_revision, 1);
    assert_eq!(stored.synced_plan_revision, None);
    let run = manager.store.start_sync_run(&dataset.id).await.unwrap();
    manager
        .store
        .commit_sync_snapshot(
            &run.id,
            &dataset.id,
            &[
                item("planning", "Quarterly Planning Needle"),
                item("lunch", "Team Lunch"),
            ],
            &[
                observation(
                    "planning",
                    moltis_connectors::SourceDisposition::Included,
                    1,
                ),
                observation("lunch", moltis_connectors::SourceDisposition::Included, 1),
            ],
            None,
        )
        .await
        .unwrap();

    let synced = manager
        .list_datasets()
        .await
        .unwrap()
        .into_iter()
        .find(|candidate| candidate.id == dataset.id)
        .unwrap();
    assert!(!synced.needs_sync);
    let lunch = manager
        .query_items(&dataset.id, ItemQueryRequest::default())
        .await
        .unwrap()
        .into_iter()
        .find(|candidate| candidate.remote_id == "lunch")
        .unwrap();
    let matches = manager
        .query_items(&dataset.id, ItemQueryRequest {
            text: Some("needle".to_owned()),
            ..ItemQueryRequest::default()
        })
        .await
        .unwrap();
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].remote_id, "planning");
    assert!(
        !serde_json::to_value(&matches[0])
            .unwrap()
            .as_object()
            .unwrap()
            .contains_key("searchText")
    );
    assert_eq!(
        manager
            .get_item(&dataset.id, &matches[0].id, false)
            .await
            .unwrap()
            .remote_id,
        "planning"
    );
    assert!(matches!(
        manager.get_item(&other.id, &matches[0].id, true).await,
        Err(ConnectorManagerError::NotFound { .. })
    ));

    let updated = manager
        .update_dataset(&dataset.id, DatasetUpdateRequest {
            name: dataset.name,
            instruction: "Keep accepted planning events".to_owned(),
            config: CalDavDatasetConfigView {
                filters: CalDavFiltersView {
                    accepted_by_account: true,
                    ..CalDavFiltersView::default()
                },
                ..CalDavDatasetConfigView::default()
            }
            .into(),
            schedule_minutes: None,
            projections: ProjectionConfig::default(),
            enabled: true,
        })
        .await
        .unwrap();
    assert!(updated.needs_sync);
    let stored = manager.dataset_required(&dataset.id).await.unwrap();
    assert_eq!(stored.plan_revision, 2);
    assert_eq!(stored.synced_plan_revision, Some(1));

    let run = manager.store.start_sync_run(&dataset.id).await.unwrap();
    manager
        .store
        .commit_sync_snapshot(
            &run.id,
            &dataset.id,
            &[item("planning", "Quarterly Planning Needle")],
            &[
                observation(
                    "planning",
                    moltis_connectors::SourceDisposition::Included,
                    2,
                ),
                observation("lunch", moltis_connectors::SourceDisposition::Filtered, 2),
            ],
            None,
        )
        .await
        .unwrap();
    let states = manager.store.source_states(&dataset.id).await.unwrap();
    assert_eq!(states["planning"].evaluated_plan_revision, 2);
    assert_eq!(
        states["lunch"].disposition,
        moltis_connectors::SourceDisposition::Filtered
    );
    assert!(matches!(
        manager.get_item(&dataset.id, &lunch.id, false).await,
        Err(ConnectorManagerError::NotFound { .. })
    ));
    assert_eq!(
        manager
            .get_item(&dataset.id, &lunch.id, true)
            .await
            .unwrap()
            .remote_id,
        "lunch"
    );
    let resynced = manager
        .list_datasets()
        .await
        .unwrap()
        .into_iter()
        .find(|candidate| candidate.id == dataset.id)
        .unwrap();
    assert!(!resynced.needs_sync);
}

#[cfg(feature = "vault")]
#[serial_test::serial(vault_runtime)]
#[tokio::test]
async fn enabled_vault_without_an_instance_never_stores_plaintext_passwords() {
    crate::vault_lifecycle::set_vault_encryption_runtime_enabled(true);
    let temp = tempfile::tempdir().unwrap();
    let manager = ConnectorManager::open(temp.path(), 1, None).await.unwrap();

    let error = manager.add_account(create_request()).await.unwrap_err();
    assert!(matches!(error, ConnectorManagerError::Unavailable(_)));
    assert!(manager.store.list_accounts().await.unwrap().is_empty());
    crate::vault_lifecycle::set_vault_encryption_runtime_enabled(false);
}

#[cfg(feature = "vault")]
#[serial_test::serial(vault_runtime)]
#[tokio::test]
async fn sealed_vault_preserves_encrypted_password_on_redacted_update() {
    crate::vault_lifecycle::set_vault_encryption_runtime_enabled(true);
    let vault_pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
    moltis_vault::run_migrations(&vault_pool).await.unwrap();
    let vault = Arc::new(moltis_vault::Vault::new(vault_pool).await.unwrap());
    let vault_password = test_password();
    vault.initialize(&vault_password).await.unwrap();

    let temp = tempfile::tempdir().unwrap();
    let manager = ConnectorManager::open(temp.path(), 1, Some(Arc::clone(&vault)))
        .await
        .unwrap();
    let created = manager.add_account(create_request()).await.unwrap();
    let before = manager
        .store
        .get_account(&created.id)
        .await
        .unwrap()
        .unwrap()
        .config["password"]
        .clone();
    assert!(before.is_object());
    vault.seal().await;

    manager
        .update_account(&created.id, AccountUpdateRequest {
            name: created.name,
            server_url: created.server_url,
            username: created.username,
            password: Some(Secret::new(REDACTED_PASSWORD.to_owned())),
            timeout_seconds: created.timeout_seconds,
            allow_insecure_http: created.allow_insecure_http,
            allow_private_network: created.allow_private_network,
            enabled: created.enabled,
        })
        .await
        .unwrap();
    let after = manager
        .store
        .get_account(&created.id)
        .await
        .unwrap()
        .unwrap()
        .config["password"]
        .clone();
    assert_eq!(after, before);
}
