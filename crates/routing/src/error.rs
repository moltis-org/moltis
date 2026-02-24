#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("routing is not configured")]
    NotConfigured,
}

pub type Result<T> = std::result::Result<T, Error>;
