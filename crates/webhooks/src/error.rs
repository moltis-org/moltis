use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Json(#[from] serde_json::Error),

    #[error(transparent)]
    Sqlx(#[from] sqlx::Error),

    #[error(transparent)]
    Migrate(#[from] sqlx::migrate::MigrateError),

    #[error("webhook not found: {webhook_id}")]
    WebhookNotFound { webhook_id: String },

    #[error("delivery not found: {delivery_id}")]
    DeliveryNotFound { delivery_id: i64 },

    #[error("auth verification failed: {reason}")]
    AuthFailed { reason: String },

    #[error("rate limited")]
    RateLimited,

    #[error("payload too large: {size} bytes (max {max})")]
    PayloadTooLarge { size: usize, max: usize },

    #[error("duplicate delivery: {key}")]
    DuplicateDelivery { key: String },

    #[error("event filtered: {event_type}")]
    EventFiltered { event_type: String },

    #[error("{message}")]
    Message { message: String },

    #[error("{context}: {source}")]
    External {
        context: String,
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
    pub fn webhook_not_found(webhook_id: impl Into<String>) -> Self {
        Self::WebhookNotFound {
            webhook_id: webhook_id.into(),
        }
    }

    #[must_use]
    pub fn auth_failed(reason: impl Into<String>) -> Self {
        Self::AuthFailed {
            reason: reason.into(),
        }
    }

    #[must_use]
    pub fn external(
        context: impl Into<String>,
        source: impl std::error::Error + Send + Sync + 'static,
    ) -> Self {
        Self::External {
            context: context.into(),
            source: Box::new(source),
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;
