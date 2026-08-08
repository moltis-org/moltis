//! Offline managed MCP repository commands.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    str::FromStr,
};

use {
    anyhow::{Context, anyhow, bail},
    clap::{Args, Subcommand, ValueEnum},
    moltis_auth::{CredentialStore, SshAuthMode},
    moltis_git_repositories::{
        Access, HttpsCredentials, HttpsSource, Materializer, RepositorySource, RequestedRevision,
        SshCredentials, SshSource,
    },
    moltis_mcp::{
        ManagedApprovalRequest, ManagedDiscoveryMode, ManagedInstallSelection,
        ManagedReconciliationResult, ManagedRepository, ManagedRepositoryAlias,
        ManagedRepositoryId, ManagedRepositoryLock, ManagedRepositoryPreview,
        ManagedRepositorySource, McpRegistry, discover_repository, preview_managed_repository,
    },
    secrecy::Secret,
    serde::Serialize,
    serde_json::{Value, json},
    sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions},
    tokio::task,
};

#[derive(Subcommand)]
pub enum McpAction {
    /// Manage MCP servers imported from Git repositories.
    Repositories {
        #[command(subcommand)]
        action: RepositoryAction,
    },
    /// Manage HTTPS Git credentials used by private repositories.
    Credentials {
        #[command(subcommand)]
        action: CredentialAction,
    },
}

#[derive(Subcommand)]
pub enum CredentialAction {
    /// List credential metadata without exposing tokens.
    List(OutputArgs),
    /// Create or replace a host-bound credential from an environment variable.
    Add(CredentialAddArgs),
    /// Remove an unused credential.
    Remove(CredentialRemoveArgs),
}

#[derive(Args, Clone)]
pub struct CredentialAddArgs {
    /// Git server host, without scheme or path.
    #[arg(long)]
    host: String,
    /// HTTPS Git username.
    #[arg(long)]
    username: String,
    /// Environment variable containing the token. Tokens are never accepted in argv.
    #[arg(long, default_value = "MOLTIS_GIT_TOKEN")]
    token_env: String,
    /// Emit machine-readable JSON.
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
pub struct CredentialRemoveArgs {
    #[arg(long)]
    id: i64,
    /// Emit machine-readable JSON.
    #[arg(long)]
    json: bool,
}

#[derive(Subcommand)]
pub enum RepositoryAction {
    /// List installed managed repositories.
    List(OutputArgs),
    /// Materialize and inspect a repository without installing it.
    Preview(RepositoryInput),
    /// Materialize and install a repository.
    Add(AddArgs),
    /// Preview or apply the latest revision of an installed repository.
    Update(UpdateArgs),
    /// Restore the previous immutable revision.
    Rollback(IdArgs),
    /// Remove an installed repository and its owned revisions.
    Remove(IdArgs),
    /// Approve exact server configurations from the active revision.
    Approve(ApproveArgs),
}

#[derive(Args, Clone)]
pub struct OutputArgs {
    /// Emit machine-readable JSON.
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
#[command(group(clap::ArgGroup::new("source").required(true).multiple(false).args(["url", "ssh", "local"])))]
pub struct SourceArgs {
    /// Public or private HTTPS Git URL.
    #[arg(long)]
    url: Option<String>,
    /// SSH Git remote.
    #[arg(long)]
    ssh: Option<String>,
    /// Existing local Git repository.
    #[arg(long)]
    local: Option<PathBuf>,
}

#[derive(Args, Clone)]
pub struct RepositoryInput {
    /// Stable display alias and default repository id.
    #[arg(long)]
    alias: String,
    #[command(flatten)]
    source: SourceArgs,
    /// Treat an HTTPS source as private.
    #[arg(long)]
    private: bool,
    /// Existing HTTPS Git credential id from moltis.db.
    #[arg(long)]
    https_credential_id: Option<i64>,
    /// Existing managed SSH target id from moltis.db.
    #[arg(long)]
    ssh_target_id: Option<i64>,
    /// Git revision to resolve.
    #[arg(long = "ref", default_value = "HEAD")]
    requested_ref: String,
    /// Stable repository id. Recommended for deployment scripts.
    #[arg(long)]
    id: Option<String>,
    /// Emit machine-readable JSON.
    #[arg(long)]
    json: bool,
}

#[derive(Clone, Copy, ValueEnum)]
enum AddApproval {
    All,
    None,
}

#[derive(Args, Clone)]
pub struct AddArgs {
    #[command(flatten)]
    repository: RepositoryInput,
    /// Approve every exact candidate, or leave all candidates unapproved.
    #[arg(long, value_enum, default_value = "none")]
    approve: AddApproval,
    /// Enable approved candidates. Requires --approve all.
    #[arg(long)]
    enable: bool,
}

#[derive(Args, Clone)]
pub struct UpdateArgs {
    #[arg(long)]
    id: String,
    /// Apply the computed reconciliation. Without this flag only a diff is printed.
    #[arg(long)]
    apply: bool,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
pub struct IdArgs {
    #[arg(long)]
    id: String,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
#[command(group(clap::ArgGroup::new("approval").required(true).multiple(false).args(["all", "server"])))]
pub struct ApproveArgs {
    #[arg(long)]
    id: String,
    /// Approve every server owned by the repository.
    #[arg(long)]
    all: bool,
    /// Approve one runtime server name. May be repeated.
    #[arg(long, action = clap::ArgAction::Append)]
    server: Vec<String>,
    /// Enable the approved servers in persisted state.
    #[arg(long)]
    enable: bool,
    #[arg(long)]
    json: bool,
}

#[derive(Clone)]
struct ResolvedRepository {
    id: ManagedRepositoryId,
    alias: ManagedRepositoryAlias,
    source: RepositorySource,
    managed_source: ManagedRepositorySource,
    revision: RequestedRevision,
    requested_ref: String,
    https_credential_id: Option<i64>,
    ssh_target_id: Option<i64>,
    https_credentials: Option<HttpsCredentials>,
    ssh_credentials: Option<SshCredentials>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CandidateOutput {
    runtime_name: String,
    identity: String,
    digest: String,
    transport: String,
    approved: bool,
    approval_blocked: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    approval_block_reason: Option<String>,
    enabled: bool,
    warnings: Vec<String>,
}

pub async fn handle_mcp(action: McpAction, data_dir: PathBuf) -> anyhow::Result<()> {
    let (value, json_output) = execute(action, data_dir).await?;
    if json_output {
        println!("{}", serde_json::to_string_pretty(&value)?);
    } else {
        print_human(&value);
    }
    Ok(())
}

async fn execute(action: McpAction, data_dir: PathBuf) -> anyhow::Result<(Value, bool)> {
    let action = match action {
        McpAction::Repositories { action } => ManagedAction::Repository(action),
        McpAction::Credentials { action } => ManagedAction::Credential(action),
    };
    let json_output = action.json_output();
    let lock_dir = data_dir.clone();
    let _lock = task::spawn_blocking(move || ManagedRepositoryLock::try_acquire(&lock_dir))
        .await
        .context("managed repository lock task failed")??;
    let value = match action {
        ManagedAction::Repository(RepositoryAction::List(_)) => list(&data_dir).await?,
        ManagedAction::Repository(RepositoryAction::Preview(input)) => {
            preview(&data_dir, input).await?
        },
        ManagedAction::Repository(RepositoryAction::Add(args)) => add(&data_dir, args).await?,
        ManagedAction::Repository(RepositoryAction::Update(args)) => {
            update(&data_dir, args).await?
        },
        ManagedAction::Repository(RepositoryAction::Rollback(args)) => {
            rollback(&data_dir, args.id).await?
        },
        ManagedAction::Repository(RepositoryAction::Remove(args)) => {
            remove(&data_dir, args.id).await?
        },
        ManagedAction::Repository(RepositoryAction::Approve(args)) => {
            approve(&data_dir, args).await?
        },
        ManagedAction::Credential(CredentialAction::List(_)) => credentials_list(&data_dir).await?,
        ManagedAction::Credential(CredentialAction::Add(args)) => {
            credentials_add(&data_dir, args).await?
        },
        ManagedAction::Credential(CredentialAction::Remove(args)) => {
            credentials_remove(&data_dir, args.id).await?
        },
    };
    Ok((value, json_output))
}

enum ManagedAction {
    Repository(RepositoryAction),
    Credential(CredentialAction),
}

impl ManagedAction {
    fn json_output(&self) -> bool {
        match self {
            Self::Repository(RepositoryAction::List(args)) => args.json,
            Self::Repository(RepositoryAction::Preview(args)) => args.json,
            Self::Repository(RepositoryAction::Add(args)) => args.repository.json,
            Self::Repository(RepositoryAction::Update(args)) => args.json,
            Self::Repository(RepositoryAction::Rollback(args))
            | Self::Repository(RepositoryAction::Remove(args)) => args.json,
            Self::Repository(RepositoryAction::Approve(args)) => args.json,
            Self::Credential(CredentialAction::List(args)) => args.json,
            Self::Credential(CredentialAction::Add(args)) => args.json,
            Self::Credential(CredentialAction::Remove(args)) => args.json,
        }
    }
}

async fn credentials_list(data_dir: &Path) -> anyhow::Result<Value> {
    let store = credential_store(data_dir).await?;
    Ok(json!({
        "credentials": store.list_git_https_credentials().await?,
    }))
}

async fn credentials_add(data_dir: &Path, args: CredentialAddArgs) -> anyhow::Result<Value> {
    let token = std::env::var(&args.token_env).with_context(|| {
        format!(
            "environment variable {} is not set; tokens are never accepted as command arguments",
            args.token_env
        )
    })?;
    if token.is_empty() {
        bail!("environment variable {} is empty", args.token_env);
    }
    persist_credential(data_dir, &args.host, &args.username, Secret::new(token)).await
}

async fn persist_credential(
    data_dir: &Path,
    host: &str,
    username: &str,
    token: Secret<String>,
) -> anyhow::Result<Value> {
    let store = credential_store_for_create(data_dir).await?;
    let existing = store
        .list_git_https_credentials()
        .await?
        .into_iter()
        .find(|entry| {
            entry.host.eq_ignore_ascii_case(host.trim()) && entry.username == username.trim()
        });
    let id = if let Some(existing) = existing {
        if existing.encrypted {
            bail!(
                "credential {} is vault-encrypted; offline provisioning refuses to replace it with plaintext; use moltis-ctl against an unsealed running gateway",
                existing.id
            );
        }
        store
            .update_git_https_credential(existing.id, host, username, token)
            .await?;
        existing.id
    } else {
        store
            .create_git_https_credential(host, username, token)
            .await?
    };
    let entry = store
        .list_git_https_credentials()
        .await?
        .into_iter()
        .find(|entry| entry.id == id)
        .ok_or_else(|| anyhow!("Git credential disappeared after persistence"))?;
    let storage_warning = (!entry.encrypted).then_some(
        "offline credential provisioning stores plaintext because the vault is not unlocked; use the authenticated running gateway when encryption at rest is required",
    );
    Ok(json!({
        "credential": entry,
        "storageWarning": storage_warning,
    }))
}

async fn credentials_remove(data_dir: &Path, id: i64) -> anyhow::Result<Value> {
    let registry = load_registry(data_dir).await?;
    let references = registry
        .repositories
        .values()
        .filter(|repository| repository.https_credential_id == Some(id))
        .count();
    let store = credential_store(data_dir).await?;
    store.delete_git_https_credential(id, references).await?;
    Ok(json!({ "id": id, "removed": true }))
}

async fn list(data_dir: &Path) -> anyhow::Result<Value> {
    let registry = load_registry(data_dir).await?;
    let repositories = registry
        .repositories
        .values()
        .map(|repository| repository_output(&registry, repository))
        .collect::<Vec<_>>();
    Ok(json!({ "repositories": repositories, "restartRequired": false }))
}

async fn preview(data_dir: &Path, input: RepositoryInput) -> anyhow::Result<Value> {
    let resolved = resolve_input(data_dir, &input).await?;
    let (preview, _temporary_revision) = materialize_ephemeral_preview(&resolved).await?;
    Ok(preview_output(&resolved, &preview, false))
}

async fn add(data_dir: &Path, args: AddArgs) -> anyhow::Result<Value> {
    if args.enable && !matches!(args.approve, AddApproval::All) {
        bail!("--enable requires --approve all");
    }
    let resolved = resolve_input(data_dir, &args.repository).await?;
    let mut registry = load_registry(data_dir).await?;
    if let Some(existing) = collision(&registry, &resolved)? {
        return Ok(json!({
            "status": "alreadyInstalled",
            "repository": repository_output(&registry, existing),
            "reconciliation": ManagedReconciliationResult::default(),
            "restartRequired": false,
        }));
    }

    let preview = match materialize_owned_preview(data_dir, &resolved).await {
        Ok(preview) => preview,
        Err(error) => {
            prune_owned_revisions(data_dir, &resolved.id).await?;
            return Err(error);
        },
    };
    let mut repository = ManagedRepository::new(
        resolved.id.clone(),
        resolved.alias.clone(),
        resolved.managed_source.clone(),
        resolved.requested_ref.clone(),
        ManagedDiscoveryMode::Explicit,
    );
    repository.https_credential_id = resolved.https_credential_id;
    repository.ssh_target_id = resolved.ssh_target_id;
    let install_preview = preview.clone();
    let reconciliation = match persist_registry(registry, move |registry| {
        registry.install_managed_repository(
            repository,
            &install_preview,
            ManagedInstallSelection::All,
        )
    })
    .await
    {
        Ok(reconciliation) => reconciliation,
        Err(error) => {
            prune_owned_revisions(data_dir, &resolved.id).await?;
            return Err(error);
        },
    };
    prune_owned_revisions(data_dir, &resolved.id).await?;

    if matches!(args.approve, AddApproval::All) {
        registry = load_registry(data_dir).await?;
        let requests = approval_requests(&registry, &resolved.id, None)?;
        let id = resolved.id.clone();
        let commit = preview.revision.commit.clone();
        persist_registry(registry, move |registry| {
            registry.approve_managed_all(&id, &commit, &requests, args.enable)
        })
        .await?;
    }
    let registry = load_registry(data_dir).await?;
    let installed = registry
        .repositories
        .get(&resolved.id)
        .ok_or_else(|| anyhow!("managed repository disappeared after installation"))?;
    Ok(json!({
        "status": "installed",
        "repository": repository_output(&registry, installed),
        "commit": preview.revision.commit,
        "candidates": candidate_outputs(&preview, Some(&registry)),
        "warnings": warning_outputs(&preview),
        "reconciliation": reconciliation,
        "restartRequired": true,
    }))
}

async fn update(data_dir: &Path, args: UpdateArgs) -> anyhow::Result<Value> {
    let id = ManagedRepositoryId::parse(args.id)?;
    let registry = load_registry(data_dir).await?;
    let repository = registry
        .repositories
        .get(&id)
        .cloned()
        .ok_or_else(|| anyhow!("managed repository '{id}' not found"))?;
    let resolved = resolve_installed(data_dir, &repository).await?;
    let preview = match materialize_owned_preview(data_dir, &resolved).await {
        Ok(preview) => preview,
        Err(error) => {
            prune_owned_revisions(data_dir, &id).await?;
            return Err(error);
        },
    };
    let diff = reconciliation_preview(&repository, &preview, &registry);
    if !args.apply {
        let mut output = preview_output(&resolved, &preview, false);
        output["reconciliation"] = serde_json::to_value(diff)?;
        prune_owned_revisions(data_dir, &id).await?;
        return Ok(output);
    }
    let update_id = id.clone();
    let reconciliation = match persist_registry(registry, move |registry| {
        registry.update_managed_repository(&update_id, &preview)
    })
    .await
    {
        Ok(reconciliation) => reconciliation,
        Err(error) => {
            prune_owned_revisions(data_dir, &id).await?;
            return Err(error);
        },
    };
    prune_owned_revisions(data_dir, &id).await?;
    let registry = load_registry(data_dir).await?;
    let installed = registry
        .repositories
        .get(&id)
        .ok_or_else(|| anyhow!("managed repository disappeared after update"))?;
    Ok(json!({
        "repository": repository_output(&registry, installed),
        "commit": installed.active.as_ref().map(|revision| &revision.commit),
        "reconciliation": reconciliation,
        "restartRequired": true,
    }))
}

async fn rollback(data_dir: &Path, id: String) -> anyhow::Result<Value> {
    let id = ManagedRepositoryId::parse(id)?;
    let registry = load_registry(data_dir).await?;
    let repository = registry
        .repositories
        .get(&id)
        .cloned()
        .ok_or_else(|| anyhow!("managed repository '{id}' not found"))?;
    let previous = repository
        .previous
        .clone()
        .ok_or_else(|| anyhow!("managed repository '{id}' has no previous revision"))?;
    ensure_owned_revision(data_dir, &id, &previous.path).await?;
    let previous_commit = previous.commit.clone();
    let preview = discover_preview(
        previous.path,
        previous.commit,
        id.clone(),
        repository.alias.clone(),
    )
    .await?;
    let rollback_id = id.clone();
    let reconciliation = persist_registry(registry, move |registry| {
        registry.rollback_managed_repository(&rollback_id, &preview)
    })
    .await?;
    Ok(json!({
        "id": id,
        "commit": previous_commit,
        "reconciliation": reconciliation,
        "restartRequired": true,
    }))
}

async fn remove(data_dir: &Path, id: String) -> anyhow::Result<Value> {
    let id = ManagedRepositoryId::parse(id)?;
    let registry = load_registry(data_dir).await?;
    if !registry.repositories.contains_key(&id) {
        bail!("managed repository '{id}' not found");
    }
    let remove_id = id.clone();
    let removed = persist_registry(registry, move |registry| {
        registry.remove_managed_repository(&remove_id)
    })
    .await?;
    if removed {
        remove_owned_directory(data_dir, &id).await?;
    }
    Ok(json!({ "id": id, "removed": removed, "restartRequired": removed }))
}

async fn approve(data_dir: &Path, args: ApproveArgs) -> anyhow::Result<Value> {
    let id = ManagedRepositoryId::parse(args.id)?;
    let registry = load_registry(data_dir).await?;
    let repository = registry
        .repositories
        .get(&id)
        .ok_or_else(|| anyhow!("managed repository '{id}' not found"))?;
    let commit = repository
        .active
        .as_ref()
        .ok_or_else(|| anyhow!("managed repository '{id}' has no active revision"))?
        .commit
        .clone();
    let selected = (!args.all).then_some(args.server.as_slice());
    let requests = approval_requests(&registry, &id, selected)?;
    let names = requests
        .iter()
        .map(|request| request.runtime_name.clone())
        .collect::<Vec<_>>();
    let approve_id = id.clone();
    persist_registry(registry, move |registry| {
        if args.all {
            registry.approve_managed_all(&approve_id, &commit, &requests, args.enable)
        } else {
            registry.approve_managed_selected(&approve_id, &commit, &requests, args.enable)
        }
    })
    .await?;
    Ok(json!({
        "id": id,
        "approved": names,
        "enabled": args.enable,
        "restartRequired": true,
    }))
}

async fn resolve_input(
    data_dir: &Path,
    input: &RepositoryInput,
) -> anyhow::Result<ResolvedRepository> {
    let id = ManagedRepositoryId::parse(input.id.clone().unwrap_or_else(|| input.alias.clone()))?;
    let alias = ManagedRepositoryAlias::parse(input.alias.clone())?;
    let revision = RequestedRevision::parse(&input.requested_ref)?;
    let requested_ref = revision.as_str().to_string();
    match (&input.source.url, &input.source.ssh, &input.source.local) {
        (Some(url), None, None) => {
            if input.ssh_target_id.is_some() {
                bail!("HTTPS repository cannot specify --ssh-target-id");
            }
            if !input.private && input.https_credential_id.is_some() {
                bail!("public HTTPS repository cannot specify --https-credential-id");
            }
            if input.private && input.https_credential_id.is_none() {
                bail!("private HTTPS repository requires --https-credential-id");
            }
            let access = if input.private {
                Access::Private
            } else {
                Access::Public
            };
            let source = HttpsSource::new(url, access)?;
            let credentials = match input.https_credential_id {
                Some(credential_id) => {
                    let store = credential_store(data_dir).await?;
                    reject_encrypted_credential(data_dir, "git_https_credentials", credential_id)
                        .await?;
                    let credential = store.get_git_https_credential(credential_id).await.map_err(|error| {
                        anyhow!("failed to resolve HTTPS Git credential {credential_id}: {error}. If it is vault-encrypted, use moltis-ctl against an unsealed running gateway")
                    })?.ok_or_else(|| anyhow!("HTTPS Git credential {credential_id} not found in {}", data_dir.join("moltis.db").display()))?;
                    Some(HttpsCredentials::new(
                        credential.host,
                        credential.username,
                        credential.token,
                    )?)
                },
                None => None,
            };
            if let Some(credentials) = &credentials
                && !credentials.host().eq_ignore_ascii_case(source.authority())
            {
                bail!("HTTPS credential host does not match repository source");
            }
            Ok(ResolvedRepository {
                id,
                alias,
                managed_source: ManagedRepositorySource::Https {
                    url: source.url().as_str().to_string(),
                    access,
                },
                source: RepositorySource::Https(source),
                revision,
                requested_ref,
                https_credential_id: input.https_credential_id,
                ssh_target_id: None,
                https_credentials: credentials,
                ssh_credentials: None,
            })
        },
        (None, Some(remote), None) => {
            if input.private {
                bail!("--private is only valid with --url");
            }
            if input.https_credential_id.is_some() {
                bail!("SSH repository cannot specify --https-credential-id");
            }
            let target_id = input
                .ssh_target_id
                .ok_or_else(|| anyhow!("SSH repository requires --ssh-target-id"))?;
            let source = SshSource::new(remote, Access::Private)?;
            let store = credential_store(data_dir).await?;
            let target = store
                .resolve_ssh_target_by_id(target_id)
                .await?
                .ok_or_else(|| {
                    anyhow!(
                        "managed SSH target {target_id} not found in {}",
                        data_dir.join("moltis.db").display()
                    )
                })?;
            if target.auth_mode != SshAuthMode::Managed {
                bail!("repository SSH target must use managed authentication");
            }
            let key_id = target
                .key_id
                .ok_or_else(|| anyhow!("managed SSH target has no key configured"))?;
            let known_host = target.known_host.filter(|pin| !pin.trim().is_empty()).ok_or_else(|| {
                anyhow!("managed SSH target has no confirmed known_host; confirm a strict host pin in Settings before deployment")
            })?;
            if !ssh_target_authority(&target.target, target.port)?
                .eq_ignore_ascii_case(source.host())
            {
                bail!("managed SSH target host/port does not match repository source");
            }
            reject_encrypted_credential(data_dir, "ssh_keys", key_id).await?;
            let private_key = store.get_ssh_private_key(key_id).await.map_err(|error| {
                anyhow!("failed to resolve managed SSH key {key_id}: {error}. If it is vault-encrypted, use moltis-ctl against an unsealed running gateway")
            })?.ok_or_else(|| anyhow!("managed SSH key {key_id} not found"))?;
            let credentials =
                SshCredentials::new(source.host(), private_key, Secret::new(known_host))?;
            Ok(ResolvedRepository {
                id,
                alias,
                managed_source: ManagedRepositorySource::Ssh {
                    remote: source.remote().to_string(),
                    access: Access::Private,
                },
                source: RepositorySource::Ssh(source),
                revision,
                requested_ref,
                https_credential_id: None,
                ssh_target_id: Some(target_id),
                https_credentials: None,
                ssh_credentials: Some(credentials),
            })
        },
        (None, None, Some(path)) => {
            if input.private || input.https_credential_id.is_some() || input.ssh_target_id.is_some()
            {
                bail!("local repository cannot specify private access or credentials");
            }
            let path = path.clone();
            let source = task::spawn_blocking(move || RepositorySource::local(path)).await??;
            let canonical_path = match &source {
                RepositorySource::Local(local) => local.path().to_path_buf(),
                _ => return Err(anyhow!("invalid local repository source")),
            };
            Ok(ResolvedRepository {
                id,
                alias,
                source,
                managed_source: ManagedRepositorySource::Local { canonical_path },
                revision,
                requested_ref,
                https_credential_id: None,
                ssh_target_id: None,
                https_credentials: None,
                ssh_credentials: None,
            })
        },
        _ => bail!("exactly one of --url, --ssh, or --local is required"),
    }
}

async fn resolve_installed(
    data_dir: &Path,
    repository: &ManagedRepository,
) -> anyhow::Result<ResolvedRepository> {
    let source = match &repository.source {
        ManagedRepositorySource::Https { url, .. } => SourceArgs {
            url: Some(url.clone()),
            ssh: None,
            local: None,
        },
        ManagedRepositorySource::Ssh { remote, .. } => SourceArgs {
            url: None,
            ssh: Some(remote.clone()),
            local: None,
        },
        ManagedRepositorySource::Local { canonical_path } => SourceArgs {
            url: None,
            ssh: None,
            local: Some(canonical_path.clone()),
        },
    };
    resolve_input(data_dir, &RepositoryInput {
        alias: repository.alias.as_str().to_string(),
        source,
        private: matches!(&repository.source, ManagedRepositorySource::Https {
            access: Access::Private,
            ..
        }),
        https_credential_id: repository.https_credential_id,
        ssh_target_id: repository.ssh_target_id,
        requested_ref: repository.requested_ref.clone(),
        id: Some(repository.id.to_string()),
        json: true,
    })
    .await
}

async fn credential_store(data_dir: &Path) -> anyhow::Result<CredentialStore> {
    let db_path = data_dir.join("moltis.db");
    if !db_path.is_file() {
        bail!(
            "credential database {} does not exist; preprovision credentials in the web UI or use a public/local repository",
            db_path.display()
        );
    }
    let options = SqliteConnectOptions::from_str(&format!("sqlite:{}", db_path.display()))?
        .create_if_missing(false)
        .foreign_keys(true)
        .busy_timeout(std::time::Duration::from_secs(5));
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .with_context(|| format!("failed to open credential database {}", db_path.display()))?;
    moltis_gateway::run_migrations(&pool)
        .await
        .context("failed to run gateway credential migrations")?;
    CredentialStore::new(pool)
        .await
        .context("failed to open credential store")
}

async fn credential_store_for_create(data_dir: &Path) -> anyhow::Result<CredentialStore> {
    std::fs::create_dir_all(data_dir).with_context(|| {
        format!(
            "failed to create Moltis data directory {}",
            data_dir.display()
        )
    })?;
    let db_path = data_dir.join("moltis.db");
    let options = SqliteConnectOptions::from_str(&format!("sqlite:{}", db_path.display()))?
        .create_if_missing(true)
        .foreign_keys(true)
        .busy_timeout(std::time::Duration::from_secs(5));
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .with_context(|| format!("failed to open credential database {}", db_path.display()))?;
    moltis_gateway::run_migrations(&pool)
        .await
        .context("failed to run gateway credential migrations")?;
    CredentialStore::new(pool)
        .await
        .context("failed to open credential store")
}

async fn reject_encrypted_credential(
    data_dir: &Path,
    table: &'static str,
    id: i64,
) -> anyhow::Result<()> {
    let db_path = data_dir.join("moltis.db");
    let options = SqliteConnectOptions::from_str(&format!("sqlite:{}", db_path.display()))?
        .create_if_missing(false)
        .busy_timeout(std::time::Duration::from_secs(5));
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await?;
    let query = match table {
        "git_https_credentials" => {
            "SELECT COALESCE(encrypted, 0) FROM git_https_credentials WHERE id = ?"
        },
        "ssh_keys" => "SELECT COALESCE(encrypted, 0) FROM ssh_keys WHERE id = ?",
        _ => bail!("unsupported credential table"),
    };
    let encrypted: Option<(i64,)> = sqlx::query_as(query).bind(id).fetch_optional(&pool).await?;
    if encrypted.is_some_and(|(encrypted,)| encrypted != 0) {
        bail!(
            "credential {id} is vault-encrypted; use moltis-ctl against an unsealed running gateway"
        );
    }
    Ok(())
}

async fn materialize_ephemeral_preview(
    resolved: &ResolvedRepository,
) -> anyhow::Result<(ManagedRepositoryPreview, tempfile::TempDir)> {
    let temporary_revision = tempfile::Builder::new()
        .prefix("moltis-mcp-preview-")
        .tempdir()
        .context("failed to create repository preview directory")?;
    let preview =
        materialize_preview_at(resolved, temporary_revision.path().join("revisions")).await?;
    Ok((preview, temporary_revision))
}

async fn materialize_owned_preview(
    data_dir: &Path,
    resolved: &ResolvedRepository,
) -> anyhow::Result<ManagedRepositoryPreview> {
    materialize_preview_at(resolved, revisions_root(data_dir, &resolved.id)).await
}

async fn materialize_preview_at(
    resolved: &ResolvedRepository,
    revisions_root: PathBuf,
) -> anyhow::Result<ManagedRepositoryPreview> {
    let materializer = Materializer::default();
    let source = resolved.source.clone();
    let revision = resolved.revision.clone();
    let https_credentials = resolved.https_credentials.clone();
    let ssh_credentials = resolved.ssh_credentials.clone();
    let materialized = task::spawn_blocking(move || {
        materializer.materialize(
            &source,
            &revision,
            &revisions_root,
            https_credentials.as_ref(),
            ssh_credentials.as_ref(),
        )
    })
    .await
    .context("repository materialization task failed")??;
    discover_preview(
        materialized.path,
        materialized.commit,
        resolved.id.clone(),
        resolved.alias.clone(),
    )
    .await
}

async fn discover_preview(
    path: PathBuf,
    commit: String,
    id: ManagedRepositoryId,
    alias: ManagedRepositoryAlias,
) -> anyhow::Result<ManagedRepositoryPreview> {
    task::spawn_blocking(move || {
        let discovery = discover_repository(&path)?;
        preview_managed_repository(&discovery, id, alias, &commit, &path)
    })
    .await
    .context("repository discovery task failed")?
    .map_err(Into::into)
}

async fn load_registry(data_dir: &Path) -> anyhow::Result<McpRegistry> {
    let path = data_dir.join("mcp-servers.json");
    task::spawn_blocking(move || McpRegistry::load(&path))
        .await
        .context("MCP registry load task failed")?
        .map_err(Into::into)
}

async fn persist_registry<T, F>(mut registry: McpRegistry, operation: F) -> anyhow::Result<T>
where
    T: Send + 'static,
    F: FnOnce(&mut McpRegistry) -> moltis_mcp::Result<T> + Send + 'static,
{
    task::spawn_blocking(move || operation(&mut registry))
        .await
        .context("MCP registry persistence task failed")?
        .map_err(Into::into)
}

fn collision<'a>(
    registry: &'a McpRegistry,
    resolved: &ResolvedRepository,
) -> anyhow::Result<Option<&'a ManagedRepository>> {
    let matches = registry
        .repositories
        .values()
        .filter(|repository| repository.id == resolved.id || repository.alias == resolved.alias)
        .collect::<Vec<_>>();
    if matches.is_empty() {
        return Ok(None);
    }
    if matches.len() == 1
        && matches[0].source == resolved.managed_source
        && matches[0].requested_ref == resolved.requested_ref
    {
        return Ok(Some(matches[0]));
    }
    bail!(
        "repository id '{}' or alias '{}' is already installed with a conflicting source or ref",
        resolved.id,
        resolved.alias.as_str()
    )
}

fn approval_requests(
    registry: &McpRegistry,
    id: &ManagedRepositoryId,
    selected: Option<&[String]>,
) -> anyhow::Result<Vec<ManagedApprovalRequest>> {
    let mut owned = registry
        .servers
        .iter()
        .filter_map(|(name, config)| {
            config.managed_origin.as_ref().and_then(|origin| {
                (&origin.repository_id == id).then(|| ManagedApprovalRequest {
                    runtime_name: name.clone(),
                    config_digest: origin.config_digest.clone(),
                })
            })
        })
        .collect::<Vec<_>>();
    owned.sort_by(|left, right| left.runtime_name.cmp(&right.runtime_name));
    let Some(selected) = selected else {
        return Ok(owned);
    };
    if selected.is_empty() {
        bail!("at least one --server is required unless --all is used");
    }
    let by_name = owned
        .into_iter()
        .map(|request| (request.runtime_name.clone(), request))
        .collect::<BTreeMap<_, _>>();
    selected
        .iter()
        .map(|name| {
            by_name.get(name).cloned().ok_or_else(|| {
                anyhow!("managed MCP server '{name}' is not owned by repository '{id}'")
            })
        })
        .collect()
}

fn reconciliation_preview(
    repository: &ManagedRepository,
    preview: &ManagedRepositoryPreview,
    registry: &McpRegistry,
) -> ManagedReconciliationResult {
    let old = registry
        .servers
        .iter()
        .filter_map(|(name, config)| {
            config.managed_origin.as_ref().and_then(|origin| {
                (origin.repository_id == repository.id).then(|| {
                    (
                        origin.identity.clone(),
                        (name.clone(), origin.config_digest.clone()),
                    )
                })
            })
        })
        .collect::<BTreeMap<_, _>>();
    let new = preview
        .candidates
        .iter()
        .filter_map(|candidate| {
            candidate.config.managed_origin.as_ref().map(|origin| {
                (
                    candidate.identity.clone(),
                    (candidate.runtime_name.clone(), origin.config_digest.clone()),
                )
            })
        })
        .collect::<BTreeMap<_, _>>();
    let mut result = ManagedReconciliationResult::default();
    for (identity, (name, digest)) in &new {
        match old.get(identity) {
            None => result.added.push(name.clone()),
            Some((old_name, old_digest)) if old_digest == digest => {
                result.unchanged.push(old_name.clone())
            },
            Some((old_name, _)) => result.updated.push(old_name.clone()),
        }
    }
    for (identity, (name, _)) in old {
        if !new.contains_key(&identity) {
            result.removed.push(name);
        }
    }
    result
}

async fn ensure_owned_revision(
    data_dir: &Path,
    id: &ManagedRepositoryId,
    revision: &Path,
) -> anyhow::Result<()> {
    let root = revisions_root(data_dir, id);
    let revision = revision.to_path_buf();
    task::spawn_blocking(move || {
        let canonical_root = root
            .canonicalize()
            .context("managed repository revisions are unavailable")?;
        let canonical_revision = revision
            .canonicalize()
            .context("managed repository revision is unavailable")?;
        let name = canonical_revision
            .file_name()
            .ok_or_else(|| anyhow!("managed repository revision is invalid"))?;
        if canonical_revision != canonical_root.join(name) {
            bail!("managed repository revision escaped its owned directory");
        }
        Ok(())
    })
    .await
    .context("repository path validation task failed")?
}

async fn remove_owned_directory(data_dir: &Path, id: &ManagedRepositoryId) -> anyhow::Result<()> {
    let root = data_dir.join("mcp-repositories");
    let directory = root.join(id.as_str());
    let id = id.as_str().to_string();
    task::spawn_blocking(move || {
        if !directory.exists() {
            return Ok(());
        }
        let canonical_root = root.canonicalize()?;
        let canonical_directory = directory.canonicalize()?;
        if canonical_directory != canonical_root.join(id) {
            bail!("managed repository directory escaped its owned root");
        }
        std::fs::remove_dir_all(canonical_directory)?;
        Ok(())
    })
    .await
    .context("repository cleanup task failed")?
}

fn revisions_root(data_dir: &Path, id: &ManagedRepositoryId) -> PathBuf {
    data_dir
        .join("mcp-repositories")
        .join(id.as_str())
        .join("revisions")
}

async fn prune_owned_revisions(data_dir: &Path, id: &ManagedRepositoryId) -> anyhow::Result<()> {
    let registry = load_registry(data_dir).await?;
    let retained = registry
        .repositories
        .get(id)
        .into_iter()
        .flat_map(|repository| [repository.active.as_ref(), repository.previous.as_ref()])
        .flatten()
        .map(|revision| revision.commit.clone())
        .collect::<std::collections::BTreeSet<_>>();
    let root = revisions_root(data_dir, id);
    let repositories_root = data_dir.join("mcp-repositories");
    let repository_name = id.as_str().to_string();
    task::spawn_blocking(move || -> anyhow::Result<()> {
        if !root.exists() {
            return Ok(());
        }
        let canonical_repositories = repositories_root.canonicalize()?;
        let canonical_root = root.canonicalize()?;
        if canonical_root
            != canonical_repositories
                .join(repository_name)
                .join("revisions")
        {
            bail!("managed repository revisions escaped their owned directory");
        }
        for entry in std::fs::read_dir(&canonical_root)? {
            let entry = entry?;
            if retained.contains(&entry.file_name().to_string_lossy().into_owned()) {
                continue;
            }
            let file_type = entry.file_type()?;
            if file_type.is_dir() && !file_type.is_symlink() {
                std::fs::remove_dir_all(entry.path())?;
            } else {
                std::fs::remove_file(entry.path())?;
            }
        }
        Ok(())
    })
    .await
    .context("repository revision cleanup task failed")??;
    Ok(())
}

fn repository_output(registry: &McpRegistry, repository: &ManagedRepository) -> Value {
    let servers = registry
        .servers
        .iter()
        .filter_map(|(name, config)| {
            config.managed_origin.as_ref().and_then(|origin| {
                (origin.repository_id == repository.id).then(|| {
                    json!({
                        "runtimeName": name,
                        "identity": origin.identity,
                        "digest": origin.config_digest,
                        "approved": origin.is_currently_approved(),
                        "enabled": config.enabled,
                    })
                })
            })
        })
        .collect::<Vec<_>>();
    json!({
        "id": repository.id,
        "alias": repository.alias.as_str(),
        "source": source_output(&repository.source, repository.https_credential_id, repository.ssh_target_id),
        "ref": repository.requested_ref,
        "commit": repository.active.as_ref().map(|revision| &revision.commit),
        "previousCommit": repository.previous.as_ref().map(|revision| &revision.commit),
        "servers": servers,
    })
}

fn source_output(
    source: &ManagedRepositorySource,
    credential_id: Option<i64>,
    target_id: Option<i64>,
) -> Value {
    match source {
        ManagedRepositorySource::Https { url, access } => json!({
            "kind": "https", "url": moltis_mcp::remote::sanitize_url_for_display(url),
            "private": *access == Access::Private, "httpsCredentialId": credential_id,
        }),
        ManagedRepositorySource::Ssh { remote, .. } => json!({
            "kind": "ssh", "remote": remote, "sshTargetId": target_id,
        }),
        ManagedRepositorySource::Local { canonical_path } => json!({
            "kind": "local", "path": canonical_path,
        }),
    }
}

fn preview_output(
    resolved: &ResolvedRepository,
    preview: &ManagedRepositoryPreview,
    restart_required: bool,
) -> Value {
    json!({
        "repository": {
            "id": resolved.id,
            "alias": resolved.alias.as_str(),
            "source": source_output(&resolved.managed_source, resolved.https_credential_id, resolved.ssh_target_id),
            "ref": resolved.requested_ref,
        },
        "commit": preview.revision.commit,
        "candidates": candidate_outputs(preview, None),
        "warnings": warning_outputs(preview),
        "restartRequired": restart_required,
    })
}

fn candidate_outputs(
    preview: &ManagedRepositoryPreview,
    registry: Option<&McpRegistry>,
) -> Vec<CandidateOutput> {
    preview
        .candidates
        .iter()
        .map(|candidate| {
            let current =
                registry.and_then(|registry| registry.servers.get(&candidate.runtime_name));
            let origin = current
                .and_then(|config| config.managed_origin.as_ref())
                .or(candidate.config.managed_origin.as_ref());
            let approval_block_reason = origin
                .and_then(moltis_mcp::ManagedOrigin::approval_block_reason)
                .map(String::from);
            CandidateOutput {
                runtime_name: candidate.runtime_name.clone(),
                identity: candidate.identity.as_str().to_string(),
                digest: origin
                    .map(|origin| origin.config_digest.clone())
                    .unwrap_or_default(),
                transport: candidate.config.transport.to_string(),
                approved: origin.is_some_and(moltis_mcp::ManagedOrigin::is_currently_approved),
                approval_blocked: approval_block_reason.is_some(),
                approval_block_reason,
                enabled: current.is_some_and(|config| config.enabled),
                warnings: origin
                    .into_iter()
                    .flat_map(|origin| &origin.warnings)
                    .map(|warning| warning.kind.as_str().to_string())
                    .collect(),
            }
        })
        .collect()
}

fn warning_outputs(preview: &ManagedRepositoryPreview) -> Vec<Value> {
    preview
        .warnings
        .iter()
        .map(|warning| {
            json!({
                "kind": warning.kind.as_str(),
                "sourceManifestPath": warning.source_manifest_path,
                "pluginName": warning.plugin_name,
                "sourceName": warning.source_name,
            })
        })
        .collect()
}

fn ssh_target_authority(target: &str, port: Option<u16>) -> anyhow::Result<String> {
    let host = target
        .trim()
        .rsplit_once('@')
        .map_or(target.trim(), |(_, host)| host)
        .trim_matches(['[', ']'])
        .to_ascii_lowercase();
    if host.is_empty()
        || !host
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b".:-".contains(&byte))
    {
        bail!("managed SSH target has an invalid host");
    }
    Ok(match port {
        Some(port) => format!("[{host}]:{port}"),
        None if host.contains(':') => format!("[{host}]"),
        None => host,
    })
}

fn print_human(value: &Value) {
    if let Some(repositories) = value.get("repositories").and_then(Value::as_array) {
        if repositories.is_empty() {
            println!("No managed MCP repositories installed.");
        } else {
            for repository in repositories {
                println!(
                    "{}\t{}\t{}",
                    repository["id"].as_str().unwrap_or_default(),
                    repository["alias"].as_str().unwrap_or_default(),
                    repository["commit"].as_str().unwrap_or("pending")
                );
            }
        }
        return;
    }
    if let Some(repository) = value.get("repository") {
        let repository = repository.get("id").and_then(Value::as_str).or_else(|| {
            repository
                .get("repository")
                .and_then(|value| value.get("id"))
                .and_then(Value::as_str)
        });
        if let Some(id) = repository {
            println!("Repository: {id}");
        }
    } else if let Some(id) = value.get("id").and_then(Value::as_str) {
        println!("Repository: {id}");
    }
    if let Some(status) = value.get("status").and_then(Value::as_str) {
        println!("Status: {status}");
    }
    if let Some(commit) = value.get("commit").and_then(Value::as_str) {
        println!("Commit: {commit}");
    }
    if let Some(candidates) = value.get("candidates").and_then(Value::as_array) {
        println!("Candidates: {}", candidates.len());
        for candidate in candidates {
            println!(
                "  {} (approved: {}, enabled: {})",
                candidate["runtimeName"].as_str().unwrap_or_default(),
                candidate["approved"].as_bool().unwrap_or(false),
                candidate["enabled"].as_bool().unwrap_or(false)
            );
        }
    }
    if value.get("restartRequired").and_then(Value::as_bool) == Some(true) {
        println!("Gateway restart required.");
    }
}

#[cfg(test)]
#[path = "mcp_commands_tests.rs"]
mod tests;
