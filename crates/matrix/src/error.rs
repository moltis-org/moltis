use std::error::Error as StdError;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("matrix SDK error: {0}")]
    Sdk(#[from] matrix_sdk::Error),

    #[error("matrix HTTP error: {0}")]
    Http(#[from] matrix_sdk::HttpError),

    #[error(transparent)]
    Channel(#[from] moltis_channels::Error),

    #[error(transparent)]
    SerdeJson(#[from] serde_json::Error),

    #[error("{message}")]
    Message { message: String },

    #[error("{context}: {source}")]
    External {
        context: String,
        #[source]
        source: Box<dyn StdError + Send + Sync>,
    },
}

impl Error {
    #[must_use]
    pub fn message(msg: impl std::fmt::Display) -> Self {
        Self::Message {
            message: msg.to_string(),
        }
    }

    #[must_use]
    pub fn external(
        context: impl Into<String>,
        source: impl StdError + Send + Sync + 'static,
    ) -> Self {
        Self::External {
            context: context.into(),
            source: Box::new(source),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_error_display() {
        let e = Error::message("test error");
        assert_eq!(e.to_string(), "test error");
    }

    #[test]
    fn external_error_display() {
        let source = std::io::Error::new(std::io::ErrorKind::TimedOut, "timed out");
        let e = Error::external("network", source);
        assert!(e.to_string().contains("network"));
        assert!(e.to_string().contains("timed out"));
    }
}
