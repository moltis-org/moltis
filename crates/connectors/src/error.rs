use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum ConnectorError {
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("migration error: {0}")]
    Migration(#[from] sqlx::migrate::MigrateError),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("filesystem error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("unknown connector kind: {0}")]
    UnknownConnectorKind(String),
    #[error("unknown sync run status: {0}")]
    UnknownSyncRunStatus(String),
    #[error("invalid stored timestamp: {0}")]
    InvalidTimestamp(i64),
    #[error("{entity} not found: {id}")]
    NotFound { entity: &'static str, id: String },
    #[error("query limit must be between 1 and {max}, got {requested}")]
    QueryLimit { requested: u64, max: u64 },
    #[error("invalid connector input: {0}")]
    InvalidInput(String),
}

pub type Result<T> = std::result::Result<T, ConnectorError>;

pub(crate) trait IoResultExt<T> {
    fn at(self, path: impl Into<PathBuf>) -> Result<T>;
}

impl<T> IoResultExt<T> for std::io::Result<T> {
    fn at(self, path: impl Into<PathBuf>) -> Result<T> {
        self.map_err(|source| ConnectorError::Io {
            path: path.into(),
            source,
        })
    }
}
