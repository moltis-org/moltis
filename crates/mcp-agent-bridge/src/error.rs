use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error(transparent)]
    Mcp(#[from] moltis_mcp::error::Error),
}

pub type Result<T> = std::result::Result<T, Error>;
