#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{message}")]
    Message { message: String },
}

impl Error {
    #[must_use]
    pub fn message(message: impl Into<String>) -> Self {
        Self::Message {
            message: message.into(),
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;
