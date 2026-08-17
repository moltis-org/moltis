use thiserror::Error;

pub type Result<T> = std::result::Result<T, HimalayaConnectorError>;

#[derive(Debug, Error)]
pub enum HimalayaConnectorError {
    #[error("invalid Himalaya account configuration: {0}")]
    AccountConfig(&'static str),
    #[error("invalid Himalaya dataset configuration: {0}")]
    DatasetConfig(&'static str),
    #[error("Himalaya executable was not found")]
    ExecutableNotFound,
    #[error("Himalaya v2 is required")]
    UnsupportedVersion,
    #[error("Himalaya command could not be started")]
    Spawn(#[source] std::io::Error),
    #[error("Himalaya command timed out")]
    Timeout,
    #[error("Himalaya command failed")]
    CommandFailed,
    #[error("Himalaya command output exceeded its safe limit")]
    OutputLimit,
    #[error("invalid Himalaya response: {0}")]
    Response(String),
    #[error("failed to encode Himalaya connector data")]
    Serialization(#[source] serde_json::Error),
}

impl From<serde_json::Error> for HimalayaConnectorError {
    fn from(error: serde_json::Error) -> Self {
        Self::Serialization(error)
    }
}
