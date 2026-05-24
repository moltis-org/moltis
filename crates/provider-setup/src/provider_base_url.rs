const COMPLETION_ENDPOINT_SUFFIXES: &[&str] = &["/chat/completions", "/responses"];

#[must_use]
pub(crate) fn provider_base_url_error(base_url: &str) -> Option<String> {
    let trimmed = base_url.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return None;
    }

    let parsed = match url::Url::parse(trimmed) {
        Ok(parsed) => parsed,
        Err(_) => {
            return Some(
                "Endpoint URL must be a valid HTTP(S) URL, such as 'https://api.example.com/v1'."
                    .to_string(),
            );
        },
    };
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return Some(
            "Endpoint URL must include an http:// or https:// scheme and a host.".to_string(),
        );
    }

    let lower = trimmed.to_ascii_lowercase();
    let suffix = COMPLETION_ENDPOINT_SUFFIXES
        .iter()
        .find(|suffix| lower.ends_with(**suffix))?;
    let base = trimmed
        .get(..trimmed.len().saturating_sub(suffix.len()))
        .filter(|base| !base.is_empty())
        .unwrap_or(trimmed);

    Some(format!(
        "Endpoint URL should be the API base URL, not the completion path. Use '{base}' instead of '{trimmed}'."
    ))
}

pub(crate) fn validate_provider_base_url(base_url: Option<&str>) -> Result<(), String> {
    let Some(base_url) = base_url else {
        return Ok(());
    };
    if let Some(error) = provider_base_url_error(base_url) {
        return Err(error);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_api_base_url() {
        assert!(provider_base_url_error("https://api.example.com/v1").is_none());
        assert!(provider_base_url_error("http://localhost:11434/v1").is_none());
        assert!(provider_base_url_error("").is_none());
    }

    #[test]
    fn rejects_invalid_url() {
        let error = provider_base_url_error("api.example.com/v1").unwrap_or_default();

        assert!(error.contains("valid HTTP(S) URL"));
    }

    #[test]
    fn rejects_url_without_http_scheme() {
        let error = provider_base_url_error("ftp://api.example.com/v1").unwrap_or_default();

        assert!(error.contains("http:// or https://"));
    }

    #[test]
    fn rejects_chat_completions_url() {
        let error =
            provider_base_url_error("https://api.deepinfra.com/v1/openai/chat/completions/")
                .unwrap_or_default();

        assert!(error.contains("https://api.deepinfra.com/v1/openai"));
        assert!(error.contains("chat/completions"));
    }

    #[test]
    fn rejects_mixed_case_chat_completions_url() {
        let error =
            provider_base_url_error("https://api.deepinfra.com/v1/openai/Chat/Completions/")
                .unwrap_or_default();

        assert!(error.contains("https://api.deepinfra.com/v1/openai"));
        assert!(error.contains("Chat/Completions"));
    }

    #[test]
    fn rejects_responses_url() {
        let error =
            provider_base_url_error("https://api.example.com/v1/responses").unwrap_or_default();

        assert!(error.contains("https://api.example.com/v1"));
        assert!(error.contains("/responses"));
    }
}
