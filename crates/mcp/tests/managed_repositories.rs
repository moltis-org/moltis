#![allow(clippy::unwrap_used)]

use std::{collections::BTreeSet, fs, path::Path};

use {
    moltis_mcp::{
        ManagedApprovalRequest, ManagedDiscoveryMode, ManagedInstallSelection, ManagedRepository,
        ManagedRepositoryAccess, ManagedRepositoryAlias, ManagedRepositoryId,
        ManagedRepositoryLock, ManagedRepositoryPreview, ManagedRepositorySource,
        ManagedWarningKind, McpRegistry, discover_repository, preview_managed_repository,
    },
    secrecy::{ExposeSecret, Secret},
};

const COMMIT_ONE: &str = "1111111111111111111111111111111111111111";
const COMMIT_TWO: &str = "2222222222222222222222222222222222222222";

fn write_manifest(root: &Path, servers: &str) {
    fs::write(
        root.join(".mcp.json"),
        format!(r#"{{"mcpServers":{{{servers}}}}}"#),
    )
    .unwrap();
}

fn preview(root: &Path, commit: &str) -> ManagedRepositoryPreview {
    let discovery = discover_repository(root).unwrap();
    preview_managed_repository(
        &discovery,
        ManagedRepositoryId::parse("repo-1").unwrap(),
        ManagedRepositoryAlias::parse("tools").unwrap(),
        commit,
        root,
    )
    .unwrap()
}

fn repository(_source: &Path) -> ManagedRepository {
    ManagedRepository::new(
        ManagedRepositoryId::parse("repo-1").unwrap(),
        ManagedRepositoryAlias::parse("tools").unwrap(),
        ManagedRepositorySource::Https {
            url: "https://example.com/owner/repository.git".into(),
            access: ManagedRepositoryAccess::Public,
        },
        "main",
        ManagedDiscoveryMode::Explicit,
    )
}

fn test_registry() -> (tempfile::TempDir, McpRegistry) {
    let directory = tempfile::tempdir().unwrap();
    let registry = McpRegistry::load(&directory.path().join("mcp.json")).unwrap();
    (directory, registry)
}

#[test]
fn managed_repository_lock_is_exclusive_for_guard_lifetime() {
    let data = tempfile::tempdir().unwrap();
    let first = ManagedRepositoryLock::try_acquire(data.path()).unwrap();
    assert!(ManagedRepositoryLock::try_acquire(data.path()).is_err());
    drop(first);
    assert!(ManagedRepositoryLock::try_acquire(data.path()).is_ok());
}

fn approval_requests(preview: &ManagedRepositoryPreview) -> Vec<ManagedApprovalRequest> {
    preview
        .candidates
        .iter()
        .map(|candidate| ManagedApprovalRequest {
            runtime_name: candidate.runtime_name.clone(),
            config_digest: candidate
                .config
                .managed_origin
                .as_ref()
                .unwrap()
                .config_digest
                .clone(),
        })
        .collect()
}

#[test]
fn names_are_deterministic_valid_and_identity_collisions_do_not_renumber() {
    let root = tempfile::tempdir().unwrap();
    write_manifest(
        root.path(),
        r#""Same Name":{"command":"node","args":["one.js"]},"same-name":{"command":"node","args":["two.js"]}"#,
    );
    let first = preview(root.path(), COMMIT_ONE);
    let second = preview(root.path(), COMMIT_ONE);

    assert_eq!(
        first
            .candidates
            .iter()
            .map(|candidate| &candidate.runtime_name)
            .collect::<Vec<_>>(),
        second
            .candidates
            .iter()
            .map(|candidate| &candidate.runtime_name)
            .collect::<Vec<_>>()
    );
    assert_ne!(
        first.candidates[0].runtime_name,
        first.candidates[1].runtime_name
    );
    assert!(first.candidates.iter().all(|candidate| {
        candidate.runtime_name.len() <= 80
            && candidate
                .runtime_name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
    }));
}

#[test]
fn preview_rejects_cwd_outside_materialized_revision() {
    let root = tempfile::tempdir().unwrap();
    write_manifest(root.path(), r#""one":{"command":"one"}"#);
    let mut discovery = discover_repository(root.path()).unwrap();
    discovery.servers[0].config.cwd = Some(tempfile::tempdir().unwrap().path().to_path_buf());
    assert!(
        preview_managed_repository(
            &discovery,
            ManagedRepositoryId::parse("repo-1").unwrap(),
            ManagedRepositoryAlias::parse("tools").unwrap(),
            COMMIT_ONE,
            root.path(),
        )
        .is_err()
    );
}

#[test]
fn install_selected_and_all_are_atomic_and_never_overwrite_manual_servers() {
    let root = tempfile::tempdir().unwrap();
    write_manifest(
        root.path(),
        r#""one":{"command":"one"},"two":{"command":"two"}"#,
    );
    let preview = preview(root.path(), COMMIT_ONE);
    let (_storage, mut registry) = test_registry();
    let collision = preview.candidates[0].runtime_name.clone();
    registry
        .servers
        .insert(collision.clone(), moltis_mcp::McpServerConfig {
            command: "manual".into(),
            ..Default::default()
        });
    assert!(
        registry
            .install_managed_repository(
                repository(root.path()),
                &preview,
                ManagedInstallSelection::All,
            )
            .is_err()
    );
    assert_eq!(registry.servers[&collision].command, "manual");
    assert!(registry.repositories.is_empty());

    registry.servers.remove(&collision);
    let selected = BTreeSet::from([preview.candidates[0].identity.clone()]);
    registry
        .install_managed_repository(
            repository(root.path()),
            &preview,
            ManagedInstallSelection::Selected(selected),
        )
        .unwrap();
    assert_eq!(registry.servers.len(), 1);
    assert!(!registry.servers.values().next().unwrap().enabled);

    let (_storage, mut all_registry) = test_registry();
    all_registry
        .install_managed_repository(
            repository(root.path()),
            &preview,
            ManagedInstallSelection::All,
        )
        .unwrap();
    assert_eq!(all_registry.servers.len(), 2);
}

#[test]
fn approval_is_race_safe_and_managed_enable_requires_current_approval() {
    let root = tempfile::tempdir().unwrap();
    write_manifest(root.path(), r#""one":{"command":"one"}"#);
    let preview = preview(root.path(), COMMIT_ONE);
    let requests = approval_requests(&preview);
    let name = requests[0].runtime_name.clone();
    let (_storage, mut registry) = test_registry();
    registry
        .install_managed_repository(
            repository(root.path()),
            &preview,
            ManagedInstallSelection::All,
        )
        .unwrap();

    assert!(registry.enable(&name).is_err());
    let mut stale = requests.clone();
    stale[0].config_digest = "stale".into();
    assert!(
        registry
            .approve_managed_selected(
                &ManagedRepositoryId::parse("repo-1").unwrap(),
                COMMIT_ONE,
                &stale,
                true,
            )
            .is_err()
    );
    assert!(!registry.servers[&name].enabled);
    registry
        .approve_managed_all(
            &ManagedRepositoryId::parse("repo-1").unwrap(),
            COMMIT_ONE,
            &requests,
            true,
        )
        .unwrap();
    assert!(registry.servers[&name].enabled);
    assert!(registry.enable(&name).unwrap());
}

#[test]
fn environment_placeholders_in_all_managed_fields_block_approval_and_enable() {
    let manifests = [
        r#""one":{"command":"${OPENAI_API_KEY}"}"#,
        r#""one":{"command":"runner","args":["${OPENAI_API_KEY}"]}"#,
        r#""one":{"command":"runner","env":{"TOKEN":"${OPENAI_API_KEY}"}}"#,
        r#""one":{"type":"http","url":"https://example.test/${OPENAI_API_KEY}"}"#,
        r#""one":{"type":"http","url":"https://example.test","headers":{"Authorization":"Bearer ${env:OPENAI_API_KEY}"}}"#,
        r#""one":{"command":"runner","args":["$OPENAI_API_KEY"]}"#,
    ];

    for manifest in manifests {
        let root = tempfile::tempdir().unwrap();
        write_manifest(root.path(), manifest);
        let preview = preview(root.path(), COMMIT_ONE);
        let origin = preview.candidates[0]
            .config
            .managed_origin
            .as_ref()
            .unwrap();
        assert!(
            origin.warnings.iter().any(|warning| {
                warning.kind == ManagedWarningKind::UnboundEnvironmentPlaceholder
            })
        );
        let requests = approval_requests(&preview);
        let name = requests[0].runtime_name.clone();
        let (_storage, mut registry) = test_registry();
        registry
            .install_managed_repository(
                repository(root.path()),
                &preview,
                ManagedInstallSelection::All,
            )
            .unwrap();

        let error = registry
            .approve_managed_all(
                &ManagedRepositoryId::parse("repo-1").unwrap(),
                COMMIT_ONE,
                &requests,
                true,
            )
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("unbound environment placeholder")
        );
        assert!(registry.enable(&name).is_err());
        assert!(!registry.servers[&name].enabled);
    }
}

#[test]
fn literal_secret_looking_value_blocks_approval_but_benign_manifest_works() {
    let root = tempfile::tempdir().unwrap();
    write_manifest(
        root.path(),
        r#""one":{"command":"runner","env":{"API_TOKEN":"literal-token"}}"#,
    );
    let blocked = preview(root.path(), COMMIT_ONE);
    let requests = approval_requests(&blocked);
    let (_storage, mut registry) = test_registry();
    registry
        .install_managed_repository(
            repository(root.path()),
            &blocked,
            ManagedInstallSelection::All,
        )
        .unwrap();
    let error = registry
        .approve_managed_all(
            &ManagedRepositoryId::parse("repo-1").unwrap(),
            COMMIT_ONE,
            &requests,
            false,
        )
        .unwrap_err();
    assert!(error.to_string().contains("literal secret-looking value"));

    let benign_root = tempfile::tempdir().unwrap();
    write_manifest(
        benign_root.path(),
        r#""one":{"command":"runner","args":["--mode","safe"],"env":{"LOG_LEVEL":"info"}}"#,
    );
    let benign = preview(benign_root.path(), COMMIT_ONE);
    let benign_requests = approval_requests(&benign);
    let (_storage, mut benign_registry) = test_registry();
    benign_registry
        .install_managed_repository(
            repository(benign_root.path()),
            &benign,
            ManagedInstallSelection::All,
        )
        .unwrap();
    benign_registry
        .approve_managed_all(
            &ManagedRepositoryId::parse("repo-1").unwrap(),
            COMMIT_ONE,
            &benign_requests,
            true,
        )
        .unwrap();
    assert!(benign_registry.servers.values().next().unwrap().enabled);
}

#[test]
fn common_literal_token_shapes_in_command_args_and_url_block_approval() {
    for manifest in [
        r#""one":{"command":"sk-production-token"}"#,
        r#""one":{"command":"runner","args":["ghp_literalTokenValue"]}"#,
        r#""one":{"type":"http","url":"https://example.test/github_pat_literalTokenValue"}"#,
    ] {
        let root = tempfile::tempdir().unwrap();
        write_manifest(root.path(), manifest);
        let preview = preview(root.path(), COMMIT_ONE);
        let requests = approval_requests(&preview);
        let (_storage, mut registry) = test_registry();
        registry
            .install_managed_repository(
                repository(root.path()),
                &preview,
                ManagedInstallSelection::All,
            )
            .unwrap();
        assert!(
            registry
                .approve_managed_all(
                    &ManagedRepositoryId::parse("repo-1").unwrap(),
                    COMMIT_ONE,
                    &requests,
                    false,
                )
                .is_err()
        );
    }
}

#[test]
fn update_invalidates_approval_and_reconciles_changed_added_and_removed() {
    let first_root = tempfile::tempdir().unwrap();
    write_manifest(
        first_root.path(),
        r#""same":{"command":"same"},"changed":{"command":"old"},"removed":{"command":"gone"}"#,
    );
    let first = preview(first_root.path(), COMMIT_ONE);
    let (_storage, mut registry) = test_registry();
    registry
        .install_managed_repository(
            repository(first_root.path()),
            &first,
            ManagedInstallSelection::All,
        )
        .unwrap();
    registry
        .approve_managed_all(
            &ManagedRepositoryId::parse("repo-1").unwrap(),
            COMMIT_ONE,
            &approval_requests(&first),
            true,
        )
        .unwrap();
    let changed_name = first
        .candidates
        .iter()
        .find(|candidate| candidate.config.command == "old")
        .unwrap()
        .runtime_name
        .clone();
    registry
        .update_managed_overlays(&changed_name, Some("Custom".into()), Some(90))
        .unwrap();

    let second_root = tempfile::tempdir().unwrap();
    write_manifest(
        second_root.path(),
        r#""same":{"command":"same"},"changed":{"command":"new"},"added":{"command":"fresh"}"#,
    );
    let second = preview(second_root.path(), COMMIT_TWO);
    let result = registry
        .update_managed_repository(&ManagedRepositoryId::parse("repo-1").unwrap(), &second)
        .unwrap();

    assert_eq!(result.added.len(), 1);
    assert_eq!(result.updated.len(), 2);
    assert_eq!(result.removed.len(), 1);
    assert!(registry.servers.values().all(|config| !config.enabled));
    assert!(registry.servers.values().all(|config| {
        config
            .managed_origin
            .as_ref()
            .is_some_and(|origin| origin.approval.is_none())
    }));
    assert_eq!(
        registry.servers[&changed_name].display_name.as_deref(),
        Some("Custom")
    );
    assert_eq!(
        registry.servers[&changed_name].request_timeout_secs,
        Some(90)
    );
}

#[test]
fn same_revision_preserves_approval_but_new_commit_invalidates_unchanged_manifest() {
    let first_root = tempfile::tempdir().unwrap();
    write_manifest(first_root.path(), r#""same":{"command":"same"}"#);
    let first = preview(first_root.path(), COMMIT_ONE);
    let (_storage, mut registry) = test_registry();
    registry
        .install_managed_repository(
            repository(first_root.path()),
            &first,
            ManagedInstallSelection::All,
        )
        .unwrap();
    registry
        .approve_managed_all(
            &ManagedRepositoryId::parse("repo-1").unwrap(),
            COMMIT_ONE,
            &approval_requests(&first),
            true,
        )
        .unwrap();

    let same = registry
        .update_managed_repository(&ManagedRepositoryId::parse("repo-1").unwrap(), &first)
        .unwrap();
    assert_eq!(same.unchanged.len(), 1);
    assert!(registry.servers.values().next().unwrap().enabled);

    let second_root = tempfile::tempdir().unwrap();
    write_manifest(second_root.path(), r#""same":{"command":"same"}"#);
    let second = preview(second_root.path(), COMMIT_TWO);
    let updated = registry
        .update_managed_repository(&ManagedRepositoryId::parse("repo-1").unwrap(), &second)
        .unwrap();
    assert_eq!(updated.updated.len(), 1);
    let config = registry.servers.values().next().unwrap();
    assert!(!config.enabled);
    assert!(config.managed_origin.as_ref().unwrap().approval.is_none());
}

#[test]
fn rollback_and_removal_obey_revision_and_ownership() {
    let first_root = tempfile::tempdir().unwrap();
    write_manifest(first_root.path(), r#""one":{"command":"old"}"#);
    let first = preview(first_root.path(), COMMIT_ONE);
    let second_root = tempfile::tempdir().unwrap();
    write_manifest(second_root.path(), r#""one":{"command":"new"}"#);
    let second = preview(second_root.path(), COMMIT_TWO);
    let (_storage, mut registry) = test_registry();
    registry
        .servers
        .insert("manual".into(), moltis_mcp::McpServerConfig {
            command: "keep".into(),
            ..Default::default()
        });
    registry
        .install_managed_repository(
            repository(first_root.path()),
            &first,
            ManagedInstallSelection::All,
        )
        .unwrap();
    registry
        .update_managed_repository(&ManagedRepositoryId::parse("repo-1").unwrap(), &second)
        .unwrap();
    registry
        .rollback_managed_repository(&ManagedRepositoryId::parse("repo-1").unwrap(), &first)
        .unwrap();
    assert!(
        registry
            .servers
            .values()
            .any(|config| config.command == "old")
    );
    assert!(
        registry
            .remove_managed_repository(&ManagedRepositoryId::parse("repo-1").unwrap())
            .unwrap()
    );
    assert_eq!(registry.servers.len(), 1);
    assert_eq!(registry.servers["manual"].command, "keep");
}

#[test]
fn persistence_is_backward_compatible_and_origin_never_contains_literal_secrets() {
    let root = tempfile::tempdir().unwrap();
    write_manifest(
        root.path(),
        r#""secret":{"command":"run","env":{"API_TOKEN":"literal-secret"},"args":["--token","argument-secret"]}"#,
    );
    let preview = preview(root.path(), COMMIT_ONE);
    let origin = preview.candidates[0]
        .config
        .managed_origin
        .as_ref()
        .unwrap();
    let serialized = serde_json::to_string(origin).unwrap();
    let debug = format!("{origin:?}");
    assert!(!serialized.contains("literal-secret"));
    assert!(!serialized.contains("argument-secret"));
    assert!(!debug.contains("literal-secret"));
    assert!(!debug.contains("argument-secret"));
    assert_eq!(origin.config_digest.len(), 64);

    write_manifest(
        root.path(),
        r#""secret":{"command":"run","env":{"API_TOKEN":"different-secret"},"args":["--token","argument-secret"]}"#,
    );
    let changed = preview_managed_repository(
        &discover_repository(root.path()).unwrap(),
        ManagedRepositoryId::parse("repo-1").unwrap(),
        ManagedRepositoryAlias::parse("tools").unwrap(),
        COMMIT_ONE,
        root.path(),
    )
    .unwrap();
    assert_ne!(
        origin.config_digest,
        changed.candidates[0]
            .config
            .managed_origin
            .as_ref()
            .unwrap()
            .config_digest
    );

    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("mcp.json");
    fs::write(&path, r#"{"legacy":{"command":"echo"}}"#).unwrap();
    let legacy = McpRegistry::load(&path).unwrap();
    assert!(legacy.repositories.is_empty());
    assert!(legacy.servers["legacy"].managed_origin.is_none());

    let (storage, mut registry) = test_registry();
    registry
        .install_managed_repository(
            repository(root.path()),
            &preview,
            ManagedInstallSelection::All,
        )
        .unwrap();
    let persisted_path = storage.path().join("mcp.json");
    let loaded = McpRegistry::load(&persisted_path).unwrap();
    assert_eq!(loaded.repositories.len(), 1);
    assert_eq!(
        loaded.servers.values().next().unwrap().env["API_TOKEN"].expose_secret(),
        "literal-secret"
    );
}

#[test]
fn managed_source_state_serializes_only_credential_references() {
    let mut repository = ManagedRepository::new(
        ManagedRepositoryId::parse("repo-1").unwrap(),
        ManagedRepositoryAlias::parse("tools").unwrap(),
        ManagedRepositorySource::Https {
            url: "https://example.com/owner/repo.git".into(),
            access: ManagedRepositoryAccess::Private,
        },
        "main",
        ManagedDiscoveryMode::Explicit,
    );
    repository.https_credential_id = Some(42);
    let serialized = serde_json::to_string(&repository).unwrap();
    assert!(serialized.contains("42"));
    assert!(!serialized.contains("password"));
    let secret = Secret::new("not-part-of-repository-state".to_string());
    assert!(!serialized.contains(secret.expose_secret()));
}

#[test]
fn unsupported_recursive_discovery_fails_before_registry_mutation() {
    let root = tempfile::tempdir().unwrap();
    write_manifest(root.path(), r#""one":{"command":"one"}"#);
    let preview = preview(root.path(), COMMIT_ONE);
    let mut repository = repository(root.path());
    repository.discovery_mode = ManagedDiscoveryMode::Recursive;
    let (_storage, mut registry) = test_registry();
    assert!(
        registry
            .install_managed_repository(repository, &preview, ManagedInstallSelection::All,)
            .is_err()
    );
    assert!(registry.servers.is_empty());
    assert!(registry.repositories.is_empty());
}
