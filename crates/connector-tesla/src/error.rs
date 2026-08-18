use thiserror::Error;

pub type Result<T> = std::result::Result<T, TeslaConnectorError>;

#[derive(Debug, Error)]
pub enum TeslaConnectorError {
    #[error("invalid Tesla account configuration: {0}")]
    AccountConfig(&'static str),
    #[error("invalid Tesla dataset configuration: {0}")]
    DatasetConfig(&'static str),
    /// Fleet API refuses every call until the developer application has been
    /// registered against a hosted public key, so it is worth its own variant
    /// with a message that names the missing setup step.
    #[error(
        "the Tesla developer application is not registered with Fleet API; complete partner \
         registration for the application domain before syncing"
    )]
    PartnerRegistrationMissing,
    #[error("Tesla authentication failed; the refresh token is invalid or expired")]
    Unauthorized,
    #[error("Tesla Fleet API rejected the request as rate limited")]
    RateLimited,
    #[error("Tesla OAuth token refresh failed")]
    OAuth(#[source] moltis_oauth::Error),
    #[error("Tesla Fleet API request failed")]
    Http(#[source] reqwest::Error),
    #[error("Tesla Fleet API returned HTTP {0}")]
    ApiStatus(u16),
    #[error("invalid Tesla Fleet API response: {0}")]
    ServerResponse(String),
    #[error("Tesla Fleet API request timed out")]
    Timeout,
    #[error("failed to encode connector item")]
    Serialization(#[source] serde_json::Error),
}

impl From<serde_json::Error> for TeslaConnectorError {
    fn from(error: serde_json::Error) -> Self {
        Self::Serialization(error)
    }
}

impl TeslaConnectorError {
    /// Maps a Fleet API status code onto the most specific variant available so
    /// callers can distinguish setup problems from transient failures.
    #[must_use]
    pub fn from_status(status: u16) -> Self {
        match status {
            401 => Self::Unauthorized,
            // Fleet API answers 403 for a token whose partner account was never
            // registered, and 412 when the request precondition (registration)
            // is unmet. Both mean the same missing setup step to a user.
            403 | 412 => Self::PartnerRegistrationMissing,
            429 => Self::RateLimited,
            other => Self::ApiStatus(other),
        }
    }
}
