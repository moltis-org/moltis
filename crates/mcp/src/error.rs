use std::error::Error as StdError;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Reqwest(#[from] reqwest::Error),
    #[error(transparent)]
    SerdeJson(#[from] serde_json::Error),
    #[error(transparent)]
    UrlParse(#[from] url::ParseError),
    #[error(transparent)]
    Transport(#[from] crate::types::McpTransportError),
    #[error(transparent)]
    Manager(#[from] crate::types::McpManagerError),
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
    pub fn message(message: impl Into<String>) -> Self {
        Self::Message {
            message: message.into(),
        }
    }

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

pub trait Context<T> {
    fn context(self, context: impl Into<String>) -> Result<T>;

    fn with_context<C, F>(self, f: F) -> Result<T>
    where
        C: Into<String>,
        F: FnOnce() -> C;
}

impl<T, E> Context<T> for std::result::Result<T, E>
where
    E: std::fmt::Display,
{
    fn context(self, context: impl Into<String>) -> Result<T> {
        let context = context.into();
        self.map_err(|source| Error::message(format!("{context}: {source}")))
    }

    fn with_context<C, F>(self, f: F) -> Result<T>
    where
        C: Into<String>,
        F: FnOnce() -> C,
    {
        let context = f().into();
        self.map_err(|source| Error::message(format!("{context}: {source}")))
    }
}

impl<T> Context<T> for Option<T> {
    fn context(self, context: impl Into<String>) -> Result<T> {
        self.ok_or_else(|| Error::message(context.into()))
    }

    fn with_context<C, F>(self, f: F) -> Result<T>
    where
        C: Into<String>,
        F: FnOnce() -> C,
    {
        self.ok_or_else(|| Error::message(f().into()))
    }
}

pub type Result<T> = std::result::Result<T, Error>;
