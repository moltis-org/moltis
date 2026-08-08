#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::{
        Arc, Barrier,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use {
    moltis_git_repositories::{
        Error as RepositoryError, HttpsCredentials, HttpsSource, RepositoryBackend,
        RequestedRevision, SshCredentials, SshSource,
    },
    moltis_mcp::{
        ManagedDiscoveryMode, ManagedRepository, ManagedRepositoryAccess, ManagedRepositoryAlias,
        ManagedRepositoryId, ManagedRepositorySource, McpManager, McpRegistry,
    },
    secrecy::Secret,
    serde_json::json,
    sqlx::SqlitePool,
    tokio::task,
};

use {
    super::{LiveMcpService, sanitize::expected_candidates},
    crate::auth::{CredentialStore, SshAuthMode},
};

#[derive(Clone)]
struct BlockingHttpsBackend {
    source: PathBuf,
    block: Arc<AtomicBool>,
    entered: Arc<Barrier>,
    release: Arc<Barrier>,
}

impl RepositoryBackend for BlockingHttpsBackend {
    fn fetch_https(
        &self,
        _source: &HttpsSource,
        _credentials: Option<&HttpsCredentials>,
        _revision: &RequestedRevision,
        destination: &Path,
        _max_fetch_bytes: u64,
        _max_fetch_duration: Duration,
    ) -> Result<(), RepositoryError> {
        if self.block.load(Ordering::SeqCst) {
            self.entered.wait();
            self.release.wait();
        }
        run_git(None, &["init", "--bare", "-q"], Some(destination))?;
        run_git(
            Some(destination),
            &[
                "fetch",
                "-q",
                self.source.to_string_lossy().as_ref(),
                "HEAD",
            ],
            None,
        )?;
        Ok(())
    }

    fn fetch_ssh(
        &self,
        _source: &SshSource,
        _credentials: &SshCredentials,
        _revision: &RequestedRevision,
        _destination: &Path,
        _max_fetch_bytes: u64,
        _max_fetch_duration: Duration,
    ) -> Result<(), RepositoryError> {
        Err(RepositoryError::Object("unexpected SSH fetch".into()))
    }
}

#[tokio::test]
async fn credential_delete_racing_install_cannot_remove_new_reference() {
    let data = tempfile::tempdir().unwrap();
    let source = source_repository();
    let store = Arc::new(
        CredentialStore::new(SqlitePool::connect("sqlite::memory:").await.unwrap())
            .await
            .unwrap(),
    );
    let credential_id = store
        .create_git_https_credential("git.example", "moltis", Secret::new("test-token".into()))
        .await
        .unwrap();
    let block = Arc::new(AtomicBool::new(false));
    let entered = Arc::new(Barrier::new(2));
    let release = Arc::new(Barrier::new(2));
    let backend = BlockingHttpsBackend {
        source: source.path().to_path_buf(),
        block: Arc::clone(&block),
        entered: Arc::clone(&entered),
        release: Arc::clone(&release),
    };
    let service = Arc::new(service(
        data.path(),
        Arc::clone(&store),
        moltis_git_repositories::Materializer::new(backend),
    ));
    let repository = json!({
        "id": "private-repo",
        "alias": "private",
        "source": { "kind": "https", "url": "https://git.example/repo.git", "private": true },
        "httpsCredentialId": credential_id,
    });
    let preview = service
        .repositories_preview_impl(repository.clone())
        .await
        .unwrap();
    block.store(true, Ordering::SeqCst);
    let mut install_params = repository;
    install_params["expectedCommit"] = preview["commit"].clone();
    install_params["selection"] = json!({
        "mode": "all",
        "candidates": expected_candidates(&preview),
    });
    let install_service = Arc::clone(&service);
    let install = tokio::spawn(async move {
        install_service
            .repositories_install_impl(install_params)
            .await
    });
    task::spawn_blocking(move || entered.wait()).await.unwrap();
    let delete_service = Arc::clone(&service);
    let delete = tokio::spawn(async move {
        delete_service
            .git_credentials_remove_impl(json!({ "id": credential_id }))
            .await
    });
    task::yield_now().await;
    task::spawn_blocking(move || release.wait()).await.unwrap();

    assert!(install.await.unwrap().is_ok());
    assert!(delete.await.unwrap().is_err());
    assert!(
        store
            .get_git_https_credential(credential_id)
            .await
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn referenced_ssh_target_and_key_delete_after_repository_removal() {
    let data = tempfile::tempdir().unwrap();
    let store = Arc::new(
        CredentialStore::new(SqlitePool::connect("sqlite::memory:").await.unwrap())
            .await
            .unwrap(),
    );
    let service = service(
        data.path(),
        Arc::clone(&store),
        moltis_git_repositories::Materializer::default(),
    );
    let key_id = store
        .create_ssh_key("git", "PRIVATE KEY", "ssh-ed25519 AAAATEST", "SHA256:test")
        .await
        .unwrap();
    let target_id = store
        .create_ssh_target(
            "git",
            "git.example",
            None,
            Some("git.example ssh-ed25519 AAAATEST"),
            SshAuthMode::Managed,
            Some(key_id),
            true,
        )
        .await
        .unwrap();
    let mut repository = ManagedRepository::new(
        ManagedRepositoryId::parse("ssh-repo").unwrap(),
        ManagedRepositoryAlias::parse("ssh-tools").unwrap(),
        ManagedRepositorySource::Ssh {
            remote: "git@git.example:owner/repo.git".into(),
            access: ManagedRepositoryAccess::Private,
        },
        "HEAD",
        ManagedDiscoveryMode::Explicit,
    );
    repository.ssh_target_id = Some(target_id);
    service
        .manager
        .inner
        .write()
        .await
        .registry
        .repositories
        .insert(repository.id.clone(), repository);

    assert!(
        service
            .managed_ssh_target_remove_impl(target_id)
            .await
            .is_err()
    );
    assert!(service.managed_ssh_key_remove_impl(key_id).await.is_err());
    service
        .repositories_remove_impl(json!({ "id": "ssh-repo" }))
        .await
        .unwrap();
    service
        .managed_ssh_target_remove_impl(target_id)
        .await
        .unwrap();
    service.managed_ssh_key_remove_impl(key_id).await.unwrap();
    assert!(store.list_ssh_targets().await.unwrap().is_empty());
    assert!(store.list_ssh_keys().await.unwrap().is_empty());
}

fn service(
    data_dir: &Path,
    store: Arc<CredentialStore>,
    materializer: moltis_git_repositories::Materializer,
) -> LiveMcpService {
    let registry = McpRegistry::load(&data_dir.join("mcp-servers.json")).unwrap();
    LiveMcpService::new_with_materializer(
        Arc::new(McpManager::new(registry)),
        Default::default(),
        Some(store),
        data_dir.to_path_buf(),
        materializer,
        moltis_mcp::ManagedRepositoryLock::try_acquire(data_dir).unwrap(),
    )
}

fn source_repository() -> tempfile::TempDir {
    let source = tempfile::tempdir().unwrap();
    run_git(
        Some(source.path()),
        &["init", "-q", "--initial-branch=main"],
        None,
    )
    .unwrap();
    fs::write(
        source.path().join(".mcp.json"),
        r#"{"mcpServers":{"one":{"command":"one"}}}"#,
    )
    .unwrap();
    run_git(Some(source.path()), &["add", ".mcp.json"], None).unwrap();
    run_git(
        Some(source.path()),
        &["commit", "-q", "-m", "initial"],
        None,
    )
    .unwrap();
    source
}

fn run_git(
    directory: Option<&Path>,
    args: &[&str],
    trailing_path: Option<&Path>,
) -> Result<(), RepositoryError> {
    let mut command = Command::new("git");
    command
        .args(["-c", "commit.gpgSign=false"])
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "Moltis Test")
        .env("GIT_AUTHOR_EMAIL", "test@moltis.invalid")
        .env("GIT_COMMITTER_NAME", "Moltis Test")
        .env("GIT_COMMITTER_EMAIL", "test@moltis.invalid");
    if let Some(directory) = directory {
        command.current_dir(directory);
    }
    if let Some(path) = trailing_path {
        command.arg(path);
    }
    let status = command
        .status()
        .map_err(|error| RepositoryError::Object(error.to_string()))?;
    if status.success() {
        Ok(())
    } else {
        Err(RepositoryError::Object("test Git command failed".into()))
    }
}
