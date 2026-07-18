use slack_morphism::prelude::*;

use moltis_channels::{Error as ChannelError, Result as ChannelResult};

pub const DEFAULT_SLACK_API_BASE_URL: &str = "https://slack.com/api";

pub fn normalize_slack_api_base_url(api_base_url: &str) -> ChannelResult<String> {
    let trimmed = api_base_url.trim().trim_end_matches('/');
    let parsed = reqwest::Url::parse(trimmed).map_err(|e| {
        ChannelError::invalid_input(format!("Slack api_base_url must be an absolute URL: {e}"))
    })?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return Err(ChannelError::invalid_input(
            "Slack api_base_url must be an absolute HTTP(S) URL",
        ));
    }
    Ok(trimmed.to_string())
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
