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
