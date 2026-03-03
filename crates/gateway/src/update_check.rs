use std::time::Duration;

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct UpdateAvailability {
    pub available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub release_url: Option<String>,
}

/// A channel entry in the releases manifest.
#[derive(Debug, Clone, serde::Deserialize)]
struct ReleaseChannel {
    version: String,
    release_url: Option<String>,
}

/// The `releases.json` manifest served at the configured URL.
#[derive(Debug, serde::Deserialize)]
struct ReleasesManifest {
    stable: Option<ReleaseChannel>,
    unstable: Option<ReleaseChannel>,
}

pub const UPDATE_CHECK_INTERVAL: Duration = Duration::from_secs(60 * 60);

const DEFAULT_RELEASES_URL: &str = "https://www.moltis.org/releases.json";

/// Resolve the releases manifest URL from config, falling back to the default.
#[must_use]
pub fn resolve_releases_url(configured: Option<&str>) -> String {
    configured
        .map(str::trim)
        .filter(|url| !url.is_empty())
        .unwrap_or(DEFAULT_RELEASES_URL)
        .to_owned()
}

/// Fetch update availability from the releases manifest.
///
/// Returns a default (no update) on any error — 404, parse failure, network
/// issues — so callers never have to handle errors.
pub async fn fetch_update_availability(
    client: &reqwest::Client,
    releases_url: &str,
    current_version: &str,
) -> UpdateAvailability {
    match try_fetch_update(client, releases_url, current_version).await {
        Ok(update) => update,
        Err(e) => {
            tracing::debug!("update check skipped: {e}");
            UpdateAvailability::default()
        },
    }
}

async fn try_fetch_update(
    client: &reqwest::Client,
    releases_url: &str,
    current_version: &str,
) -> Result<UpdateAvailability, Box<dyn std::error::Error + Send + Sync>> {
    let response = client.get(releases_url).send().await?;
    if !response.status().is_success() {
        return Err(format!("HTTP {}", response.status()).into());
    }
    let manifest: ReleasesManifest = response.json().await?;

    let channel = if is_pre_release(current_version) {
        manifest.unstable.or(manifest.stable)
    } else {
        manifest.stable
    };

    match channel {
        Some(release) => Ok(update_from_release(
            &release.version,
            release.release_url.as_deref(),
            current_version,
        )),
        None => Ok(UpdateAvailability::default()),
    }
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

fn is_pre_release(version: &str) -> bool {
    let normalized = normalize_version(version);
    normalized.contains('-')
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
    fn compares_semver_versions() {
        assert!(is_newer_version("0.3.0", "0.2.9"));
        assert!(is_newer_version("v1.0.0", "0.9.9"));
        assert!(!is_newer_version("0.2.5", "0.2.5"));
        assert!(!is_newer_version("0.2.4", "0.2.5"));
        assert!(!is_newer_version("latest", "0.2.5"));
    }

    #[test]
    fn resolves_releases_url_with_config_override() {
        assert_eq!(
            resolve_releases_url(Some(" https://example.com/releases.json ")),
            "https://example.com/releases.json"
        );
    }

    #[test]
    fn resolves_releases_url_default_when_missing_or_blank() {
        assert_eq!(resolve_releases_url(Some("   ")), DEFAULT_RELEASES_URL);
        assert_eq!(resolve_releases_url(None), DEFAULT_RELEASES_URL);
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

    #[test]
    fn detects_pre_release_versions() {
        assert!(is_pre_release("0.11.0-rc.1"));
        assert!(is_pre_release("v0.11.0-beta.2"));
        assert!(!is_pre_release("0.10.7"));
        assert!(!is_pre_release("v0.10.7"));
    }

    #[test]
    fn selects_channel_based_on_current_version() {
        let stable = ReleaseChannel {
            version: "0.10.7".into(),
            release_url: Some("https://github.com/moltis-org/moltis/releases/tag/v0.10.7".into()),
        };
        let unstable = ReleaseChannel {
            version: "0.11.0-rc.2".into(),
            release_url: Some(
                "https://github.com/moltis-org/moltis/releases/tag/v0.11.0-rc.2".into(),
            ),
        };

        // Stable current → picks stable channel
        let current_stable = "0.10.6";
        assert!(!is_pre_release(current_stable));
        let update = update_from_release(
            &stable.version,
            stable.release_url.as_deref(),
            current_stable,
        );
        assert!(update.available);
        assert_eq!(update.latest_version.as_deref(), Some("0.10.7"));

        // Pre-release current → would pick unstable channel
        let current_pre = "0.11.0-rc.1";
        assert!(is_pre_release(current_pre));
        let update = update_from_release(
            &unstable.version,
            unstable.release_url.as_deref(),
            current_pre,
        );
        // Both are 0.11.0 after stripping pre-release suffix, so no update
        assert!(!update.available);
    }
}
