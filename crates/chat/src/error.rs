use thiserror::Error;

#[derive(Debug, Error)]
pub enum ChatError {
    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Json(#[from] serde_json::Error),

    #[error("no config directory")]
    NoConfigDirectory,

    #[error("{message}")]
    Message { message: String },

    #[error("{context}")]
    External {
        context: &'static str,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
}

impl ChatError {
    #[must_use]
    pub fn message(message: impl std::fmt::Display) -> Self {
        Self::Message {
            message: message.to_string(),
        }
    }

    #[must_use]
    pub fn external(
        context: &'static str,
        source: impl std::error::Error + Send + Sync + 'static,
    ) -> Self {
        Self::External {
            context,
            source: Box::new(source),
        }
    }
}

pub type Result<T> = std::result::Result<T, ChatError>;
