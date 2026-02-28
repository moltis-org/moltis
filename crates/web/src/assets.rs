//! Static asset serving: filesystem (dev) or embedded (release).
//!
//! In dev mode (`cargo run`), assets are served from disk so edits are
//! picked up on reload. In release builds the assets directory is embedded
//! into the binary via `include_dir!` and served with immutable
//! cache-control headers keyed by a content hash.

use std::{path::PathBuf, sync::LazyLock};

use {
    axum::{extract::Path, http::StatusCode, response::IntoResponse},
    tracing::info,
};

// ── Embedded assets ──────────────────────────────────────────────────────────

static ASSETS: include_dir::Dir = include_dir::include_dir!("$CARGO_MANIFEST_DIR/src/assets");

// ── Asset serving: filesystem (dev) or embedded (release) ────────────────────

/// Filesystem path to serve assets from, if available. Checked once at startup.
/// Set via `MOLTIS_ASSETS_DIR` env var, or auto-detected from the crate source
/// tree when running via `cargo run`.
static FS_ASSETS_DIR: LazyLock<Option<PathBuf>> = LazyLock::new(|| {
    // Explicit env var takes precedence
    if let Ok(dir) = std::env::var("MOLTIS_ASSETS_DIR") {
        let p = PathBuf::from(dir);
        if p.is_dir() {
            info!("Serving assets from filesystem: {}", p.display());
            return Some(p);
        }
    }

    // Auto-detect: works when running from the repo via `cargo run`
    let cargo_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/assets");
    if cargo_dir.is_dir() {
        info!("Serving assets from filesystem: {}", cargo_dir.display());
        return Some(cargo_dir);
    }

    info!("Serving assets from embedded binary");
    None
});

/// Whether we're serving from the filesystem (dev mode) or embedded (release).
pub(crate) fn is_dev_assets() -> bool {
    FS_ASSETS_DIR.is_some()
}

/// Compute a short content hash of all embedded assets. Only used in release
/// mode (embedded assets) for cache-busting versioned URLs.
pub(crate) fn asset_content_hash() -> String {
    use std::{collections::BTreeMap, hash::Hasher};

    let mut files = BTreeMap::new();
    let mut stack: Vec<&include_dir::Dir<'_>> = vec![&ASSETS];
    while let Some(dir) = stack.pop() {
        for file in dir.files() {
            files.insert(file.path().display().to_string(), file.contents());
        }
        for sub in dir.dirs() {
            stack.push(sub);
        }
    }

    let mut h = std::hash::DefaultHasher::new();
    for (path, contents) in &files {
        h.write(path.as_bytes());
        h.write(contents);
    }
    format!("{:016x}", h.finish())
}

fn mime_for_path(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or("") {
        "css" => "text/css; charset=utf-8",
        "js" => "application/javascript; charset=utf-8",
        "mjs" => "application/javascript; charset=utf-8",
        "html" => "text/html; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "ico" => "image/x-icon",
        "json" => "application/json",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        _ => "application/octet-stream",
    }
}

/// Read an asset file, preferring filesystem over embedded.
fn read_asset(path: &str) -> Option<Vec<u8>> {
    if let Some(dir) = FS_ASSETS_DIR.as_ref() {
        let file_path = dir.join(path);
        // Prevent path traversal
        if file_path.starts_with(dir)
            && let Ok(bytes) = std::fs::read(&file_path)
        {
            return Some(bytes);
        }
    }
    ASSETS.get_file(path).map(|f| f.contents().to_vec())
}

/// Versioned assets: `/assets/v/<hash>/path` — immutable, cached forever.
pub async fn versioned_asset_handler(
    Path((_version, path)): Path<(String, String)>,
) -> impl IntoResponse {
    let cache = if is_dev_assets() {
        "no-cache, no-store"
    } else {
        "public, max-age=31536000, immutable"
    };
    serve_asset(&path, cache)
}

/// Unversioned assets: `/assets/path` — always revalidate.
pub async fn asset_handler(Path(path): Path<String>) -> impl IntoResponse {
    let cache = if is_dev_assets() {
        "no-cache, no-store"
    } else {
        "no-cache"
    };
    serve_asset(&path, cache)
}

/// PWA manifest: `/manifest.json` — served from assets root.
pub async fn manifest_handler() -> impl IntoResponse {
    serve_asset("manifest.json", "no-cache")
}

/// Service worker: `/sw.js` — served from assets root, no-cache for updates.
pub async fn service_worker_handler() -> impl IntoResponse {
    serve_asset("sw.js", "no-cache")
}

fn serve_asset(path: &str, cache_control: &'static str) -> axum::response::Response {
    match read_asset(path) {
        Some(body) => {
            let mut response = (
                StatusCode::OK,
                [
                    ("content-type", mime_for_path(path)),
                    ("cache-control", cache_control),
                    ("x-content-type-options", "nosniff"),
                ],
                body,
            )
                .into_response();

            // Harden SVG delivery against script execution when user-controlled
            // SVGs are ever introduced. Static first-party SVGs continue to render.
            if path.rsplit('.').next().unwrap_or("") == "svg" {
                response.headers_mut().insert(
                    axum::http::header::CONTENT_SECURITY_POLICY,
                    axum::http::HeaderValue::from_static(
                        "default-src 'none'; img-src 'self' data:; style-src 'none'; script-src 'none'; object-src 'none'; frame-ancestors 'none'",
                    ),
                );
            }

            response
        },
        None => (StatusCode::NOT_FOUND, "not found").into_response(),
    }
}
