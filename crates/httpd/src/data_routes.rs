//! Export/import Moltis data as `.tar.gz` archives.
//!
//! - `GET  /api/data/export` — stream a backup archive
//! - `POST /api/data/import` — upload and apply an archive
//! - `POST /api/data/import/preview` — upload and preview without applying

use {
    axum::{
        Json, Router,
        body::Bytes,
        extract::Query,
        http::{StatusCode, header},
        response::IntoResponse,
        routing::{get, post},
    },
    moltis_portable::{ConflictStrategy, ExportOptions, ImportOptions},
    serde::Deserialize,
    tokio_util::io::ReaderStream,
    tracing::warn,
};

use crate::AppState;

/// Maximum import archive size: 2 GB.
const MAX_IMPORT_SIZE: usize = 2 * 1024 * 1024 * 1024;

#[derive(Debug, Deserialize, Default)]
pub struct ExportQuery {
    #[serde(default = "default_true")]
    pub include_provider_keys: bool,
    #[serde(default)]
    pub include_media: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Deserialize, Default)]
pub struct ImportQuery {
    #[serde(default)]
    pub conflict: Option<String>,
}

pub fn data_router() -> Router<AppState> {
    Router::new()
        .route("/export", get(export_handler))
        .route(
            "/import",
            post(import_handler).layer(axum::extract::DefaultBodyLimit::max(MAX_IMPORT_SIZE)),
        )
        .route(
            "/import/preview",
            post(import_preview_handler)
                .layer(axum::extract::DefaultBodyLimit::max(MAX_IMPORT_SIZE)),
        )
}

/// `GET /api/data/export`
///
/// Writes the archive to a temporary file, then streams it to the client.
/// This avoids buffering the entire archive in memory while still allowing
/// the synchronous `tar`/`flate2` writers to work naturally.
async fn export_handler(Query(query): Query<ExportQuery>) -> impl IntoResponse {
    let config_dir = match moltis_config::config_dir() {
        Some(d) => d,
        None => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"ok": false, "error": "config directory not set"})),
            )
                .into_response();
        },
    };
    let data_dir = moltis_config::data_dir();

    let opts = ExportOptions {
        include_provider_keys: query.include_provider_keys,
        include_media: query.include_media,
    };

    // Write to a temp file so we don't buffer the full archive in memory.
    let tmp_file = match tempfile::NamedTempFile::new() {
        Ok(f) => f,
        Err(e) => {
            warn!(error = %e, "failed to create temp file for export");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"ok": false, "error": "failed to create temp file"})),
            )
                .into_response();
        },
    };

    let tmp_path = tmp_file.path().to_path_buf();
    {
        let file = match std::fs::File::create(&tmp_path) {
            Ok(f) => f,
            Err(e) => {
                warn!(error = %e, "failed to open temp file for export");
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({"ok": false, "error": "failed to open temp file"})),
                )
                    .into_response();
            },
        };

        if let Err(e) = moltis_portable::export_archive(&config_dir, &data_dir, &opts, file).await {
            warn!(error = %e, "data export failed");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"ok": false, "error": e.to_string()})),
            )
                .into_response();
        }
    }

    // Stream the temp file to the client.
    let async_file = match tokio::fs::File::open(&tmp_path).await {
        Ok(f) => f,
        Err(e) => {
            warn!(error = %e, "failed to reopen temp file for streaming");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"ok": false, "error": "failed to read export"})),
            )
                .into_response();
        },
    };

    // Keep the NamedTempFile alive so it isn't deleted until streaming completes.
    // The file will be cleaned up when `tmp_file` is dropped after the response.
    let stream = ReaderStream::new(tokio::io::BufReader::new(async_file));
    let body = axum::body::Body::from_stream(stream);

    let now = time::OffsetDateTime::now_utc();
    let filename = format!(
        "moltis-backup-{:04}{:02}{:02}-{:02}{:02}{:02}.tar.gz",
        now.year(),
        now.month() as u8,
        now.day(),
        now.hour(),
        now.minute(),
        now.second(),
    );

    let headers = [
        (header::CONTENT_TYPE, "application/gzip".to_owned()),
        (
            header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{filename}\""),
        ),
    ];
    (headers, body).into_response()
}

/// `POST /api/data/import`
async fn import_handler(Query(query): Query<ImportQuery>, body: Bytes) -> impl IntoResponse {
    run_import(query, body, false).await
}

/// `POST /api/data/import/preview`
async fn import_preview_handler(
    Query(query): Query<ImportQuery>,
    body: Bytes,
) -> impl IntoResponse {
    run_import(query, body, true).await
}

async fn run_import(query: ImportQuery, body: Bytes, dry_run: bool) -> impl IntoResponse {
    if body.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"ok": false, "error": "empty body"})),
        )
            .into_response();
    }

    let config_dir = match moltis_config::config_dir() {
        Some(d) => d,
        None => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"ok": false, "error": "config directory not set"})),
            )
                .into_response();
        },
    };
    let data_dir = moltis_config::data_dir();

    let conflict = match query.conflict.as_deref() {
        Some("overwrite") => ConflictStrategy::Overwrite,
        _ => ConflictStrategy::Skip,
    };

    let opts = ImportOptions { conflict, dry_run };

    // Cursor<Bytes> implements Read without copying the data.
    let reader = std::io::Cursor::new(body);
    match moltis_portable::import_archive(&config_dir, &data_dir, &opts, reader).await {
        Ok(result) => Json(serde_json::json!({
            "ok": true,
            "imported": result.imported,
            "skipped": result.skipped,
            "warnings": result.warnings,
            "manifest": result.manifest,
        }))
        .into_response(),
        Err(e) => {
            warn!(error = %e, "data import failed");
            (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"ok": false, "error": e.to_string()})),
            )
                .into_response()
        },
    }
}
