//! Archive manifest: versioning, inventory, and inspection.

use std::io::Read;

use {
    flate2::read::GzDecoder,
    serde::{Deserialize, Serialize},
    tar::Archive,
};

/// Current archive format version. Bump when layout changes in a
/// backwards-incompatible way.
pub const FORMAT_VERSION: u32 = 1;
pub(crate) const MAX_COMPRESSED_ARCHIVE_BYTES: u64 = 2 * 1024 * 1024 * 1024;
pub(crate) const MAX_UNCOMPRESSED_ARCHIVE_BYTES: u64 = 16 * 1024 * 1024 * 1024;
pub(crate) const MAX_ARCHIVE_ENTRIES: usize = 100_000;
pub(crate) const MAX_MANAGED_FILE_BYTES: u64 = 1024 * 1024 * 1024;
pub(crate) const MAX_MANAGED_TOTAL_BYTES: u64 = 8 * 1024 * 1024 * 1024;
pub(crate) const MAX_MANAGED_ENTRIES: usize = 100_000;
pub(crate) const MAX_MANAGED_TOTAL_PATH_BYTES: usize = 16 * 1024 * 1024;
pub(crate) const MAX_ARCHIVE_PATH_METADATA_BYTES: u64 = 8192;

/// Top-level manifest stored as `manifest.json` inside the archive.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportManifest {
    pub format_version: u32,
    pub moltis_version: String,
    pub created_at: String,
    pub inventory: ArchiveInventory,
}

/// Counts of items in the archive, used for preview before import.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ArchiveInventory {
    pub config_files: Vec<String>,
    pub workspace_files: Vec<String>,
    pub has_moltis_db: bool,
    pub has_memory_db: bool,
    pub session_files: Vec<String>,
    pub media_files: Vec<String>,
    #[serde(default)]
    pub managed_files: ManagedFilesInventory,
}

/// Managed Files included beneath the archive's stable `files/` subtree.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ManagedFilesInventory {
    pub files: Vec<String>,
    pub directories: Vec<String>,
    pub total_bytes: u64,
}

impl ArchiveInventory {
    pub fn session_count(&self) -> usize {
        self.session_files
            .iter()
            .filter(|f| f.ends_with(".jsonl"))
            .count()
    }

    pub fn media_count(&self) -> usize {
        self.media_files.len()
    }
}

/// Read the manifest from an archive without extracting anything else.
pub fn inspect_archive<R: Read>(reader: R) -> anyhow::Result<ExportManifest> {
    let decoder = GzDecoder::new(reader.take(MAX_COMPRESSED_ARCHIVE_BYTES + 1));
    let mut archive = Archive::new(decoder);
    let mut entry_count = 0usize;
    let mut uncompressed_bytes = 0u64;

    for entry in archive.entries()?.raw(true) {
        let mut entry = entry?;
        entry_count += 1;
        if entry_count > MAX_ARCHIVE_ENTRIES {
            anyhow::bail!("archive exceeds the {MAX_ARCHIVE_ENTRIES} entry limit");
        }
        uncompressed_bytes = uncompressed_bytes
            .checked_add(entry.size())
            .ok_or_else(|| anyhow::anyhow!("archive uncompressed size exceeds supported range"))?;
        if uncompressed_bytes > MAX_UNCOMPRESSED_ARCHIVE_BYTES {
            anyhow::bail!("archive exceeds the 16 GiB uncompressed import limit");
        }
        let entry_type = entry.header().entry_type();
        if entry_type.is_gnu_longname()
            || entry_type.is_gnu_longlink()
            || entry_type.is_pax_global_extensions()
            || entry_type.is_pax_local_extensions()
        {
            if entry.size() > MAX_ARCHIVE_PATH_METADATA_BYTES {
                anyhow::bail!("archive path metadata exceeds the 8 KiB limit");
            }
            continue;
        }
        let path = entry.path()?.to_path_buf();

        // The manifest is always the first entry, but search regardless.
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        if name == "manifest.json" {
            let manifest: ExportManifest = serde_json::from_reader(&mut entry)?;
            return Ok(manifest);
        }
    }

    anyhow::bail!("archive does not contain a manifest.json")
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn manifest_round_trip() {
        let manifest = ExportManifest {
            format_version: FORMAT_VERSION,
            moltis_version: "test".into(),
            created_at: "2026-05-01T00:00:00Z".into(),
            inventory: ArchiveInventory {
                config_files: vec!["moltis.toml".into()],
                workspace_files: vec!["SOUL.md".into()],
                has_moltis_db: true,
                has_memory_db: false,
                session_files: vec!["main.jsonl".into()],
                media_files: vec![],
                managed_files: ManagedFilesInventory {
                    files: vec!["notes/today.txt".into()],
                    directories: vec!["notes".into()],
                    total_bytes: 12,
                },
            },
        };
        let json = serde_json::to_string(&manifest).unwrap();
        let decoded: ExportManifest = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.format_version, FORMAT_VERSION);
        assert!(decoded.inventory.has_moltis_db);
        assert_eq!(decoded.inventory.session_count(), 1);
        assert_eq!(decoded.inventory.managed_files.files.len(), 1);
        assert_eq!(decoded.inventory.managed_files.total_bytes, 12);
    }

    #[test]
    fn old_manifest_defaults_managed_files_inventory() {
        let json = r#"{
            "format_version": 1,
            "moltis_version": "test",
            "created_at": "2026-05-01T00:00:00Z",
            "inventory": {
                "config_files": [],
                "workspace_files": [],
                "has_moltis_db": false,
                "has_memory_db": false,
                "session_files": [],
                "media_files": []
            }
        }"#;
        let decoded: ExportManifest = serde_json::from_str(json).unwrap();
        assert!(decoded.inventory.managed_files.files.is_empty());
        assert_eq!(decoded.inventory.managed_files.total_bytes, 0);
    }
}
