use std::fmt::Display;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Io(#[from] std::io::Error),
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

pub trait Context<T> {
    fn context(self, context: impl Into<String>) -> Result<T>;

    fn with_context<C, F>(self, f: F) -> Result<T>
    where
        C: Into<String>,
        F: FnOnce() -> C;
}

impl<T, E> Context<T> for std::result::Result<T, E>
where
    E: Display,
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
