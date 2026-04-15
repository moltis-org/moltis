use {
    std::path::Path,
    tracing::{debug, warn},
};

pub(super) fn spawn_startup_log_persistence(
    log_buffer: Option<&crate::logs::LogBuffer>,
    data_dir: &Path,
) {
    let Some(log_buffer) = log_buffer.cloned() else {
        return;
    };

    let persistence_path = data_dir.join("logs.jsonl");
    tokio::spawn(async move {
        let started = std::time::Instant::now();
        match tokio::task::spawn_blocking(move || {
            log_buffer.enable_persistence(persistence_path.clone());
            persistence_path
        })
        .await
        {
            Ok(path) => {
                debug!(
                    path = %path.display(),
                    elapsed_ms = started.elapsed().as_millis(),
                    "startup log persistence initialized"
                );
            },
            Err(error) => {
                warn!(%error, "startup log persistence initialization worker failed");
            },
        }
    });
}
