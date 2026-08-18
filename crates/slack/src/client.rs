use slack_morphism::prelude::*;

use moltis_channels::{
    Error as ChannelError, Result as ChannelResult, normalize_slack_api_base_url,
    validate_slack_api_base_url,
};

pub const DEFAULT_SLACK_API_BASE_URL: &str = "https://slack.com/api";

/// Build a Slack client after full base-URL validation, including DNS
/// resolution of hostname targets against the SSRF policy.
///
/// Use this at account registration. Hot paths whose config already passed
/// registration can use the cheaper [`slack_client_for_base_url`].
pub async fn validated_slack_client_for_base_url(
    api_base_url: &str,
) -> ChannelResult<SlackClient<SlackClientHyperHttpsConnector>> {
    validate_slack_api_base_url(api_base_url).await?;
    slack_client_for_base_url(api_base_url)
}

pub fn slack_client_for_base_url(
    api_base_url: &str,
) -> ChannelResult<SlackClient<SlackClientHyperHttpsConnector>> {
    let api_base_url = normalize_slack_api_base_url(api_base_url)?;
    let connector = SlackClientHyperConnector::new()
        .map_err(|e| ChannelError::unavailable(format!("hyper connector: {e}")))?
        .with_slack_api_url(&api_base_url);
    Ok(SlackClient::new(connector))
}

pub fn slack_api_method_url(api_base_url: &str, method: &str) -> ChannelResult<String> {
    Ok(format!(
        "{}/{}",
        normalize_slack_api_base_url(api_base_url)?,
        method.trim_start_matches('/')
    ))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_trailing_slashes() {
        assert_eq!(
            normalize_slack_api_base_url("https://proxy.example/api/").unwrap(),
            "https://proxy.example/api"
        );
    }

    #[test]
    fn rejects_relative_base_urls() {
        assert!(normalize_slack_api_base_url("/api").is_err());
    }

    #[test]
    fn rejects_localhost_base_urls() {
        assert!(normalize_slack_api_base_url("http://localhost:3000/api").is_err());
    }

    #[test]
    fn rejects_private_ip_base_urls() {
        assert!(normalize_slack_api_base_url("http://169.254.169.254/api").is_err());
        assert!(normalize_slack_api_base_url("http://10.0.0.1/api").is_err());
        assert!(normalize_slack_api_base_url("http://[fd00::1]/api").is_err());
        assert!(normalize_slack_api_base_url("http://[::ffff:169.254.169.254]/api").is_err());
    }

    #[test]
    fn builds_method_url_from_default_base() {
        assert_eq!(
            slack_api_method_url(DEFAULT_SLACK_API_BASE_URL, "chat.startStream").unwrap(),
            "https://slack.com/api/chat.startStream"
        );
    }

    #[test]
    fn builds_method_url_from_trailing_slash_base() {
        assert_eq!(
            slack_api_method_url("https://proxy.example/api/", "chat.startStream").unwrap(),
            "https://proxy.example/api/chat.startStream"
        );
    }
}
