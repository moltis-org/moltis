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

/// Compare two version strings. Handles three cases:
/// 1. Both date-based (`YYYYMMDD.NN`) — compare as `(date, seq)` tuples.
/// 2. Both semver (`x.y.z`) — compare as `(major, minor, patch)` tuples.
/// 3. Mixed: any date-based version is considered newer than any semver version,
///    ensuring users on old semver builds see the update to the new scheme.
fn is_newer_version(latest: &str, current: &str) -> bool {
    let latest_n = normalize_version(latest);
    let current_n = normalize_version(current);

    match (
        parse_date_version(&latest_n),
        parse_date_version(&current_n),
    ) {
        // Both date-based
        (Some(l), Some(c)) => l > c,
        // Latest is date-based, current is semver → latest wins
        (Some(_), None) => true,
        // Latest is semver, current is date-based → no update
        (None, Some(_)) => false,
        // Neither is date-based — try semver
        (None, None) => {
            matches!(
                (parse_semver_triplet(&latest_n), parse_semver_triplet(&current_n)),
                (Some(l), Some(c)) if l > c
            )
        },
    }
}

fn normalize_version(value: &str) -> String {
    value.trim().trim_start_matches(['v', 'V']).to_owned()
}

/// Parse a date-based version `YYYYMMDD.NN` into `(date, sequence)`.
fn parse_date_version(version: &str) -> Option<(u32, u32)> {
    let (date_str, seq_str) = version.split_once('.')?;
    // Date part must be exactly 8 digits
    if date_str.len() != 8 || !date_str.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    // Sequence part must be 1-2 digits (no extra segments)
    if seq_str.contains('.') || !seq_str.chars().all(|c| c.is_ascii_digit()) || seq_str.is_empty() {
        return None;
    }
    let date: u32 = date_str.parse().ok()?;
    let seq: u32 = seq_str.parse().ok()?;
    Some((date, seq))
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

    // --- date version parsing ---

    #[test]
    fn parses_valid_date_versions() {
        assert_eq!(parse_date_version("20260311.01"), Some((20260311, 1)));
        assert_eq!(parse_date_version("20260311.1"), Some((20260311, 1)));
        assert_eq!(parse_date_version("20260101.99"), Some((20260101, 99)));
    }

    #[test]
    fn rejects_invalid_date_versions() {
        assert_eq!(parse_date_version("0.10.18"), None);
        assert_eq!(parse_date_version("v0.10.18"), None);
        assert_eq!(parse_date_version("latest"), None);
        assert_eq!(parse_date_version("2026031.01"), None); // 7 digits
        assert_eq!(parse_date_version("202603110.01"), None); // 9 digits
    }

    #[test]
    fn semver_not_confused_with_date() {
        assert_eq!(parse_date_version("1.2.3"), None);
        assert_eq!(parse_date_version("0.10.18"), None);
    }

    // --- semver parsing ---

    #[test]
    fn parses_valid_semver() {
        assert_eq!(parse_semver_triplet("0.10.18"), Some((0, 10, 18)));
        assert_eq!(parse_semver_triplet("v1.2.3"), Some((1, 2, 3)));
        assert_eq!(parse_semver_triplet("0.11.0-rc.1"), Some((0, 11, 0)));
    }

    #[test]
    fn rejects_date_version_as_semver() {
        // Date versions have only two segments, so semver parse fails
        assert_eq!(parse_semver_triplet("20260311.01"), None);
    }

    // --- version comparison ---

    #[test]
    fn compares_semver_versions() {
        assert!(is_newer_version("0.3.0", "0.2.9"));
        assert!(is_newer_version("v1.0.0", "0.9.9"));
        assert!(!is_newer_version("0.2.5", "0.2.5"));
        assert!(!is_newer_version("0.2.4", "0.2.5"));
        assert!(!is_newer_version("latest", "0.2.5"));
    }

    #[test]
    fn compares_date_versions() {
        assert!(is_newer_version("20260312.01", "20260311.01"));
        assert!(is_newer_version("20260311.02", "20260311.01"));
        assert!(!is_newer_version("20260311.01", "20260311.01"));
        assert!(!is_newer_version("20260310.01", "20260311.01"));
    }

    #[test]
    fn date_version_newer_than_any_semver() {
        // Users on old semver builds should see date-based updates
        assert!(is_newer_version("20260311.01", "0.10.18"));
        assert!(is_newer_version("20260311.01", "99.99.99"));
    }

    #[test]
    fn semver_not_newer_than_date_version() {
        assert!(!is_newer_version("0.10.18", "20260311.01"));
        assert!(!is_newer_version("99.99.99", "20260311.01"));
    }

    // --- existing tests updated ---

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
            "20260311.01",
            Some("https://github.com/moltis-org/moltis/releases/tag/20260311.01"),
            "0.10.18",
        );

        assert!(update.available);
        assert_eq!(update.latest_version.as_deref(), Some("20260311.01"));
        assert_eq!(
            update.release_url.as_deref(),
            Some("https://github.com/moltis-org/moltis/releases/tag/20260311.01")
        );
    }

    #[test]
    fn builds_update_payload_date_to_date() {
        let update = update_from_release(
            "20260312.01",
            Some("https://github.com/moltis-org/moltis/releases/tag/20260312.01"),
            "20260311.01",
        );

        assert!(update.available);
        assert_eq!(update.latest_version.as_deref(), Some("20260312.01"));
    }

    #[test]
    fn detects_pre_release_versions() {
        assert!(is_pre_release("0.11.0-rc.1"));
        assert!(is_pre_release("v0.11.0-beta.2"));
        assert!(!is_pre_release("0.10.7"));
        assert!(!is_pre_release("v0.10.7"));
        assert!(!is_pre_release("20260311.01"));
    }

    #[test]
    fn selects_channel_based_on_current_version() {
        let stable = ReleaseChannel {
            version: "20260311.01".into(),
            release_url: Some(
                "https://github.com/moltis-org/moltis/releases/tag/20260311.01".into(),
            ),
        };
        let unstable = ReleaseChannel {
            version: "0.11.0-rc.2".into(),
            release_url: Some(
                "https://github.com/moltis-org/moltis/releases/tag/v0.11.0-rc.2".into(),
            ),
        };

        // Stable current → picks stable channel, date version is newer
        let current_stable = "0.10.18";
        assert!(!is_pre_release(current_stable));
        let update = update_from_release(
            &stable.version,
            stable.release_url.as_deref(),
            current_stable,
        );
        assert!(update.available);
        assert_eq!(update.latest_version.as_deref(), Some("20260311.01"));

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
