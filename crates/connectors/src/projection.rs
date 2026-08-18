use std::{
    fmt::Write as _,
    path::{Path, PathBuf},
};

use {
    sha2::{Digest, Sha256},
    tokio::fs,
    uuid::Uuid,
};

use crate::{
    ConnectorError, ConnectorItem, Dataset, ProjectionManifest, ProjectionManifestItem, Result,
    error::IoResultExt,
};

#[cfg_attr(
    feature = "tracing",
    tracing::instrument(skip(export_root, dataset, items), fields(dataset_id = %dataset.id, item_count = items.len()))
)]
pub async fn write_projection(
    export_root: &Path,
    dataset_slug: &str,
    dataset: &Dataset,
    items: &[ConnectorItem],
) -> Result<ProjectionManifest> {
    if dataset_slug.trim().is_empty() {
        return Err(ConnectorError::InvalidInput(
            "dataset projection slug must not be empty".to_owned(),
        ));
    }
    if items.iter().any(|item| item.dataset_id != dataset.id) {
        return Err(ConnectorError::InvalidInput(
            "all projected items must belong to the dataset".to_owned(),
        ));
    }

    fs::create_dir_all(export_root).await.at(export_root)?;
    let target = projection_directory(export_root, dataset_slug, &dataset.id)?;
    let staging = export_root.join(format!(".connector-export-{}", Uuid::new_v4()));
    fs::create_dir(&staging).await.at(&staging)?;

    let result = populate_staging(&staging, dataset, items).await;
    let manifest = match result {
        Ok(manifest) => manifest,
        Err(error) => {
            let _ = fs::remove_dir_all(&staging).await;
            return Err(error);
        },
    };

    if let Err(error) = publish_directory(&staging, &target).await {
        let _ = fs::remove_dir_all(&staging).await;
        return Err(error);
    }
    record_projection(items.len());
    Ok(manifest)
}

pub async fn cleanup_projection_artifacts(export_root: &Path) -> Result<usize> {
    let mut entries = match fs::read_dir(export_root).await {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(source) => {
            return Err(ConnectorError::Io {
                path: export_root.to_path_buf(),
                source,
            });
        },
    };
    let mut handled = 0;
    while let Some(entry) = entries.next_entry().await.at(export_root)? {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let path = entry.path();
        let file_type = entry.file_type().await.at(&path)?;
        if !file_type.is_dir() {
            continue;
        }
        if name.starts_with(".connector-export-") {
            fs::remove_dir_all(&path).await.at(&path)?;
            handled += 1;
            continue;
        }
        let Some(target_name) = name.strip_prefix(".connector-backup-") else {
            continue;
        };
        if target_name.is_empty() {
            fs::remove_dir_all(&path).await.at(&path)?;
            handled += 1;
            continue;
        }
        let target = export_root.join(target_name);
        match fs::metadata(&target).await {
            Ok(_) => fs::remove_dir_all(&path).await.at(&path)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                fs::rename(&path, &target).await.at(&target)?;
            },
            Err(source) => {
                return Err(ConnectorError::Io {
                    path: target,
                    source,
                });
            },
        }
        handled += 1;
    }
    Ok(handled)
}

pub fn projection_directory(
    export_root: &Path,
    dataset_slug: &str,
    dataset_id: &str,
) -> Result<PathBuf> {
    if dataset_slug.trim().is_empty() || dataset_id.trim().is_empty() {
        return Err(ConnectorError::InvalidInput(
            "dataset projection slug and id must not be empty".to_owned(),
        ));
    }
    Ok(export_root.join(format!(
        "{}-{}",
        sanitize_component(dataset_slug),
        short_hash(dataset_id)
    )))
}

async fn populate_staging(
    staging: &Path,
    dataset: &Dataset,
    items: &[ConnectorItem],
) -> Result<ProjectionManifest> {
    let markdown_dir = staging.join("items");
    if dataset.projections.markdown {
        fs::create_dir(&markdown_dir).await.at(&markdown_dir)?;
    }

    let mut manifest_items = Vec::with_capacity(items.len());
    for item in items {
        let markdown_path = if dataset.projections.markdown {
            let filename = item_filename(item);
            let relative = PathBuf::from("items").join(&filename);
            let path = staging.join(&relative);
            atomic_write(&path, render_markdown(item)?.as_bytes()).await?;
            Some(path_to_manifest_string(&relative))
        } else {
            None
        };
        manifest_items.push(ProjectionManifestItem {
            remote_id: item.remote_id.clone(),
            kind: item.kind.clone(),
            content_hash: item.content_hash.clone(),
            markdown_path,
        });
    }

    let jsonl_path = if dataset.projections.jsonl {
        let mut output = Vec::new();
        for item in items {
            serde_json::to_writer(&mut output, item)?;
            output.push(b'\n');
        }
        atomic_write(&staging.join("items.jsonl"), &output).await?;
        Some("items.jsonl".to_owned())
    } else {
        None
    };

    let manifest = ProjectionManifest {
        dataset_id: dataset.id.clone(),
        dataset_name: dataset.name.clone(),
        source_sync_at: dataset.last_sync_at,
        source_plan_revision: dataset.synced_plan_revision,
        projections: dataset.projections,
        generated_at: time::OffsetDateTime::now_utc(),
        item_count: items.len(),
        jsonl_path,
        items: manifest_items,
    };
    let manifest_json = serde_json::to_vec_pretty(&manifest)?;
    atomic_write(&staging.join("manifest.json"), &manifest_json).await?;
    Ok(manifest)
}

async fn publish_directory(staging: &Path, target: &Path) -> Result<()> {
    if fs::metadata(target).await.is_err() {
        return fs::rename(staging, target).await.at(target);
    }

    let parent = target.parent().ok_or_else(|| {
        ConnectorError::InvalidInput("projection target has no parent directory".to_owned())
    })?;
    let target_name = target
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            ConnectorError::InvalidInput("projection target has no valid directory name".to_owned())
        })?;
    let backup = parent.join(format!(".connector-backup-{target_name}"));
    if fs::metadata(&backup).await.is_ok() {
        fs::remove_dir_all(&backup).await.at(&backup)?;
    }
    fs::rename(target, &backup).await.at(target)?;
    if let Err(source) = fs::rename(staging, target).await {
        if let Err(error) = fs::rename(&backup, target).await {
            warn_backup_restore(target, &backup, &error);
        }
        return Err(ConnectorError::Io {
            path: target.to_path_buf(),
            source,
        });
    }
    if let Err(error) = fs::remove_dir_all(&backup).await {
        warn_backup_cleanup(&backup, &error);
    }
    Ok(())
}

async fn atomic_write(path: &Path, contents: &[u8]) -> Result<()> {
    let parent = path.parent().ok_or_else(|| {
        ConnectorError::InvalidInput("projection file has no parent directory".to_owned())
    })?;
    let temporary = parent.join(format!(".tmp-{}", Uuid::new_v4()));
    fs::write(&temporary, contents).await.at(&temporary)?;
    if let Err(source) = fs::rename(&temporary, path).await {
        let _ = fs::remove_file(&temporary).await;
        return Err(ConnectorError::Io {
            path: path.to_path_buf(),
            source,
        });
    }
    Ok(())
}

fn render_markdown(item: &ConnectorItem) -> Result<String> {
    let body = serde_json::to_string_pretty(&item.body_json)?;
    let fence = code_fence(&body);
    Ok(format!(
        "# Connector item\n\n{fence}json\n{body}\n{fence}\n"
    ))
}

fn code_fence(value: &str) -> String {
    let longest = value
        .split(|character| character != '`')
        .map(str::len)
        .max()
        .unwrap_or_default();
    "`".repeat(longest.saturating_add(1).max(3))
}

fn item_filename(item: &ConnectorItem) -> String {
    format!(
        "{}-{}.md",
        sanitize_component(&item.remote_id),
        short_hash(&item.remote_id)
    )
}

fn sanitize_component(value: &str) -> String {
    let mut output = String::with_capacity(value.len().min(64));
    let mut previous_separator = false;
    for character in value.chars().take(64) {
        let safe = character.is_ascii_alphanumeric() || matches!(character, '-' | '_');
        if safe {
            output.push(character);
            previous_separator = false;
        } else if !previous_separator {
            output.push('_');
            previous_separator = true;
        }
    }
    let output = output.trim_matches('_');
    if output.is_empty() || output == "." || output == ".." {
        "dataset".to_owned()
    } else {
        output.to_owned()
    }
}

fn short_hash(value: &str) -> String {
    let digest = Sha256::digest(value.as_bytes());
    let mut output = String::with_capacity(16);
    for byte in digest.iter().take(8) {
        let _ = write!(output, "{byte:02x}");
    }
    output
}

fn path_to_manifest_string(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(feature = "metrics")]
fn record_projection(item_count: usize) {
    metrics::counter!("moltis_connector_projection_writes_total").increment(1);
    metrics::histogram!("moltis_connector_projection_items").record(item_count as f64);
}

#[cfg(not(feature = "metrics"))]
fn record_projection(_item_count: usize) {}

#[cfg(feature = "tracing")]
fn warn_backup_cleanup(path: &Path, error: &std::io::Error) {
    tracing::warn!(path = %path.display(), %error, "failed to remove obsolete connector projection backup");
}

#[cfg(not(feature = "tracing"))]
fn warn_backup_cleanup(_path: &Path, _error: &std::io::Error) {}

#[cfg(feature = "tracing")]
fn warn_backup_restore(target: &Path, backup: &Path, error: &std::io::Error) {
    tracing::error!(
        target = %target.display(),
        backup = %backup.display(),
        %error,
        "failed to restore connector projection backup"
    );
}

#[cfg(not(feature = "tracing"))]
fn warn_backup_restore(_target: &Path, _backup: &Path, _error: &std::io::Error) {}
