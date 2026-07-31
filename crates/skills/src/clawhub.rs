//! ClawHub registry client for searching and installing individual skills.
//!
//! Uses the public ClawHub REST API at `https://clawhub.ai/api/v1/`.
//! No authentication required for read operations. Rate limit: 180 req/min.

use std::path::{Component, Path};

use {
    cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt},
    cap_std::fs::{Dir, OpenOptions},
    serde::{Deserialize, Serialize},
};

use crate::{
    error::{Error, Result},
    manifest::ManifestStore,
    parse,
    types::{RepoEntry, SkillMetadata, SkillState},
};

const BASE_URL: &str = "https://clawhub.ai";
const USER_AGENT: &str = "moltis-skills";

// ── API response types ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResponse {
    pub results: Vec<SearchResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResult {
    #[serde(default)]
    pub score: f64,
    pub slug: String,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub summary: Option<String>,
    /// Millisecond timestamp.
    #[serde(default)]
    pub updated_at: Option<u64>,
    #[serde(default)]
    pub version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillInfoResponse {
    pub skill: SkillInfo,
    #[serde(default)]
    pub latest_version: Option<VersionInfo>,
    #[serde(default)]
    pub owner: Option<OwnerInfo>,
    #[serde(default)]
    pub moderation: Option<ModerationInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillInfo {
    pub slug: String,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub stats: Option<SkillStats>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillStats {
    #[serde(default)]
    pub downloads: u64,
    #[serde(default)]
    pub installs_all_time: u64,
    #[serde(default)]
    pub stars: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionInfo {
    pub version: String,
    #[serde(default)]
    pub changelog: Option<String>,
    #[serde(default)]
    pub license: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OwnerInfo {
    #[serde(default)]
    pub handle: Option<String>,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub image: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModerationInfo {
    #[serde(default)]
    pub is_suspicious: Option<bool>,
    #[serde(default)]
    pub verdict: Option<String>,
}

// ── Client ──────────────────────────────────────────────────────────────────

// ── Scan response types ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanResponse {
    #[serde(default)]
    pub security: Option<SecurityInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SecurityInfo {
    /// Overall status: `"clean"`, `"suspicious"`, `"malicious"`, etc.
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub has_warnings: bool,
    #[serde(default)]
    pub virustotal_url: Option<String>,
    #[serde(default)]
    pub scanners: Option<ScannerResults>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScannerResults {
    #[serde(default)]
    pub vt: Option<ScannerEntry>,
    #[serde(default)]
    pub llm: Option<ScannerEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScannerEntry {
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub verdict: Option<String>,
    #[serde(default)]
    pub analysis: Option<String>,
}

/// Maximum retries for rate-limited (429) responses.
const MAX_RETRIES: u32 = 3;
/// Base delay for exponential backoff (seconds).
const BACKOFF_BASE_SECS: u64 = 2;

pub struct ClawHubClient {
    client: reqwest::Client,
    base_url: String,
}

impl Default for ClawHubClient {
    fn default() -> Self {
        Self::new()
    }
}

impl ClawHubClient {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
            base_url: BASE_URL.to_string(),
        }
    }

    /// Send a GET request with retry on 429 (rate limit) using exponential backoff.
    ///
    /// Respects the `retry-after` header when present, otherwise uses exponential
    /// backoff: 2s, 4s, 8s.
    async fn get_with_retry(&self, url: &str, query: &[(&str, &str)]) -> Result<reqwest::Response> {
        let mut attempt = 0;
        loop {
            let resp = self
                .client
                .get(url)
                .query(query)
                .header("User-Agent", USER_AGENT)
                .send()
                .await?;

            if resp.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
                attempt += 1;
                if attempt > MAX_RETRIES {
                    return Err(Error::Install(format!(
                        "ClawHub rate limit exceeded after {MAX_RETRIES} retries"
                    )));
                }

                // Use retry-after header if present, otherwise exponential backoff.
                let wait_secs = resp
                    .headers()
                    .get("retry-after")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.parse::<u64>().ok())
                    .unwrap_or(BACKOFF_BASE_SECS.pow(attempt));

                tracing::debug!(attempt, wait_secs, "ClawHub rate limited (429), retrying");
                tokio::time::sleep(std::time::Duration::from_secs(wait_secs)).await;
                continue;
            }

            return Ok(resp);
        }
    }

    /// Search for skills on ClawHub.
    pub async fn search(&self, query: &str) -> Result<SearchResponse> {
        let url = format!("{}/api/v1/search", self.base_url);
        let resp = self.get_with_retry(&url, &[("q", query)]).await?;

        if !resp.status().is_success() {
            return Err(Error::Install(format!(
                "ClawHub search failed: HTTP {}",
                resp.status()
            )));
        }

        resp.json().await.map_err(Into::into)
    }

    /// Get metadata for a specific skill.
    pub async fn skill_info(&self, slug: &str) -> Result<SkillInfoResponse> {
        let url = format!("{}/api/v1/skills/{}", self.base_url, slug);
        let resp = self.get_with_retry(&url, &[]).await?;

        if !resp.status().is_success() {
            return Err(Error::Install(format!(
                "ClawHub skill info failed for '{}': HTTP {}",
                slug,
                resp.status()
            )));
        }

        resp.json().await.map_err(Into::into)
    }

    /// Get security scan results for a skill.
    pub async fn scan(&self, slug: &str) -> Result<ScanResponse> {
        let url = format!("{}/api/v1/skills/{}/scan", self.base_url, slug);
        let resp = self.get_with_retry(&url, &[]).await?;

        if !resp.status().is_success() {
            return Err(Error::Install(format!(
                "ClawHub scan failed for '{}': HTTP {}",
                slug,
                resp.status()
            )));
        }

        resp.json().await.map_err(Into::into)
    }

    /// Download a skill as a zip archive.
    pub async fn download_zip(&self, slug: &str, version: &str) -> Result<Vec<u8>> {
        let url = format!("{}/api/v1/download", self.base_url);
        let resp = self
            .get_with_retry(&url, &[("slug", slug), ("version", version)])
            .await?;

        if !resp.status().is_success() {
            return Err(Error::Install(format!(
                "ClawHub download failed for '{slug}@{version}': HTTP {}",
                resp.status()
            )));
        }

        let bytes = resp.bytes().await?;
        Ok(bytes.to_vec())
    }
}

// ── Enriched search results ─────────────────────────────────────────────────

/// Enriched search result with additional metadata from skill info lookups.
/// This is what we return to the frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnrichedSearchResult {
    pub score: f64,
    pub slug: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default)]
    pub downloads: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner_handle: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner_image: Option<String>,
    #[serde(default)]
    pub stars: u64,
}

impl From<SearchResult> for EnrichedSearchResult {
    fn from(r: SearchResult) -> Self {
        Self {
            score: r.score,
            slug: r.slug,
            display_name: r.display_name,
            summary: r.summary,
            updated_at: r.updated_at,
            version: r.version,
            downloads: 0,
            owner_handle: None,
            owner_image: None,
            stars: 0,
        }
    }
}

// ── Install from ClawHub ────────────────────────────────────────────────────

/// Install a single skill from ClawHub by slug.
///
/// Downloads the skill zip archive, extracts all files (SKILL.md, scripts,
/// templates, references, etc.) to `install_dir/clawhub-<slug>/`, and
/// records the skill in the manifest.
pub async fn install_from_clawhub(slug: &str, install_dir: &Path) -> Result<Vec<SkillMetadata>> {
    validate_slug(slug)?;

    let client = ClawHubClient::new();

    // Get skill metadata and version.
    let info = client.skill_info(slug).await?;
    let version = info
        .latest_version
        .as_ref()
        .map(|v| v.version.clone())
        .ok_or_else(|| Error::Install(format!("skill '{slug}' has no published version")))?;

    let dir_name = format!("clawhub-{slug}");
    let target = install_dir.join(&dir_name);

    tokio::fs::create_dir_all(install_dir).await?;
    let install_root = Dir::open_ambient_dir(install_dir, cap_std::ambient_authority())?;
    match install_root.symlink_metadata(&dir_name) {
        Ok(metadata) if metadata.is_dir() => install_root.remove_dir_all(&dir_name)?,
        Ok(_) => {
            return Err(Error::Install(format!(
                "ClawHub install destination is not a real directory: {target:?}"
            )));
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {},
        Err(error) => return Err(Error::Io(error)),
    }
    install_root.create_dir(&dir_name)?;
    let target_dir = install_root.open_dir_nofollow(&dir_name)?;

    // Download zip archive.
    let zip_bytes = client.download_zip(slug, &version).await?;

    // Extract zip on a blocking thread (zip I/O is synchronous).
    let extraction =
        tokio::task::spawn_blocking(move || extract_zip_into(&zip_bytes, &target_dir)).await;
    finish_extraction(extraction, &install_root, Path::new(&dir_name))?;

    let mut options = OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No);
    let metadata_dir = install_root.open_dir_nofollow(&dir_name)?;
    let skill_file = match metadata_dir.open_with("SKILL.md", &options) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let _ = install_root.remove_dir_all(&dir_name);
            return Err(Error::Install(format!(
                "ClawHub skill '{slug}' has no SKILL.md"
            )));
        },
        Err(error) => return Err(Error::Io(error)),
    };
    let mut skill_file = tokio::fs::File::from_std(skill_file.into_std());
    let mut content = String::new();
    tokio::io::AsyncReadExt::read_to_string(&mut skill_file, &mut content).await?;
    let metadata = parse::parse_metadata(&content, &target)?;

    let skill_states = vec![SkillState {
        name: metadata.name.clone(),
        relative_path: dir_name.clone(),
        trusted: false,
        enabled: false,
    }];

    // Write manifest.
    let manifest_path = ManifestStore::default_path()?;
    let store = ManifestStore::new(manifest_path);
    let mut manifest = store.load()?;

    // Remove existing entry if re-installing.
    let source_key = clawhub_source_key(slug);
    manifest.remove_repo(&source_key);

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;

    manifest.add_repo(RepoEntry {
        source: source_key,
        repo_name: dir_name,
        installed_at_ms: now,
        commit_sha: Some(version),
        format: crate::formats::PluginFormat::Skill,
        quarantined: false,
        quarantine_reason: None,
        provenance: None,
        skills: skill_states,
    });
    store.save(&manifest)?;

    tracing::info!(%slug, name = %metadata.name, "installed skill from ClawHub");
    Ok(vec![metadata])
}

fn finish_extraction(
    extraction: std::result::Result<Result<()>, tokio::task::JoinError>,
    install_root: &Dir,
    target: &Path,
) -> Result<()> {
    let error = match extraction {
        Ok(Ok(())) => return Ok(()),
        Ok(Err(error)) => error,
        Err(error) => Error::Join(error),
    };

    let _ = install_root.remove_dir_all(target);
    Err(error)
}

/// Build the manifest source key for a ClawHub skill.
pub fn clawhub_source_key(slug: &str) -> String {
    format!("clawhub:{slug}")
}

/// Check if a manifest source key is a ClawHub skill.
pub fn is_clawhub_source(source: &str) -> bool {
    source.starts_with("clawhub:")
}

pub fn validate_slug(slug: &str) -> Result<()> {
    if slug.is_empty() || slug.len() > 128 {
        return Err(Error::Install(
            "invalid ClawHub slug: empty or too long".into(),
        ));
    }
    if slug
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        Ok(())
    } else {
        Err(Error::Install(format!(
            "invalid ClawHub slug: '{slug}' (only alphanumeric, hyphens, underscores allowed)"
        )))
    }
}

/// Extract a zip archive into a target directory with security checks.
#[cfg(test)]
fn extract_zip(zip_bytes: &[u8], target: &Path) -> Result<()> {
    let parent = target
        .parent()
        .ok_or_else(|| Error::Install("zip target has no parent directory".into()))?;
    let file_name = target
        .file_name()
        .ok_or_else(|| Error::Install("zip target has no directory name".into()))?;
    let parent_dir = Dir::open_ambient_dir(parent, cap_std::ambient_authority())?;
    let target_dir = parent_dir.open_dir_nofollow(file_name)?;
    extract_zip_into(zip_bytes, &target_dir)
}

fn extract_zip_into(zip_bytes: &[u8], target_dir: &Dir) -> Result<()> {
    let reader = std::io::Cursor::new(zip_bytes);
    let mut archive = zip::ZipArchive::new(reader)
        .map_err(|e| Error::Install(format!("invalid zip archive: {e}")))?;

    for i in 0..archive.len() {
        let mut file = archive
            .by_index(i)
            .map_err(|e| Error::Install(format!("zip entry error: {e}")))?;

        let raw_name = file.name().to_string();
        if file.is_symlink() {
            return Err(Error::Install(format!(
                "zip archive contains unsupported symlink entry: {raw_name:?}"
            )));
        }
        validate_zip_unix_mode(&raw_name, file.is_dir(), file.unix_mode())?;

        let relative = file.enclosed_name().ok_or_else(|| {
            Error::Install(format!("zip archive contains unsafe path: {raw_name:?}"))
        })?;
        if relative.as_os_str().is_empty()
            || !relative
                .components()
                .all(|component| matches!(component, Component::Normal(_)))
        {
            return Err(Error::Install(format!(
                "zip archive contains non-normal path: {raw_name:?}"
            )));
        }

        if file.is_dir() {
            create_zip_directories(target_dir, &relative)?;
            continue;
        }

        let parent =
            create_zip_directories(target_dir, relative.parent().unwrap_or(Path::new("")))?;
        let file_name = relative
            .file_name()
            .ok_or_else(|| Error::Install(format!("zip entry has no filename: {raw_name:?}")))?;
        let mut options = OpenOptions::new();
        options
            .write(true)
            .create_new(true)
            .follow(FollowSymlinks::No);
        let mut dest_file = parent.open_with(file_name, &options).map_err(|error| {
            Error::Install(format!(
                "cannot securely create zip destination {raw_name:?}: {error}"
            ))
        })?;
        std::io::copy(&mut file, &mut dest_file)?;
    }
    Ok(())
}

fn validate_zip_unix_mode(
    raw_name: &str,
    is_directory: bool,
    unix_mode: Option<u32>,
) -> Result<()> {
    const FILE_TYPE_MASK: u32 = 0o170000;
    const REGULAR_FILE: u32 = 0o100000;
    const DIRECTORY: u32 = 0o040000;

    let Some(mode) = unix_mode else {
        return Ok(());
    };
    let file_type = mode & FILE_TYPE_MASK;
    let supported = if is_directory {
        matches!(file_type, 0 | DIRECTORY)
    } else {
        matches!(file_type, 0 | REGULAR_FILE)
    };
    if !supported {
        return Err(Error::Install(format!(
            "zip archive contains unsupported special entry: {raw_name:?} (Unix mode {mode:#o})"
        )));
    }

    Ok(())
}

fn create_zip_directories(target: &Dir, relative: &Path) -> Result<Dir> {
    let mut current = target.try_clone()?;
    for component in relative.components() {
        let Component::Normal(segment) = component else {
            return Err(Error::Install("zip entry contains non-normal path".into()));
        };

        match current.create_dir(segment) {
            Ok(()) => {},
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {},
            Err(error) => return Err(Error::Io(error)),
        }
        current = current.open_dir_nofollow(segment).map_err(|error| {
            Error::Install(format!("zip entry parent is not a real directory: {error}"))
        })?;
    }

    Ok(current)
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use {
        super::*,
        std::io::{Cursor, Write},
    };

    use zip::{ZipWriter, write::SimpleFileOptions};

    fn build_zip(build: impl FnOnce(&mut ZipWriter<Cursor<Vec<u8>>>)) -> Vec<u8> {
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        build(&mut writer);
        writer.finish().unwrap().into_inner()
    }

    fn add_zip_file(writer: &mut ZipWriter<Cursor<Vec<u8>>>, name: &str, contents: &[u8]) {
        writer
            .start_file(name, SimpleFileOptions::default())
            .unwrap();
        writer.write_all(contents).unwrap();
    }

    #[test]
    fn clawhub_source_key_format() {
        assert_eq!(clawhub_source_key("my-skill"), "clawhub:my-skill");
    }

    #[test]
    fn is_clawhub_source_matches() {
        assert!(is_clawhub_source("clawhub:my-skill"));
        assert!(!is_clawhub_source("garrytan/gbrain"));
        assert!(!is_clawhub_source("clawhub"));
    }

    #[test]
    fn validate_slug_accepts_valid() {
        assert!(validate_slug("my-skill").is_ok());
        assert!(validate_slug("skill_v2").is_ok());
        assert!(validate_slug("arxiv").is_ok());
    }

    #[test]
    fn validate_slug_rejects_invalid() {
        assert!(validate_slug("").is_err());
        assert!(validate_slug("../etc/passwd").is_err());
        assert!(validate_slug("foo bar").is_err());
        assert!(validate_slug("foo/bar").is_err());
    }

    #[test]
    fn extract_zip_writes_normal_nested_files() {
        let zip = build_zip(|writer| {
            writer
                .add_directory("scripts/", SimpleFileOptions::default())
                .unwrap();
            add_zip_file(writer, "scripts/run.sh", b"#!/bin/sh\n");
            add_zip_file(writer, "SKILL.md", b"# Test\n");
        });
        let temp_dir = tempfile::tempdir().unwrap();
        let target = temp_dir.path().join("target");
        std::fs::create_dir(&target).unwrap();

        extract_zip(&zip, &target).unwrap();

        assert_eq!(
            std::fs::read(target.join("scripts/run.sh")).unwrap(),
            b"#!/bin/sh\n"
        );
        assert_eq!(std::fs::read(target.join("SKILL.md")).unwrap(), b"# Test\n");
    }

    #[test]
    fn extract_zip_rejects_parent_traversal() {
        let zip = build_zip(|writer| add_zip_file(writer, "../outside.txt", b"escaped"));
        let temp_dir = tempfile::tempdir().unwrap();
        let target = temp_dir.path().join("target");
        std::fs::create_dir(&target).unwrap();

        assert!(extract_zip(&zip, &target).is_err());
        assert!(!temp_dir.path().join("outside.txt").exists());
    }

    #[test]
    fn extract_zip_rejects_symlink_entries() {
        let zip = build_zip(|writer| {
            writer
                .add_symlink("link", "../outside", SimpleFileOptions::default())
                .unwrap();
        });
        let temp_dir = tempfile::tempdir().unwrap();
        let target = temp_dir.path().join("target");
        std::fs::create_dir(&target).unwrap();

        let error = extract_zip(&zip, &target).unwrap_err();
        assert!(error.to_string().contains("unsupported symlink entry"));
        assert!(!target.join("link").exists());
    }

    #[test]
    fn zip_unix_mode_validation_rejects_special_file_types() {
        assert!(validate_zip_unix_mode("file", false, None).is_ok());
        assert!(validate_zip_unix_mode("file", false, Some(0o644)).is_ok());
        assert!(validate_zip_unix_mode("file", false, Some(0o100644)).is_ok());
        assert!(validate_zip_unix_mode("dir/", true, Some(0o040755)).is_ok());

        for mode in [0o010644, 0o020644, 0o060644, 0o120777, 0o140777] {
            assert!(
                validate_zip_unix_mode("special", false, Some(mode)).is_err(),
                "special Unix mode should be rejected: {mode:#o}"
            );
        }
    }

    #[test]
    fn extract_zip_rejects_symlink_then_nested_file_bypass() {
        let zip = build_zip(|writer| {
            writer
                .add_symlink("link", "../outside", SimpleFileOptions::default())
                .unwrap();
            add_zip_file(writer, "link/escaped.txt", b"escaped");
        });
        let temp_dir = tempfile::tempdir().unwrap();
        let target = temp_dir.path().join("target");
        let outside = temp_dir.path().join("outside");
        std::fs::create_dir(&target).unwrap();
        std::fs::create_dir(&outside).unwrap();

        assert!(extract_zip(&zip, &target).is_err());
        assert!(!outside.join("escaped.txt").exists());
        assert!(!target.join("link").exists());
    }

    #[cfg(unix)]
    #[test]
    fn extract_zip_rejects_preexisting_symlink_ancestry() {
        let zip = build_zip(|writer| add_zip_file(writer, "link/escaped.txt", b"escaped"));
        let temp_dir = tempfile::tempdir().unwrap();
        let target = temp_dir.path().join("target");
        let outside = temp_dir.path().join("outside");
        std::fs::create_dir(&target).unwrap();
        std::fs::create_dir(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, target.join("link")).unwrap();

        let error = extract_zip(&zip, &target).unwrap_err();
        assert!(error.to_string().contains("not a real directory"));
        assert!(!outside.join("escaped.txt").exists());
    }

    #[cfg(unix)]
    #[test]
    fn extract_zip_rejects_preexisting_symlink_destination() {
        let zip = build_zip(|writer| add_zip_file(writer, "SKILL.md", b"replaced"));
        let temp_dir = tempfile::tempdir().unwrap();
        let target = temp_dir.path().join("target");
        let outside = temp_dir.path().join("outside.md");
        std::fs::create_dir(&target).unwrap();
        std::fs::write(&outside, b"outside").unwrap();
        std::os::unix::fs::symlink(&outside, target.join("SKILL.md")).unwrap();

        assert!(extract_zip(&zip, &target).is_err());
        assert_eq!(std::fs::read(&outside).unwrap(), b"outside");
    }

    #[test]
    fn extract_zip_rejects_preexisting_file_destination() {
        let zip = build_zip(|writer| add_zip_file(writer, "SKILL.md", b"replaced"));
        let temp_dir = tempfile::tempdir().unwrap();
        let target = temp_dir.path().join("target");
        std::fs::create_dir(&target).unwrap();
        std::fs::write(target.join("SKILL.md"), b"original").unwrap();

        assert!(extract_zip(&zip, &target).is_err());
        assert_eq!(std::fs::read(target.join("SKILL.md")).unwrap(), b"original");
    }

    #[tokio::test]
    async fn failed_extraction_removes_partially_extracted_target() {
        let zip = build_zip(|writer| {
            add_zip_file(writer, "partial.txt", b"partial");
            add_zip_file(writer, "../outside.txt", b"escaped");
        });
        let temp_dir = tempfile::tempdir().unwrap();
        let target_name = Path::new("clawhub-test");
        let target = temp_dir.path().join(target_name);
        std::fs::create_dir(&target).unwrap();
        let install_root =
            Dir::open_ambient_dir(temp_dir.path(), cap_std::ambient_authority()).unwrap();
        let target_dir = install_root.open_dir_nofollow(target_name).unwrap();

        let extraction =
            tokio::task::spawn_blocking(move || extract_zip_into(&zip, &target_dir)).await;
        let error = finish_extraction(extraction, &install_root, target_name).unwrap_err();

        assert!(error.to_string().contains("unsafe path"));
        assert!(!target.exists());
    }

    #[tokio::test]
    async fn extraction_join_error_removes_partially_extracted_target() {
        let temp_dir = tempfile::tempdir().unwrap();
        let target_name = Path::new("clawhub-test");
        let target = temp_dir.path().join(target_name);
        std::fs::create_dir(&target).unwrap();
        std::fs::write(target.join("partial.txt"), b"partial").unwrap();
        let install_root =
            Dir::open_ambient_dir(temp_dir.path(), cap_std::ambient_authority()).unwrap();
        let extraction =
            tokio::task::spawn_blocking(|| -> Result<()> { panic!("simulated extraction panic") })
                .await;

        let error = finish_extraction(extraction, &install_root, target_name).unwrap_err();

        assert!(matches!(error, Error::Join(_)));
        assert!(!target.exists());
    }

    #[cfg(unix)]
    #[test]
    fn extract_zip_rejects_symlink_install_root() {
        let zip = build_zip(|writer| add_zip_file(writer, "SKILL.md", b"escaped"));
        let temp_dir = tempfile::tempdir().unwrap();
        let target = temp_dir.path().join("target");
        let outside = temp_dir.path().join("outside");
        std::fs::create_dir(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, &target).unwrap();

        assert!(extract_zip(&zip, &target).is_err());
        assert!(!outside.join("SKILL.md").exists());
    }

    /// Test with the actual JSON shape returned by the ClawHub /api/v1/search endpoint.
    #[test]
    fn search_response_deserialises_real_format() {
        let json = r#"{"results":[{"score":3.54,"slug":"csv-handler","displayName":"Csv Handler","summary":"Handle CSV files","version":null,"updatedAt":1772056835938}]}"#;
        let resp: SearchResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.results.len(), 1);
        assert_eq!(resp.results[0].slug, "csv-handler");
        assert_eq!(resp.results[0].display_name.as_deref(), Some("Csv Handler"));
        assert_eq!(resp.results[0].updated_at, Some(1772056835938));
        assert!(resp.results[0].version.is_none());
    }

    /// Test with the actual JSON shape returned by the ClawHub /api/v1/skills/<slug> endpoint.
    #[test]
    fn skill_info_response_deserialises_real_format() {
        let json = r#"{
            "skill": {
                "slug": "csv-handler",
                "displayName": "Csv Handler",
                "summary": "Handle CSV files",
                "stats": { "downloads": 2185, "installsAllTime": 12, "stars": 3, "comments": 0, "versions": 2 }
            },
            "latestVersion": { "version": "2.1.0", "changelog": "Added features", "license": null },
            "owner": { "handle": "datadrivenconstruction", "displayName": "datadrivenconstruction", "image": "https://avatars.githubusercontent.com/u/94158709?v=4" },
            "moderation": null
        }"#;
        let resp: SkillInfoResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.skill.slug, "csv-handler");
        assert_eq!(resp.skill.stats.as_ref().unwrap().downloads, 2185);
        assert_eq!(resp.skill.stats.as_ref().unwrap().stars, 3);
        assert_eq!(resp.latest_version.as_ref().unwrap().version, "2.1.0");
        assert_eq!(
            resp.owner.as_ref().unwrap().handle.as_deref(),
            Some("datadrivenconstruction")
        );
        assert!(resp.moderation.is_none());
    }

    #[test]
    fn enriched_result_from_search_result() {
        let sr = SearchResult {
            score: 3.5,
            slug: "test".into(),
            display_name: Some("Test Skill".into()),
            summary: Some("A test".into()),
            updated_at: Some(1234567890000),
            version: None,
        };
        let enriched: EnrichedSearchResult = sr.into();
        assert_eq!(enriched.slug, "test");
        assert_eq!(enriched.downloads, 0);
        assert!(enriched.owner_handle.is_none());
    }

    /// Integration test: hit the real ClawHub search API.
    #[tokio::test]
    async fn live_search_returns_results() {
        let client = ClawHubClient::new();
        let resp = client.search("csv").await;
        match resp {
            Ok(r) => {
                assert!(
                    !r.results.is_empty(),
                    "search for 'csv' should return results"
                );
                let first = &r.results[0];
                assert!(!first.slug.is_empty());
                assert!(first.display_name.is_some());
            },
            Err(e) => {
                // Network errors are ok in CI (no internet), but print for debugging.
                eprintln!("live search test skipped (network error): {e}");
            },
        }
    }

    /// Integration test: hit the real ClawHub scan API.
    #[tokio::test]
    async fn live_scan_returns_security_data() {
        let client = ClawHubClient::new();
        let resp = client.scan("csv-handler").await;
        match resp {
            Ok(scan) => {
                let sec = scan.security.expect("should have security data");
                assert!(
                    sec.status.is_some(),
                    "scan should have a status (clean/suspicious)"
                );
                assert!(sec.scanners.is_some(), "scan should have scanner results");
                let scanners = sec.scanners.unwrap();
                assert!(scanners.vt.is_some(), "should have VirusTotal results");
            },
            Err(e) => {
                eprintln!("live scan test skipped (network error): {e}");
            },
        }
    }

    /// Integration test: hit the real ClawHub skill info API.
    #[tokio::test]
    async fn live_skill_info_returns_metadata() {
        let client = ClawHubClient::new();
        let resp = client.skill_info("csv-handler").await;
        match resp {
            Ok(info) => {
                assert_eq!(info.skill.slug, "csv-handler");
                assert!(info.latest_version.is_some());
                assert!(info.owner.is_some());
            },
            Err(e) => {
                eprintln!("live skill_info test skipped (network error): {e}");
            },
        }
    }
}
