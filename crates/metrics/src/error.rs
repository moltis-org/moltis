use std::error::Error as StdError;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[cfg(feature = "sqlite")]
    #[error(transparent)]
    Sqlx(#[from] sqlx::Error),
    #[cfg(feature = "sqlite")]
    #[error(transparent)]
    SerdeJson(#[from] serde_json::Error),
    #[cfg(feature = "prometheus")]
    #[error(transparent)]
    Prometheus(#[from] metrics_exporter_prometheus::BuildError),
    #[error("{context}: {source}")]
    External {
        context: String,
        #[source]
        source: Box<dyn StdError + Send + Sync>,
    },
}

impl Error {
    #[must_use]
    pub fn external<E>(context: impl Into<String>, source: E) -> Self
    where
        E: StdError + Send + Sync + 'static,
    {
        Self::External {
            context: context.into(),
            source: Box::new(source),
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;
