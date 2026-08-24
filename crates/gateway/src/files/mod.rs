mod error;
mod path;
mod service;
mod types;
mod upload;

pub use {
    error::{FilesError, FilesResult},
    service::LocalFilesService,
    types::{DirectoryListing, EntryKind, EntryMetadata, FileEntry, OpenedFile},
    upload::PendingUpload,
};

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests;
