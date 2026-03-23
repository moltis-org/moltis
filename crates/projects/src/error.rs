use std::{error::Error as StdError, path::PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Sqlx(#[from] sqlx::Error),
    #[error(transparent)]
    Migration(#[from] sqlx::migrate::MigrateError),
    #[error(transparent)]
    TomlDeserialize(#[from] toml::de::Error),
    #[error(transparent)]
    TomlSerialize(#[from] toml::ser::Error),
    #[error("failed to execute `{operation}`: {source}")]
    CommandExecution {
        operation: &'static str,
        #[source]
        source: std::io::Error,
    },
    #[error("`{operation}` failed: {stderr}")]
    CommandFailed {
        operation: &'static str,
        stderr: String,
    },
    #[error("{path} is not a git repository")]
    NotGitRepository { path: PathBuf },
    #[error("no default branch found (tried main, master, develop, trunk)")]
    DefaultBranchNotFound,
    #[error("{context}: {source}")]
    External {
        context: String,
        #[source]
        source: Box<dyn StdError + Send + Sync>,
    },
}

impl Error {
    #[must_use]
    pub fn command_execution(operation: &'static str, source: std::io::Error) -> Self {
        Self::CommandExecution { operation, source }
    }

    #[must_use]
    pub fn command_failed(operation: &'static str, stderr: impl Into<String>) -> Self {
        Self::CommandFailed {
            operation,
            stderr: stderr.into(),
        }
    }

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
