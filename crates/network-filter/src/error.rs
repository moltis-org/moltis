use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Json(#[from] serde_json::Error),

    #[error("{0}")]
    Message(String),
}

impl Error {
    #[must_use]
    pub fn message(msg: impl Into<String>) -> Self {
        Self::Message(msg.into())
    }
}

pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn io_error_converts() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "gone");
        let err: Error = io_err.into();
        assert!(matches!(err, Error::Io(_)));
        assert!(err.to_string().contains("gone"));
    }

    #[test]
    fn json_error_converts() {
        let json_err = serde_json::from_str::<String>("not-json").unwrap_err();
        let err: Error = json_err.into();
        assert!(matches!(err, Error::Json(_)));
    }

    #[test]
    fn message_error() {
        let err = Error::message("something broke");
        assert_eq!(err.to_string(), "something broke");
        assert!(matches!(err, Error::Message(_)));
    }
}
