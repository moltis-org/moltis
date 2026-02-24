use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error(transparent)]
    Telegram(#[from] teloxide::RequestError),

    #[error(transparent)]
    Reqwest(#[from] reqwest::Error),

    #[error(transparent)]
    Channel(#[from] moltis_channels::Error),

    #[error("{message}")]
    Message { message: String },

    #[error("{context}")]
    External {
        context: &'static str,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
}

impl Error {
    #[must_use]
    pub fn message(message: impl Into<String>) -> Self {
        Self::Message {
            message: message.into(),
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

pub type Result<T> = std::result::Result<T, Error>;
