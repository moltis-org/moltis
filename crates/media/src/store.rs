use {
    anyhow::Result,
    std::path::{Path, PathBuf},
};

/// Download and save media to `<data_dir>/media/` with UUID-based naming.
pub async fn save_media_source(_url: &str, _base_dir: &Path) -> Result<PathBuf> {
    todo!("download URL, detect MIME, save as {{name}}---{{uuid}}.{{ext}}")
}
