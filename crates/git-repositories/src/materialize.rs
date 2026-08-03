use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use crate::{
    Access, Error, HttpsCredentials, RepositoryBackend, RepositorySource, RequestedRevision,
    SshCredentials, SystemRepositoryBackend,
    transport::NetworkOperationGuard,
    tree::{canonicalize_published, resolve_commit, write_explicit_mcp_tree},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MaterializationLimits {
    /// Maximum number of selected entries, including directories and gitlinks.
    pub max_files: usize,
    /// Maximum recursion depth of a selected committed tree.
    pub max_tree_depth: usize,
    pub max_blob_bytes: u64,
    /// Maximum size of a manifest parsed during sparse content discovery.
    pub max_manifest_bytes: u64,
    pub max_total_bytes: u64,
    /// Maximum marketplace entries or native plugin roots inspected.
    pub max_plugin_roots: usize,
    /// Maximum manifest blobs inspected while selecting explicit MCP content.
    pub max_manifests: usize,
    /// Maximum bytes acquired into the temporary Git object database.
    pub max_fetch_bytes: u64,
    /// Maximum wall-clock duration of one network fetch.
    pub max_fetch_duration: Duration,
}

impl Default for MaterializationLimits {
    fn default() -> Self {
        Self {
            max_files: 100_000,
            max_tree_depth: 256,
            max_blob_bytes: 128 * 1024 * 1024,
            max_manifest_bytes: 1024 * 1024,
            max_total_bytes: 1024 * 1024 * 1024,
            max_plugin_roots: 256,
            max_manifests: 512,
            max_fetch_bytes: 512 * 1024 * 1024,
            max_fetch_duration: Duration::from_secs(120),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MaterializedRepository {
    pub path: PathBuf,
    pub commit: String,
}

#[derive(Clone)]
pub struct Materializer {
    backend: Arc<dyn RepositoryBackend>,
    limits: MaterializationLimits,
}

impl Default for Materializer {
    fn default() -> Self {
        Self::new(SystemRepositoryBackend::default())
    }
}

impl Materializer {
    pub fn new(backend: impl RepositoryBackend + 'static) -> Self {
        Self {
            backend: Arc::new(backend),
            limits: MaterializationLimits::default(),
        }
    }

    #[must_use]
    pub fn with_limits(mut self, limits: MaterializationLimits) -> Self {
        self.limits = limits;
        self
    }

    pub fn materialize(
        &self,
        source: &RepositorySource,
        revision: &RequestedRevision,
        revisions_root: &Path,
        https_credentials: Option<&HttpsCredentials>,
        ssh_credentials: Option<&SshCredentials>,
    ) -> Result<MaterializedRepository, Error> {
        #[cfg(feature = "tracing")]
        let _span = tracing::info_span!(
            "git_repository_materialize",
            source_kind = source.kind(),
            revision = revision.as_str()
        )
        .entered();
        #[cfg(feature = "metrics")]
        metrics::counter!("git_repository_materializations_total", "source" => source.kind())
            .increment(1);

        fs::create_dir_all(revisions_root).map_err(|error| Error::io(revisions_root, error))?;
        let root =
            fs::canonicalize(revisions_root).map_err(|error| Error::io(revisions_root, error))?;
        let staging = tempfile::Builder::new()
            .prefix(".materialize-")
            .tempdir_in(&root)
            .map_err(|error| Error::io(&root, error))?;
        let fetched = staging.path().join("repository.git");
        let tree = staging.path().join("tree");
        let network_operation = match source {
            RepositorySource::Local(_) => None,
            RepositorySource::Https(_) | RepositorySource::Ssh(_) => Some(
                NetworkOperationGuard::acquire(self.limits.max_fetch_duration)?,
            ),
        };

        let (repo, resolved_revision) = match source {
            RepositorySource::Local(source) => {
                (open_isolated(source.path())?, revision.as_str().to_string())
            },
            RepositorySource::Https(source) => {
                if source.access() == Access::Private && https_credentials.is_none() {
                    return Err(Error::MissingHttpsCredentials);
                }
                if let Some(credentials) = https_credentials
                    && !credentials.host().eq_ignore_ascii_case(source.authority())
                {
                    return Err(Error::CredentialHostMismatch {
                        expected: source.authority().to_string(),
                        actual: credentials.host().to_string(),
                    });
                }
                self.backend.fetch_https(
                    source,
                    https_credentials,
                    revision,
                    &fetched,
                    self.limits.max_fetch_bytes,
                    network_operation
                        .as_ref()
                        .ok_or(Error::FetchTimeout(self.limits.max_fetch_duration))?
                        .remaining()?,
                )?;
                (open_isolated(&fetched)?, "FETCH_HEAD".to_string())
            },
            RepositorySource::Ssh(source) => {
                let credentials = ssh_credentials.ok_or(Error::MissingSshCredentials)?;
                if !credentials.host().eq_ignore_ascii_case(source.host()) {
                    return Err(Error::CredentialHostMismatch {
                        expected: source.host().to_string(),
                        actual: credentials.host().to_string(),
                    });
                }
                self.backend.fetch_ssh(
                    source,
                    credentials,
                    revision,
                    &fetched,
                    self.limits.max_fetch_bytes,
                    network_operation
                        .as_ref()
                        .ok_or(Error::FetchTimeout(self.limits.max_fetch_duration))?
                        .remaining()?,
                )?;
                (open_isolated(&fetched)?, "FETCH_HEAD".to_string())
            },
        };

        let commit_id = resolve_commit(&repo, &resolved_revision)?;
        if let Some(operation) = network_operation.as_ref() {
            operation.check()?;
        }
        let commit = commit_id.to_string();
        if commit.len() != 40 {
            return Err(Error::Object("resolved commit is not SHA-1".into()));
        }
        let published = root.join(&commit);
        if published.exists() {
            return Ok(MaterializedRepository {
                path: canonicalize_published(&root, published)?,
                commit,
            });
        }

        fs::create_dir(&tree).map_err(|error| Error::io(&tree, error))?;
        write_explicit_mcp_tree(&repo, commit_id, &tree, self.limits)?;
        if let Some(operation) = network_operation.as_ref() {
            operation.check()?;
        }
        sync_directory(&tree)?;
        match fs::rename(&tree, &published) {
            Ok(()) => {},
            Err(_) if published.is_dir() => {},
            Err(error) => return Err(Error::io(&published, error)),
        }
        sync_directory(&root)?;
        Ok(MaterializedRepository {
            path: canonicalize_published(&root, published)?,
            commit,
        })
    }
}

fn open_isolated(path: &Path) -> Result<gix::Repository, Error> {
    gix::open_opts(path, gix::open::Options::isolated()).map_err(|source| Error::OpenRepository {
        path: path.to_path_buf(),
        source: Box::new(source),
    })
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> Result<(), Error> {
    let directory = fs::File::open(path).map_err(|error| Error::io(path, error))?;
    directory.sync_all().map_err(|error| Error::io(path, error))
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> Result<(), Error> {
    Ok(())
}
