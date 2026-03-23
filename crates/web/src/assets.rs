//! Static asset serving with three-tier resolution:
//!
//! 1. **Dev filesystem** — `MOLTIS_ASSETS_DIR` env var or auto-detected from
//!    the crate source tree when running via `cargo run`.
//! 2. **External share dir** — `share_dir()/web/` for packaged deployments
//!    (Debian, RPM, Docker) where assets live outside the binary.
//! 3. **Embedded fallback** — `include_dir!` compiled into the binary (only
//!    available when the `embedded-assets` feature is enabled).

use std::{
    path::{Component, Path as FsPath, PathBuf},
    sync::LazyLock,
};

use {
    axum::{extract::Path, http::StatusCode, response::IntoResponse},
    tracing::info,
};

// ── Embedded assets (feature-gated) ─────────────────────────────────────────

#[cfg(feature = "embedded-assets")]
static ASSETS: include_dir::Dir = include_dir::include_dir!("$CARGO_MANIFEST_DIR/src/assets");

// Fail compilation with a clear message if style.css hasn't been generated.
// Run `just build-css` (or `cd crates/web/ui && ./build.sh`) to generate it.
#[cfg(feature = "embedded-assets")]
const _: &str = include_str!("assets/style.css");

// ── Asset source resolution ─────────────────────────────────────────────────

/// Resolved asset source, checked once at startup.
enum AssetSource {
    /// Filesystem directory (dev mode or `MOLTIS_ASSETS_DIR`).
    Filesystem(PathBuf),
    /// External share directory (`share_dir()/web/`).
    External(PathBuf),
    /// Embedded in binary (feature `embedded-assets`).
    #[cfg(feature = "embedded-assets")]
    Embedded,
    /// No assets available (embedded-assets feature disabled, no external dir).
    #[cfg(not(feature = "embedded-assets"))]
    Unavailable,
}

static ASSET_SOURCE: LazyLock<AssetSource> = LazyLock::new(|| {
    // 1. Explicit env var
    if let Ok(dir) = std::env::var("MOLTIS_ASSETS_DIR") {
        let p = PathBuf::from(dir);
        if p.is_dir() {
            info!("Serving assets from filesystem: {}", p.display());
            return AssetSource::Filesystem(p);
        }
    }

    // 2. Auto-detect cargo source tree (dev mode)
    let cargo_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/assets");
    if cargo_dir.is_dir() {
        info!("Serving assets from filesystem: {}", cargo_dir.display());
        return AssetSource::Filesystem(cargo_dir);
    }

    // 3. External share directory
    if let Some(share) = moltis_config::share_dir() {
        let web_dir = share.join("web");
        if web_dir.is_dir() {
            info!(
                "Serving assets from external share dir: {}",
                web_dir.display()
            );
            return AssetSource::External(web_dir);
        }
    }

    // 4. Embedded fallback (or unavailable)
    #[cfg(feature = "embedded-assets")]
    {
        info!("Serving assets from embedded binary");
        AssetSource::Embedded
    }
    #[cfg(not(feature = "embedded-assets"))]
    {
        info!("No asset source available (embedded-assets feature disabled)");
        AssetSource::Unavailable
    }
});

/// Whether we're serving from the filesystem (dev mode) or embedded/external (release).
pub(crate) fn is_dev_assets() -> bool {
    matches!(*ASSET_SOURCE, AssetSource::Filesystem(_))
}

/// Compute a short content hash of all assets for cache-busting versioned URLs.
pub(crate) fn asset_content_hash() -> String {
    use std::{collections::BTreeMap, hash::Hasher};

    let mut h = std::hash::DefaultHasher::new();

    match &*ASSET_SOURCE {
        AssetSource::Filesystem(dir) | AssetSource::External(dir) => {
            let mut files = BTreeMap::new();
            walk_dir_for_hash(dir, dir, &mut files);
            for (path, contents) in &files {
                h.write(path.as_bytes());
                h.write(contents);
            }
        },
        #[cfg(feature = "embedded-assets")]
        AssetSource::Embedded => {
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
            for (path, contents) in &files {
                h.write(path.as_bytes());
                h.write(contents);
            }
        },
        #[cfg(not(feature = "embedded-assets"))]
        AssetSource::Unavailable => {},
    }

    format!("{:016x}", h.finish())
}

/// Walk a filesystem directory for hashing, storing (relative_path, file_bytes)
/// pairs sorted by path.
fn walk_dir_for_hash(
    base: &FsPath,
    dir: &FsPath,
    out: &mut std::collections::BTreeMap<String, Vec<u8>>,
) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk_dir_for_hash(base, &path, out);
        } else if let Ok(bytes) = std::fs::read(&path) {
            let rel = path
                .strip_prefix(base)
                .unwrap_or(&path)
                .display()
                .to_string();
            out.insert(rel, bytes);
        }
    }
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

/// Read a file from a filesystem directory with path-traversal protection.
fn read_from_dir(dir: &std::path::Path, path: &str) -> Option<Vec<u8>> {
    let rel = FsPath::new(path);
    if rel.is_absolute() {
        return None;
    }

    if !rel
        .components()
        .all(|component| matches!(component, Component::Normal(_)))
    {
        return None;
    }

    std::fs::read(dir.join(rel)).ok()
}

/// Read an asset file using three-tier resolution.
fn read_asset(path: &str) -> Option<Vec<u8>> {
    match &*ASSET_SOURCE {
        AssetSource::Filesystem(dir) | AssetSource::External(dir) => read_from_dir(dir, path),
        #[cfg(feature = "embedded-assets")]
        AssetSource::Embedded => ASSETS.get_file(path).map(|f| f.contents().to_vec()),
        #[cfg(not(feature = "embedded-assets"))]
        AssetSource::Unavailable => None,
    }
}

/// Read raw asset bytes by path. Used by `share_render.rs` for the favicon.
pub fn read_asset_bytes(path: &str) -> Option<Vec<u8>> {
    read_asset(path)
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
        #[cfg(not(feature = "embedded-assets"))]
        None => {
            // When embedded-assets is disabled and no external dir is available,
            // provide a helpful error message.
            if matches!(*ASSET_SOURCE, AssetSource::Unavailable) {
                (
                    StatusCode::SERVICE_UNAVAILABLE,
                    "Web assets are not available. Install assets to /usr/share/moltis/web/ \
                     or set MOLTIS_SHARE_DIR to the directory containing them.",
                )
                    .into_response()
            } else {
                (StatusCode::NOT_FOUND, "not found").into_response()
            }
        },
        #[cfg(feature = "embedded-assets")]
        None => (StatusCode::NOT_FOUND, "not found").into_response(),
    }
}
