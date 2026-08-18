use std::io;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum FilesError {
    #[error("invalid logical file path")]
    InvalidPath,
    #[error("the managed files root cannot be mutated")]
    RootMutation,
    #[error("file entry not found")]
    NotFound,
    #[error("file entry already exists")]
    Conflict,
    #[error("file entry is not a directory")]
    NotDirectory,
    #[error("file entry is not a regular file")]
    NotFile,
    #[error("symbolic links and special files are not supported")]
    UnsupportedEntry,
    #[error("storage operation failed")]
    Io(#[source] io::Error),
}

pub type FilesResult<T> = Result<T, FilesError>;

pub(crate) fn map_io(error: io::Error) -> FilesError {
    match error.kind() {
        io::ErrorKind::NotFound => FilesError::NotFound,
        io::ErrorKind::AlreadyExists => FilesError::Conflict,
        io::ErrorKind::DirectoryNotEmpty => FilesError::Conflict,
        io::ErrorKind::NotADirectory => FilesError::NotDirectory,
        _ => FilesError::Io(error),
    }
}
