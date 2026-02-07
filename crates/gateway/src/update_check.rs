use std::time::Duration;

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct UpdateAvailability {
    pub available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub release_url: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum UpdateCheckError {
    #[error("repository URL is not a GitHub repository: {0}")]
    UnsupportedRepository(String),
    #[error("request failed: {0}")]
    Request(#[from] reqwest::Error),
}

#[derive(Debug, serde::Deserialize)]
struct GithubLatestRelease {
    tag_name: String,
    html_url: Option<String>,
}

pub const UPDATE_CHECK_INTERVAL: Duration = Duration::from_secs(60 * 60);

pub fn github_latest_release_api_url(repository_url: &str) -> Result<String, UpdateCheckError> {
    let slug = github_repo_slug(repository_url)
        .ok_or_else(|| UpdateCheckError::UnsupportedRepository(repository_url.to_owned()))?;
    Ok(format!(
        "https://api.github.com/repos/{slug}/releases/latest"
    ))
}

pub async fn fetch_update_availability(
    client: &reqwest::Client,
    latest_release_api_url: &str,
    current_version: &str,
) -> Result<UpdateAvailability, UpdateCheckError> {
    let release = client
        .get(latest_release_api_url)
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .send()
        .await?
        .error_for_status()?
        .json::<GithubLatestRelease>()
        .await?;

    Ok(update_from_release(
        &release.tag_name,
        release.html_url.as_deref(),
        current_version,
    ))
}

fn update_from_release(
    tag_name: &str,
    release_url: Option<&str>,
    current: &str,
) -> UpdateAvailability {
    let latest = normalize_version(tag_name);
    UpdateAvailability {
        available: is_newer_version(&latest, current),
        latest_version: Some(latest),
        release_url: release_url.map(str::to_owned),
    }
}

fn github_repo_slug(repository_url: &str) -> Option<String> {
    let trimmed = repository_url.trim();
    let without_scheme = trimmed
        .strip_prefix("https://")
        .or_else(|| trimmed.strip_prefix("http://"))?;

    let mut parts = without_scheme.split('/');
    let host = parts.next()?.trim();
    if !host.eq_ignore_ascii_case("github.com") {
        return None;
    }

    let owner = parts.next()?.trim();
    let repo_part = parts.next()?.trim();
    let repo = repo_part.strip_suffix(".git").unwrap_or(repo_part);

    if owner.is_empty() || repo.is_empty() {
        return None;
    }
    Some(format!("{owner}/{repo}"))
}

fn is_newer_version(latest: &str, current: &str) -> bool {
    let latest = parse_semver_triplet(latest);
    let current = parse_semver_triplet(current);
    matches!((latest, current), (Some(l), Some(c)) if l > c)
}

fn normalize_version(value: &str) -> String {
    value.trim().trim_start_matches(['v', 'V']).to_owned()
}

fn parse_semver_triplet(version: &str) -> Option<(u64, u64, u64)> {
    let normalized = normalize_version(version);
    let core = normalized
        .split_once(['-', '+'])
        .map(|(v, _)| v)
        .unwrap_or(&normalized);
    let mut parts = core.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_github_repo_slug() {
        assert_eq!(
            github_repo_slug("https://github.com/moltis-org/moltis"),
            Some("moltis-org/moltis".to_owned())
        );
        assert_eq!(
            github_repo_slug("https://github.com/moltis-org/moltis/"),
            Some("moltis-org/moltis".to_owned())
        );
        assert_eq!(
            github_repo_slug("https://github.com/moltis-org/moltis.git"),
            Some("moltis-org/moltis".to_owned())
        );
        assert_eq!(
            github_repo_slug("https://example.com/moltis-org/moltis"),
            None
        );
    }

    #[test]
    fn compares_semver_versions() {
        assert!(is_newer_version("0.3.0", "0.2.9"));
        assert!(is_newer_version("v1.0.0", "0.9.9"));
        assert!(!is_newer_version("0.2.5", "0.2.5"));
        assert!(!is_newer_version("0.2.4", "0.2.5"));
        assert!(!is_newer_version("latest", "0.2.5"));
    }

    #[test]
    fn strips_pre_release_metadata_before_compare() {
        assert!(is_newer_version("v0.3.0-rc.1", "0.2.9"));
        assert!(!is_newer_version("v0.2.5+build.42", "0.2.5"));
    }

    #[test]
    fn builds_update_payload_from_release() {
        let update = update_from_release(
            "v0.3.0",
            Some("https://github.com/moltis-org/moltis/releases/tag/v0.3.0"),
            "0.2.5",
        );

        assert!(update.available);
        assert_eq!(update.latest_version.as_deref(), Some("0.3.0"));
        assert_eq!(
            update.release_url.as_deref(),
            Some("https://github.com/moltis-org/moltis/releases/tag/v0.3.0")
        );
    }
}
