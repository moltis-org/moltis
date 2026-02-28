//! Provider-specific CalDAV service discovery.
//!
//! Maps well-known providers to their CalDAV endpoints.

/// Well-known CalDAV base URL for Fastmail.
pub const FASTMAIL_CALDAV_URL: &str = "https://caldav.fastmail.com";

/// Well-known CalDAV base URL for iCloud.
/// Requires an app-specific password.
pub const ICLOUD_CALDAV_URL: &str = "https://caldav.icloud.com";

/// Resolve the CalDAV base URL for a given provider.
///
/// If `provider` is `None` or `"generic"`, the caller-supplied `url` is used.
/// For known providers, the well-known URL is returned even if `url` is `None`.
#[must_use]
pub fn resolve_base_url(provider: Option<&str>, url: Option<&str>) -> Option<String> {
    match provider {
        Some("fastmail") => Some(
            url.map(String::from)
                .unwrap_or_else(|| FASTMAIL_CALDAV_URL.to_string()),
        ),
        Some("icloud") => Some(
            url.map(String::from)
                .unwrap_or_else(|| ICLOUD_CALDAV_URL.to_string()),
        ),
        Some("generic") | None => url.map(String::from),
        Some(other) => {
            #[cfg(feature = "tracing")]
            tracing::warn!(provider = other, "unknown CalDAV provider, using URL as-is");
            url.map(String::from)
        },
    }
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fastmail_defaults_to_well_known() {
        let url = resolve_base_url(Some("fastmail"), None);
        assert_eq!(url.as_deref(), Some(FASTMAIL_CALDAV_URL));
    }

    #[test]
    fn fastmail_custom_url_overrides() {
        let url = resolve_base_url(Some("fastmail"), Some("https://custom.fastmail.com/dav"));
        assert_eq!(url.as_deref(), Some("https://custom.fastmail.com/dav"));
    }

    #[test]
    fn icloud_defaults_to_well_known() {
        let url = resolve_base_url(Some("icloud"), None);
        assert_eq!(url.as_deref(), Some(ICLOUD_CALDAV_URL));
    }

    #[test]
    fn generic_requires_explicit_url() {
        assert!(resolve_base_url(Some("generic"), None).is_none());
        assert_eq!(
            resolve_base_url(Some("generic"), Some("https://my.dav.server/caldav")).as_deref(),
            Some("https://my.dav.server/caldav")
        );
    }

    #[test]
    fn none_provider_requires_explicit_url() {
        assert!(resolve_base_url(None, None).is_none());
        assert_eq!(
            resolve_base_url(None, Some("https://example.com/caldav")).as_deref(),
            Some("https://example.com/caldav")
        );
    }
}
