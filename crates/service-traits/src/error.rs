use serde_json::Value;

/// Error type returned by service methods.
#[derive(Debug, thiserror::Error)]
pub enum ServiceError {
    #[error("{message}")]
    Message { message: String },
    #[error("{message}")]
    Forbidden { message: String },
    #[error("{0}")]
    Serde(#[from] serde_json::Error),
}

impl ServiceError {
    #[must_use]
    pub fn message(message: impl std::fmt::Display) -> Self {
        Self::Message {
            message: message.to_string(),
        }
    }

    #[must_use]
    pub fn forbidden(message: impl std::fmt::Display) -> Self {
        Self::Forbidden {
            message: message.to_string(),
        }
    }
}

impl From<String> for ServiceError {
    fn from(value: String) -> Self {
        Self::message(value)
    }
}

impl From<&str> for ServiceError {
    fn from(value: &str) -> Self {
        Self::message(value)
    }
}

impl From<ServiceError> for moltis_protocol::ErrorShape {
    fn from(err: ServiceError) -> Self {
        let code = match &err {
            ServiceError::Forbidden { .. } => moltis_protocol::error_codes::FORBIDDEN,
            _ => moltis_protocol::error_codes::INTERNAL,
        };
        Self::new(code, err.to_string())
    }
}

pub type ServiceResult<T = Value> = Result<T, ServiceError>;
