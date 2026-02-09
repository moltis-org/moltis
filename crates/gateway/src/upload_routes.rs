//! Generic session file upload endpoint.
//!
//! `POST /api/sessions/{session_key}/upload` accepts raw binary data.
//! The `Content-Type` header determines behaviour:
//! - **Audio** (`audio/*`): stored + optionally transcribed (`?transcribe=true`)
//! - **Images** (`image/*`): stored, URL returned
//! - **Other**: stored, URL returned

use {
    axum::{
        Json,
        body::Bytes,
        extract::{Path, Query, State},
        http::{HeaderMap, StatusCode},
        response::IntoResponse,
    },
    serde::Deserialize,
    tracing::warn,
};

/// Query parameters for the upload endpoint.
#[derive(Debug, Deserialize, Default)]
pub struct UploadQuery {
    /// Request STT transcription for audio uploads.
    #[serde(default)]
    pub transcribe: bool,
    /// Optional STT provider override (e.g. `whisper`, `groq`).
    pub provider: Option<String>,
    /// Optional language hint for transcription.
    pub language: Option<String>,
    /// Optional transcription prompt.
    pub prompt: Option<String>,
}

/// Maximum upload size: 25 MB (also used as the route-level body limit).
pub const MAX_UPLOAD_SIZE: usize = 25 * 1024 * 1024;

/// `POST /api/sessions/{session_key}/upload`
///
/// Accepts raw binary body. `Content-Type` header is required.
/// Optional `X-Filename` header for custom filenames.
pub async fn session_upload(
    State(state): State<crate::server::AppState>,
    Path(session_key): Path<String>,
    Query(query): Query<UploadQuery>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    // Reject empty body.
    if body.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "ok": false, "error": "empty body" })),
        )
            .into_response();
    }

    // Enforce upload size limit.
    if body.len() > MAX_UPLOAD_SIZE {
        return (
            StatusCode::PAYLOAD_TOO_LARGE,
            Json(serde_json::json!({
                "ok": false,
                "error": format!("body exceeds maximum upload size ({} bytes)", MAX_UPLOAD_SIZE),
            })),
        )
            .into_response();
    }

    // Read Content-Type (required).
    let content_type = headers
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/octet-stream");

    // Determine file extension from content-type.
    let ext = extension_for_content_type(content_type);

    // Read optional X-Filename or generate one.
    let filename = headers
        .get("x-filename")
        .and_then(|v| v.to_str().ok())
        .map(sanitize_filename)
        .unwrap_or_else(|| {
            let ts = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            let prefix = if content_type.starts_with("audio/") {
                "voice"
            } else if content_type.starts_with("image/") {
                "image"
            } else {
                "file"
            };
            format!("{prefix}-{ts}.{ext}")
        });

    // We need the session store to save media.
    let Some(ref store) = state.gateway.services.session_store else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({ "ok": false, "error": "session store not available" })),
        )
            .into_response();
    };

    // Save the file.
    let size = body.len();
    if let Err(e) = store.save_media(&session_key, &filename, &body).await {
        warn!(session_key, filename, error = %e, "failed to save uploaded media");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "ok": false, "error": format!("failed to save file: {e}") })),
        )
            .into_response();
    }

    let url = format!(
        "/api/sessions/{}/media/{}",
        urlencoding::encode(&session_key),
        urlencoding::encode(&filename),
    );

    // Optionally transcribe audio.
    let transcription = if query.transcribe && content_type.starts_with("audio/") {
        let format = format_name_for_content_type(content_type);

        match state
            .gateway
            .services
            .stt
            .transcribe_bytes(
                body,
                format,
                query.provider.as_deref(),
                query.language.as_deref(),
                query.prompt.as_deref(),
            )
            .await
        {
            Ok(result) => Some(result),
            Err(e) => {
                warn!(session_key, error = %e, "transcription failed for uploaded audio");
                return (
                    StatusCode::OK,
                    Json(serde_json::json!({
                        "ok": true,
                        "url": url,
                        "filename": filename,
                        "contentType": content_type,
                        "size": size,
                        "transcriptionError": e,
                    })),
                )
                    .into_response();
            },
        }
    } else {
        None
    };

    let mut response = serde_json::json!({
        "ok": true,
        "url": url,
        "filename": filename,
        "contentType": content_type,
        "size": size,
    });

    if let Some(t) = transcription {
        response["transcription"] = t;
    }

    Json(response).into_response()
}

/// Sanitize a user-provided filename: keep only safe characters.
fn sanitize_filename(name: &str) -> String {
    let sanitized: String = name
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_' || *c == '.')
        .collect();
    // Strip leading dots to prevent hidden files / path traversal remnants.
    let sanitized = sanitized.trim_start_matches('.');
    if sanitized.is_empty() {
        "upload".to_string()
    } else {
        sanitized.to_string()
    }
}

/// Map an audio content-type to a short format name for STT.
fn format_name_for_content_type(ct: &str) -> &'static str {
    let base = ct.split(';').next().unwrap_or(ct).trim();
    match base {
        "audio/webm" => "webm",
        "audio/ogg" | "audio/opus" => "ogg",
        "audio/mpeg" | "audio/mp3" => "mp3",
        "audio/aac" | "audio/mp4" | "audio/m4a" => "aac",
        "audio/pcm" | "audio/wav" | "audio/x-wav" => "pcm",
        _ => "webm",
    }
}

/// Map a content-type to a file extension.
fn extension_for_content_type(ct: &str) -> &'static str {
    let base = ct.split(';').next().unwrap_or(ct).trim();
    match base {
        "audio/webm" => "webm",
        "audio/ogg" | "audio/opus" => "ogg",
        "audio/mpeg" | "audio/mp3" => "mp3",
        "audio/aac" | "audio/mp4" => "aac",
        "audio/wav" | "audio/x-wav" => "wav",
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/svg+xml" => "svg",
        "application/pdf" => "pdf",
        "text/plain" => "txt",
        _ => "bin",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_filename() {
        assert_eq!(sanitize_filename("voice.webm"), "voice.webm");
        assert_eq!(sanitize_filename("my file (1).mp3"), "myfile1.mp3");
        assert_eq!(sanitize_filename("../../../etc/passwd"), "etcpasswd");
        assert_eq!(sanitize_filename(""), "upload");
        assert_eq!(sanitize_filename("hello-world_2.ogg"), "hello-world_2.ogg");
    }

    #[test]
    fn test_extension_for_content_type() {
        assert_eq!(extension_for_content_type("audio/webm"), "webm");
        assert_eq!(extension_for_content_type("audio/webm;codecs=opus"), "webm");
        assert_eq!(extension_for_content_type("audio/ogg"), "ogg");
        assert_eq!(extension_for_content_type("audio/mpeg"), "mp3");
        assert_eq!(extension_for_content_type("image/png"), "png");
        assert_eq!(extension_for_content_type("image/jpeg"), "jpg");
        assert_eq!(extension_for_content_type("application/pdf"), "pdf");
        assert_eq!(extension_for_content_type("something/else"), "bin");
    }
}
