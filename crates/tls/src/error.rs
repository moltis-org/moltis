use moltis_common::FromMessage;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    AddrParse(#[from] std::net::AddrParseError),
    #[error(transparent)]
    Rcgen(#[from] rcgen::Error),
    #[error(transparent)]
    Rustls(#[from] rustls::Error),
    #[error(transparent)]
    InvalidDnsName(#[from] rustls::pki_types::InvalidDnsNameError),
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

impl FromMessage for Error {
    fn from_message(message: String) -> Self {
        Self::Message { message }
    }
}

pub type Result<T> = std::result::Result<T, Error>;

moltis_common::impl_context!();
