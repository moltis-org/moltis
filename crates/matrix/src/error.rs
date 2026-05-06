use moltis_channels::Error as ChannelError;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("matrix SDK error: {0}")]
    Sdk(#[from] matrix_sdk::Error),

    #[error("matrix HTTP error: {0}")]
    Http(#[from] matrix_sdk::HttpError),

    #[error("invalid config: {0}")]
    Config(String),

    #[error("room not found: {0}")]
    RoomNotFound(String),

    #[error("ruma ID parse error: {0}")]
    IdParse(#[from] matrix_sdk::ruma::IdParseError),
}

impl From<Error> for ChannelError {
    fn from(e: Error) -> Self {
        match e {
            Error::Config(msg) => ChannelError::invalid_input(msg),
            Error::RoomNotFound(id) => ChannelError::unavailable(format!("room not found: {id}")),
            other => ChannelError::external("matrix", other),
        }
    }
}
