//! ClawHub registry client for searching and installing individual skills.
//!
//! Uses the public ClawHub REST API at `https://clawhub.ai/api/v1/`.
//! No authentication required for read operations. Rate limit: 180 req/min.

use std::path::Path;

use serde::{Deserialize, Serialize};

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
    #[serde(default)]
    pub updated_at: Option<String>,
    #[serde(default)]
    pub downloads: Option<u64>,
    #[serde(default)]
    pub owner_handle: Option<String>,
    #[serde(default)]
    pub verified: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillInfoResponse {
    pub skill: SkillInfo,
    #[serde(default)]
    pub latest_version: Option<VersionInfo>,
    #[serde(default)]
    pub owner: Option<OwnerInfo>,
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
    pub tags: Vec<String>,
    #[serde(default)]
    pub stats: Option<SkillStats>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillStats {
    #[serde(default)]
    pub downloads: u64,
    #[serde(default)]
    pub installs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionInfo {
    pub version: String,
    #[serde(default)]
    pub changelog: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OwnerInfo {
    #[serde(default)]
    pub handle: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionDetailResponse {
    pub files: Vec<FileEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileEntry {
    pub path: String,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub sha256: Option<String>,
}

// ── Client ──────────────────────────────────────────────────────────────────

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

    /// Search for skills on ClawHub.
    pub async fn search(&self, query: &str) -> Result<SearchResponse> {
        let url = format!("{}/api/v1/search", self.base_url);
        let resp = self
            .client
            .get(&url)
            .query(&[("q", query)])
            .header("User-Agent", USER_AGENT)
            .send()
            .await?;

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
        let resp = self
            .client
            .get(&url)
            .header("User-Agent", USER_AGENT)
            .send()
            .await?;

        if !resp.status().is_success() {
            return Err(Error::Install(format!(
                "ClawHub skill info failed for '{}': HTTP {}",
                slug,
                resp.status()
            )));
        }

        resp.json().await.map_err(Into::into)
    }

    /// List files in a specific version of a skill.
    pub async fn version_files(&self, slug: &str, version: &str) -> Result<Vec<FileEntry>> {
        let url = format!(
            "{}/api/v1/skills/{}/versions/{}",
            self.base_url, slug, version
        );
        let resp = self
            .client
            .get(&url)
            .header("User-Agent", USER_AGENT)
            .send()
            .await?;

        if !resp.status().is_success() {
            return Err(Error::Install(format!(
                "ClawHub version files failed for '{slug}@{version}': HTTP {}",
                resp.status()
            )));
        }

        let detail: VersionDetailResponse = resp.json().await?;
        Ok(detail.files)
    }

    /// Download a skill as a zip archive.
    pub async fn download_zip(&self, slug: &str, version: &str) -> Result<Vec<u8>> {
        let url = format!("{}/api/v1/download", self.base_url);
        let resp = self
            .client
            .get(&url)
            .query(&[("slug", slug), ("version", version)])
            .header("User-Agent", USER_AGENT)
            .send()
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

    // Remove existing if re-installing.
    if target.exists() {
        tokio::fs::remove_dir_all(&target).await?;
    }
    tokio::fs::create_dir_all(&target).await?;

    // Download zip archive.
    let zip_bytes = client.download_zip(slug, &version).await?;

    // Extract zip on a blocking thread (zip I/O is synchronous).
    let target_owned = target.clone();
    tokio::task::spawn_blocking(move || extract_zip(&zip_bytes, &target_owned)).await??;

    // Parse SKILL.md to get metadata.
    let skill_md_path = target.join("SKILL.md");
    if !skill_md_path.exists() {
        let _ = tokio::fs::remove_dir_all(&target).await;
        return Err(Error::Install(format!(
            "ClawHub skill '{slug}' has no SKILL.md"
        )));
    }

    let content = tokio::fs::read_to_string(&skill_md_path).await?;
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

/// Extract a zip archive into a target directory with security checks.
fn extract_zip(zip_bytes: &[u8], target: &Path) -> Result<()> {
    use std::io::Read;

    let reader = std::io::Cursor::new(zip_bytes);
    let mut archive = zip::ZipArchive::new(reader)
        .map_err(|e| Error::Install(format!("invalid zip archive: {e}")))?;

    let canonical_target = std::fs::canonicalize(target)?;

    for i in 0..archive.len() {
        let mut file = archive
            .by_index(i)
            .map_err(|e| Error::Install(format!("zip entry error: {e}")))?;

        let raw_name = file.name().to_string();

        // Security: reject symlinks, absolute paths, path traversal.
        if raw_name.contains("..") || raw_name.starts_with('/') {
            tracing::warn!(path = %raw_name, "skipping unsafe zip entry");
            continue;
        }

        // Strip leading directory if all entries share a common prefix
        // (common for GitHub-style archives: `slug-version/...`).
        let dest = target.join(&raw_name);

        if file.is_dir() {
            std::fs::create_dir_all(&dest)?;
            continue;
        }

        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
            let canonical_parent = std::fs::canonicalize(parent)?;
            if !canonical_parent.starts_with(&canonical_target) {
                return Err(Error::Install("zip entry escaped install directory".into()));
            }
        }

        let mut buf = Vec::with_capacity(file.size() as usize);
        file.read_to_end(&mut buf)?;
        std::fs::write(&dest, &buf)?;
    }
    Ok(())
}

/// Build the manifest source key for a ClawHub skill.
pub fn clawhub_source_key(slug: &str) -> String {
    format!("clawhub:{slug}")
}

/// Check if a manifest source key is a ClawHub skill.
pub fn is_clawhub_source(source: &str) -> bool {
    source.starts_with("clawhub:")
}

fn validate_slug(slug: &str) -> Result<()> {
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

// ── Tests ───────────────────────────────────────────────────────────────────

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

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
    fn search_response_deserialises() {
        let json = r#"{"results":[{"score":0.95,"slug":"arxiv","displayName":"Arxiv","summary":"Search papers","updatedAt":"2026-01-01"}]}"#;
        let resp: SearchResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.results.len(), 1);
        assert_eq!(resp.results[0].slug, "arxiv");
        assert_eq!(resp.results[0].display_name.as_deref(), Some("Arxiv"));
    }

    #[test]
    fn skill_info_response_deserialises() {
        let json = r#"{"skill":{"slug":"arxiv","displayName":"Arxiv","summary":"Search","tags":["research"],"stats":{"downloads":100,"installs":50}},"latestVersion":{"version":"1.0.0"},"owner":{"handle":"alice"}}"#;
        let resp: SkillInfoResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.skill.slug, "arxiv");
        assert_eq!(resp.latest_version.unwrap().version, "1.0.0");
        assert_eq!(resp.owner.unwrap().handle.as_deref(), Some("alice"));
    }
}
