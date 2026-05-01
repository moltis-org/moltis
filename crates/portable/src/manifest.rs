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
    let decoder = GzDecoder::new(reader);
    let mut archive = Archive::new(decoder);

    for entry in archive.entries()? {
        let mut entry = entry?;
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
            },
        };
        let json = serde_json::to_string(&manifest).unwrap();
        let decoded: ExportManifest = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.format_version, FORMAT_VERSION);
        assert!(decoded.inventory.has_moltis_db);
        assert_eq!(decoded.inventory.session_count(), 1);
    }
}
