use {
    anyhow::{Context, Result},
    std::path::Path,
    tokio::io::AsyncWriteExt,
};

use super::PushStore;

pub(super) async fn load(path: &Path) -> Result<PushStore> {
    if !tokio::fs::try_exists(path)
        .await
        .with_context(|| format!("failed to inspect push store {}", path.display()))?
    {
        return Ok(PushStore::default());
    }

    let content = tokio::fs::read_to_string(path)
        .await
        .with_context(|| format!("failed to read push store {}", path.display()))?;
    serde_json::from_str(&content)
        .with_context(|| format!("failed to parse push store {}", path.display()))
}

pub(super) async fn save(path: &Path, store: &PushStore) -> Result<()> {
    let content = serde_json::to_vec_pretty(store).context("failed to serialize push store")?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("push.json");
    let temp_path = path.with_file_name(format!(
        ".{file_name}.{}.tmp",
        uuid::Uuid::new_v4().simple()
    ));

    let result = async {
        let mut options = tokio::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            options.mode(0o600);
        }
        let mut file = options
            .open(&temp_path)
            .await
            .with_context(|| format!("failed to create {}", temp_path.display()))?;
        file.write_all(&content)
            .await
            .context("failed to write temporary push store")?;
        file.flush()
            .await
            .context("failed to flush temporary push store")?;
        file.sync_all()
            .await
            .context("failed to sync temporary push store")?;
        drop(file);
        tokio::fs::rename(&temp_path, path)
            .await
            .with_context(|| format!("failed to replace push store {}", path.display()))?;
        #[cfg(unix)]
        if let Some(parent) = path.parent() {
            match tokio::fs::File::open(parent).await {
                Ok(directory) => {
                    if let Err(error) = directory.sync_all().await {
                        tracing::warn!(%error, "failed to sync push store directory");
                    }
                },
                Err(error) => {
                    tracing::warn!(%error, "failed to open push store directory for sync")
                },
            }
        }
        Ok(())
    }
    .await;

    if result.is_err() {
        match tokio::fs::remove_file(&temp_path).await {
            Ok(()) => {},
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {},
            Err(error) => tracing::warn!(%error, "failed to clean up temporary push store"),
        }
    }
    result
}
