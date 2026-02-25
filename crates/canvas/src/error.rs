#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{message}")]
    Message { message: String },
}

pub type Result<T> = std::result::Result<T, Error>;
