use std::{net::IpAddr, path::PathBuf, time::Duration};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid repository source: {0}")]
    InvalidSource(String),
    #[error("invalid Git revision: {0}")]
    InvalidRevision(String),
    #[error("repository credential is bound to {actual}, not {expected}")]
    CredentialHostMismatch { expected: String, actual: String },
    #[error("private HTTPS repository requires credentials")]
    MissingHttpsCredentials,
    #[error("private SSH repository requires a private key and known_hosts pin")]
    MissingSshCredentials,
    #[error("failed to open Git repository at {}: {source}", path.display())]
    OpenRepository {
        path: PathBuf,
        #[source]
        source: Box<gix::open::Error>,
    },
    #[error("failed to resolve revision {revision}: {message}")]
    ResolveRevision { revision: String, message: String },
    #[error("resolved object for {revision} is not a commit")]
    NotACommit { revision: String },
    #[error("Git object materialization failed: {0}")]
    Object(String),
    #[error("repository exceeds materialization limit: {0}")]
    LimitExceeded(String),
    #[error(
        "repository has no supported explicit MCP manifest (.moltis/mcp-repository.json, .claude-plugin/marketplace.json, or .mcp.json)"
    )]
    NoSupportedManifest,
    #[error("invalid MCP repository manifest {}: {message}", path.display())]
    InvalidManifest { path: PathBuf, message: String },
    #[error("unsafe declared repository path: {0}")]
    UnsafeDeclaredPath(String),
    #[error("unsafe tree entry: {0}")]
    UnsafeTreeEntry(String),
    #[error("Git transport failed: {0}")]
    Transport(String),
    #[error("DNS resolution failed for {host}: {source}")]
    DnsResolution {
        host: String,
        #[source]
        source: std::io::Error,
    },
    #[error("repository host {host} resolves to prohibited address {address}")]
    ProhibitedNetworkAddress { host: String, address: IpAddr },
    #[error("Git subprocess failed: {0}")]
    GitSubprocess(String),
    #[error("Git fetch did not complete within {0:?}")]
    FetchTimeout(Duration),
    #[error("too many managed repository operations are already running")]
    OperationBusy,
    #[error("filesystem operation failed for {}: {source}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("published revision path escaped the revisions root")]
    PathEscape,
}

impl Error {
    pub(crate) fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }
}
