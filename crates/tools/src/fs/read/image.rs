//! Image processing for the `Read` tool.
//!
//! Intercepts known image extensions before binary rejection, runs them
//! through `moltis_media::image_ops::optimize_for_llm` (Lanczos3 resize,
//! transparency-aware format selection, progressive JPEG compression),
//! and returns a structured payload with base64 data + dimension info.

use {
    serde_json::{Value, json},
    tokio::fs,
};

use crate::{Result, error::Error, fs::shared::require_absolute};

/// Image extensions that get special handling (resize + base64) instead
/// of binary rejection. Matches Claude Code's `UYq` set.
const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp"];

/// Check whether a lowercased path ends with an image extension.
pub(crate) fn is_image_extension(lower_path: &str) -> bool {
    IMAGE_EXTENSIONS.iter().any(|ext| lower_path.ends_with(ext))
}

/// Read an image file, optimize it for LLM consumption, and return a
/// structured payload with base64 data + original/final dimensions.
pub(crate) async fn read_image(file_path: &str) -> Result<Value> {
    require_absolute(file_path, "file_path")?;

    let path = std::path::PathBuf::from(file_path);
    if !fs::try_exists(&path).await.unwrap_or(false) {
        return Ok(json!({
            "kind": "not_found",
            "file_path": file_path,
            "error": "file does not exist",
            "detail": "",
        }));
    }

    let bytes = fs::read(&path)
        .await
        .map_err(|e| Error::message(format!("failed to read image '{file_path}': {e}")))?;

    let path_owned = file_path.to_string();
    tokio::task::spawn_blocking(move || -> Result<Value> {
        match moltis_media::image_ops::optimize_for_llm(&bytes, None) {
            Ok(optimized) => {
                use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
                let media_type =
                    moltis_media::mime::detect_mime(&optimized.data, Some(&optimized.media_type));
                Ok(json!({
                    "kind": "image",
                    "file_path": path_owned,
                    "media_type": media_type,
                    "original_width": optimized.original_width,
                    "original_height": optimized.original_height,
                    "final_width": optimized.final_width,
                    "final_height": optimized.final_height,
                    "was_resized": optimized.was_resized,
                    "bytes": optimized.data.len(),
                    "base64": BASE64.encode(&optimized.data),
                }))
            },
            Err(e) => Ok(json!({
                "kind": "image_error",
                "file_path": path_owned,
                "error": format!("failed to process image: {e}"),
                "detail": "The file may be corrupted or use an unsupported image format.",
            })),
        }
    })
    .await
    .map_err(|e| Error::message(format!("image processing task failed: {e}")))?
}
