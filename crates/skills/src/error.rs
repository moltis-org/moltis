use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Yaml(#[from] serde_yaml::Error),
    #[error(transparent)]
    Http(#[from] reqwest::Error),
    #[error(transparent)]
    Walk(#[from] walkdir::Error),
    #[error(transparent)]
    Join(#[from] tokio::task::JoinError),
    #[cfg(feature = "file-watcher")]
    #[error(transparent)]
    Notify(#[from] notify_debouncer_full::notify::Error),
    #[error("{0}")]
    Parse(String),
    #[error("{0}")]
    Install(String),
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    Validation(String),
    #[error("{0}")]
    Bundle(String),
}

pub type Result<T> = std::result::Result<T, Error>;
