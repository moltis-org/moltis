use {
    cap_std::fs::Metadata,
    serde::{Deserialize, Serialize},
    std::fs::File,
    time::OffsetDateTime,
};

use super::FilesResult;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryKind {
    File,
    Directory,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EntryMetadata {
    pub kind: EntryKind,
    pub size_bytes: Option<u64>,
    pub modified_at: Option<OffsetDateTime>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileEntry {
    pub name: String,
    pub path: String,
    #[serde(flatten)]
    pub metadata: EntryMetadata,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirectoryListing {
    pub path: String,
    pub entries: Vec<FileEntry>,
}

pub struct OpenedFile {
    pub file: File,
    pub name: String,
    pub metadata: EntryMetadata,
}

pub(crate) fn entry_metadata(metadata: &Metadata) -> FilesResult<EntryMetadata> {
    let kind = if metadata.is_file() {
        EntryKind::File
    } else if metadata.is_dir() {
        EntryKind::Directory
    } else {
        EntryKind::Other
    };
    let modified_at = metadata
        .modified()
        .ok()
        .map(|modified| OffsetDateTime::from(modified.into_std()));
    Ok(EntryMetadata {
        kind,
        size_bytes: (kind == EntryKind::File).then_some(metadata.len()),
        modified_at,
    })
}
