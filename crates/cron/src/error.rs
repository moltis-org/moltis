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

    #[error(transparent)]
    ChronoParse(#[from] chrono::ParseError),

    #[error(transparent)]
    CronParse(#[from] cron::error::Error),

    #[error("unknown timezone: {timezone}")]
    UnknownTimezone { timezone: String },

    #[error("job not found: {job_id}")]
    JobNotFound { job_id: String },

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
    pub fn job_not_found(job_id: impl Into<String>) -> Self {
        Self::JobNotFound {
            job_id: job_id.into(),
        }
    }

    #[must_use]
    pub fn unknown_timezone(timezone: impl Into<String>) -> Self {
        Self::UnknownTimezone {
            timezone: timezone.into(),
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
