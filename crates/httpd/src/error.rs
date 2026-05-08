use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error("{0}")]
    Config(String),

    #[error("{0}")]
    Tls(String),

    #[error("{0}")]
    Ssh(String),

    #[error("{0}")]
    Protocol(String),

    #[error("{0}")]
    Ngrok(String),
}

pub type Result<T> = std::result::Result<T, Error>;
