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

#[cfg(feature = "embedded-assets")]
use tracing::warn;
use {
    axum::{extract::Path, http::StatusCode, response::IntoResponse},
    serde::Serialize,
    tracing::info,
};

#[cfg(feature = "embedded-assets")]
use tracing::warn;

// ── Embedded assets (feature-gated) ─────────────────────────────────────────

#[cfg(feature = "embedded-assets")]
static ASSETS: include_dir::Dir = include_dir::include_dir!("$CARGO_MANIFEST_DIR/src/assets");

// Fail compilation with a clear message if style.css hasn't been generated.
// Run `just build-css` (or `cd crates/web/ui && ./build.sh`) to generate it.
#[cfg(feature = "embedded-assets")]
const _: &str = include_str!("assets/css/style.css");

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

struct AssetState {
    source: AssetSource,
    hash: String,
    fallback_reason: Option<&'static str>,
    #[cfg(feature = "embedded-assets")]
    external_hash: Option<String>,
    #[cfg(feature = "embedded-assets")]
    embedded_hash: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AssetVersionInfo<'a> {
    moltis_version: &'static str,
    asset_hash: &'a str,
    asset_source: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    fallback_reason: Option<&'static str>,
    #[cfg(feature = "embedded-assets")]
    #[serde(skip_serializing_if = "Option::is_none")]
    external_asset_hash: Option<&'a str>,
    #[cfg(feature = "embedded-assets")]
    #[serde(skip_serializing_if = "Option::is_none")]
    embedded_asset_hash: Option<&'a str>,
}

impl AssetState {
    fn from_source(source: AssetSource) -> Self {
        let hash = hash_for_source(&source);
        Self {
            source,
            hash,
            fallback_reason: None,
            #[cfg(feature = "embedded-assets")]
            external_hash: None,
            #[cfg(feature = "embedded-assets")]
            embedded_hash: None,
        }
    }

    #[cfg(feature = "embedded-assets")]
    fn embedded_fallback(
        embedded_hash: String,
        external_hash: String,
        fallback_reason: &'static str,
    ) -> Self {
        Self {
            source: AssetSource::Embedded,
            hash: embedded_hash.clone(),
            fallback_reason: Some(fallback_reason),
            external_hash: Some(external_hash),
            embedded_hash: Some(embedded_hash),
        }
    }

    fn source_name(&self) -> &'static str {
        match &self.source {
            AssetSource::Filesystem(_) => "filesystem",
            AssetSource::External(_) => "external",
            #[cfg(feature = "embedded-assets")]
            AssetSource::Embedded => "embedded",
            #[cfg(not(feature = "embedded-assets"))]
            AssetSource::Unavailable => "unavailable",
        }
    }

    fn version_info(&self) -> AssetVersionInfo<'_> {
        AssetVersionInfo {
            moltis_version: moltis_config::VERSION,
            asset_hash: &self.hash,
            asset_source: self.source_name(),
            fallback_reason: self.fallback_reason,
            #[cfg(feature = "embedded-assets")]
            external_asset_hash: self.external_hash.as_deref(),
            #[cfg(feature = "embedded-assets")]
            embedded_asset_hash: self.embedded_hash.as_deref(),
        }
    }
}

static ASSET_STATE: LazyLock<AssetState> = LazyLock::new(resolve_asset_state);

fn resolve_asset_state() -> AssetState {
    // 1. Explicit env var
    let explicit_dir = std::env::var("MOLTIS_ASSETS_DIR").ok().map(PathBuf::from);

    // 2. Auto-detect cargo source tree (dev mode)
    let cargo_dir = Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/assets"));

    // 3. External share directory
    resolve_asset_state_with_paths(explicit_dir, cargo_dir, moltis_config::share_dir())
}

fn resolve_asset_state_with_paths(
    explicit_dir: Option<PathBuf>,
    cargo_dir: Option<PathBuf>,
    share_dir: Option<PathBuf>,
) -> AssetState {
    if let Some(dir) = explicit_dir.filter(|dir| dir.is_dir()) {
        info!("Serving assets from filesystem: {}", dir.display());
        return AssetState::from_source(AssetSource::Filesystem(dir));
    }

    if let Some(dir) = cargo_dir.filter(|dir| dir.is_dir()) {
        info!("Serving assets from filesystem: {}", dir.display());
        return AssetState::from_source(AssetSource::Filesystem(dir));
    }

    if let Some(share) = share_dir {
        let web_dir = share.join("web");
        if web_dir.is_dir() {
            #[cfg(feature = "embedded-assets")]
            {
                let external_hash = hash_filesystem_dir(&web_dir);
                let embedded_hash = hash_embedded_assets();
                if external_hash == embedded_hash {
                    info!(
                        "Serving assets from external share dir: {}",
                        web_dir.display()
                    );
                    return AssetState {
                        source: AssetSource::External(web_dir),
                        hash: external_hash.clone(),
                        fallback_reason: None,
                        external_hash: Some(external_hash),
                        embedded_hash: Some(embedded_hash),
                    };
                }

                warn!(
                    external_dir = %web_dir.display(),
                    external_hash = %external_hash,
                    embedded_hash = %embedded_hash,
                    "External web assets differ from the embedded UI, serving embedded assets instead"
                );
                return AssetState::embedded_fallback(
                    embedded_hash,
                    external_hash,
                    "external-assets-mismatch",
                );
            }

            #[cfg(not(feature = "embedded-assets"))]
            {
                info!(
                    "Serving assets from external share dir: {}",
                    web_dir.display()
                );
                return AssetState::from_source(AssetSource::External(web_dir));
            }
        }
    }

    // 4. Embedded fallback (or unavailable)
    #[cfg(feature = "embedded-assets")]
    {
        info!("Serving assets from embedded binary");
        AssetState::from_source(AssetSource::Embedded)
    }
    #[cfg(not(feature = "embedded-assets"))]
    {
        info!("No asset source available (embedded-assets feature disabled)");
        AssetState::from_source(AssetSource::Unavailable)
    }
}

/// Whether we're serving from the filesystem (dev mode) or embedded/external (release).
pub(crate) fn is_dev_assets() -> bool {
    matches!(&ASSET_STATE.source, AssetSource::Filesystem(_))
}

/// Compute a short content hash of all assets for cache-busting versioned URLs.
pub(crate) fn asset_content_hash() -> String {
    ASSET_STATE.hash.clone()
}

fn hash_for_source(source: &AssetSource) -> String {
    match source {
        AssetSource::Filesystem(dir) | AssetSource::External(dir) => hash_filesystem_dir(dir),
        #[cfg(feature = "embedded-assets")]
        AssetSource::Embedded => hash_embedded_assets(),
        #[cfg(not(feature = "embedded-assets"))]
        AssetSource::Unavailable => String::new(),
    }
}

fn hash_filesystem_dir(dir: &FsPath) -> String {
    let mut files = std::collections::BTreeMap::new();
    walk_dir_for_hash(dir, dir, &mut files);
    hash_file_map(
        files
            .iter()
            .map(|(path, bytes)| (path.as_str(), bytes.as_slice())),
    )
}

#[cfg(feature = "embedded-assets")]
fn hash_embedded_assets() -> String {
    let mut files = std::collections::BTreeMap::new();
    let mut stack: Vec<&include_dir::Dir<'_>> = vec![&ASSETS];
    while let Some(dir) = stack.pop() {
        for file in dir.files() {
            files.insert(file.path().display().to_string(), file.contents());
        }
        for sub in dir.dirs() {
            stack.push(sub);
        }
    }
    hash_file_map(files.iter().map(|(path, bytes)| (path.as_str(), *bytes)))
}

fn hash_file_map<'a>(files: impl IntoIterator<Item = (&'a str, &'a [u8])>) -> String {
    use std::hash::Hasher;

    let mut hasher = std::hash::DefaultHasher::new();
    for (path, contents) in files {
        hasher.write(path.as_bytes());
        hasher.write(contents);
    }
    format!("{:016x}", hasher.finish())
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
    if path == "version.json" {
        return serde_json::to_vec(&ASSET_STATE.version_info()).ok();
    }

    match &ASSET_STATE.source {
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
            if matches!(&ASSET_STATE.source, AssetSource::Unavailable) {
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

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::{Path, PathBuf},
    };

    use tempfile::TempDir;

    use crate::assets::{hash_embedded_assets, resolve_asset_state_with_paths};

    fn copy_dir_recursive(src: &Path, dst: &Path) {
        let entries = fs::read_dir(src).unwrap_or_else(|e| panic!("read_dir failed: {e}"));
        for entry in entries {
            let entry = entry.unwrap_or_else(|e| panic!("dir entry failed: {e}"));
            let src_path = entry.path();
            let dst_path = dst.join(entry.file_name());
            if src_path.is_dir() {
                fs::create_dir_all(&dst_path)
                    .unwrap_or_else(|e| panic!("create_dir_all failed: {e}"));
                copy_dir_recursive(&src_path, &dst_path);
            } else {
                fs::copy(&src_path, &dst_path)
                    .unwrap_or_else(|e| panic!("copy failed for {}: {e}", src_path.display()));
            }
        }
    }

    #[test]
    fn prefers_embedded_assets_when_external_share_dir_is_stale() {
        let share_dir = TempDir::new().unwrap_or_else(|e| panic!("tempdir failed: {e}"));
        let web_dir = share_dir.path().join("web");
        fs::create_dir_all(&web_dir).unwrap_or_else(|e| panic!("create_dir_all failed: {e}"));
        fs::write(
            web_dir.join("index.html"),
            "<!doctype html><title>stale</title>",
        )
        .unwrap_or_else(|e| panic!("write failed: {e}"));

        let state =
            resolve_asset_state_with_paths(None, None, Some(share_dir.path().to_path_buf()));

        assert_eq!(state.source_name(), "embedded");
        assert_eq!(state.fallback_reason, Some("external-assets-mismatch"));
        assert_eq!(state.hash, hash_embedded_assets());
    }

    #[test]
    fn keeps_external_assets_when_they_match_embedded_bundle() {
        let share_dir = TempDir::new().unwrap_or_else(|e| panic!("tempdir failed: {e}"));
        let web_dir = share_dir.path().join("web");
        fs::create_dir_all(&web_dir).unwrap_or_else(|e| panic!("create_dir_all failed: {e}"));
        copy_dir_recursive(
            &PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/assets"),
            &web_dir,
        );

        let state =
            resolve_asset_state_with_paths(None, None, Some(share_dir.path().to_path_buf()));

        assert_eq!(state.source_name(), "external");
        assert_eq!(state.hash, hash_embedded_assets());
        assert_eq!(state.fallback_reason, None);
    }

    #[test]
    fn version_json_reports_asset_source_and_hash() {
        let share_dir = TempDir::new().unwrap_or_else(|e| panic!("tempdir failed: {e}"));
        let web_dir = share_dir.path().join("web");
        fs::create_dir_all(&web_dir).unwrap_or_else(|e| panic!("create_dir_all failed: {e}"));
        fs::write(
            web_dir.join("index.html"),
            "<!doctype html><title>stale</title>",
        )
        .unwrap_or_else(|e| panic!("write failed: {e}"));

        let state =
            resolve_asset_state_with_paths(None, None, Some(share_dir.path().to_path_buf()));
        let version_json = serde_json::to_value(state.version_info())
            .unwrap_or_else(|e| panic!("json failed: {e}"));

        assert_eq!(version_json["assetSource"], "embedded");
        assert_eq!(version_json["fallbackReason"], "external-assets-mismatch");
        assert_eq!(version_json["assetHash"], hash_embedded_assets());
        assert_eq!(version_json["moltisVersion"], moltis_config::VERSION);
    }
}
