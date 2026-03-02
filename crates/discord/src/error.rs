use moltis_channels::Error as ChannelError;

/// Errors specific to the Discord channel plugin.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("discord config: {0}")]
    Config(String),

    #[error("discord gateway: {0}")]
    Gateway(String),

    #[error("discord send: {0}")]
    Send(String),

    #[error(transparent)]
    Channel(#[from] ChannelError),
}

impl From<Error> for ChannelError {
    fn from(err: Error) -> Self {
        match err {
            Error::Config(msg) => ChannelError::invalid_input(msg),
            Error::Gateway(msg) => ChannelError::unavailable(msg),
            Error::Send(msg) => ChannelError::external("Discord send", std::io::Error::other(msg)),
            Error::Channel(e) => e,
        }
    }
}
