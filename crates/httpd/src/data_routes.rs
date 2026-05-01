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

    let mut buf = Vec::new();
    match moltis_portable::export_archive(&config_dir, &data_dir, &opts, &mut buf).await {
        Ok(_manifest) => {
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
            (headers, buf).into_response()
        },
        Err(e) => {
            warn!(error = %e, "data export failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"ok": false, "error": e.to_string()})),
            )
                .into_response()
        },
    }
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

    let reader = std::io::Cursor::new(body.to_vec());
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
