const COMPLETION_ENDPOINT_SUFFIXES: &[&str] = &["/chat/completions", "/responses"];

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub(crate) enum ProviderBaseUrlError {
    #[error("Endpoint URL must be a valid HTTP(S) URL, such as 'https://api.example.com/v1'.")]
    InvalidUrl,
    #[error("Endpoint URL must include an http:// or https:// scheme and a host.")]
    MissingHttpHost,
    #[error(
        "Endpoint URL should be the API base URL, not the completion path. Use '{suggested_base_url}' instead of '{base_url}'."
    )]
    CompletionEndpoint {
        base_url: String,
        suggested_base_url: String,
    },
}

#[must_use]
pub(crate) fn provider_base_url_error(base_url: &str) -> Option<ProviderBaseUrlError> {
    let trimmed = base_url.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return None;
    }

    let parsed = match url::Url::parse(trimmed) {
        Ok(parsed) => parsed,
        Err(_) => return Some(ProviderBaseUrlError::InvalidUrl),
    };
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return Some(ProviderBaseUrlError::MissingHttpHost);
    }

    let lower = trimmed.to_ascii_lowercase();
    let suffix = COMPLETION_ENDPOINT_SUFFIXES
        .iter()
        .find(|suffix| lower.ends_with(**suffix))?;
    let base = trimmed
        .get(..trimmed.len().saturating_sub(suffix.len()))
        .filter(|base| !base.is_empty())
        .unwrap_or(trimmed);

    Some(ProviderBaseUrlError::CompletionEndpoint {
        base_url: trimmed.to_string(),
        suggested_base_url: base.to_string(),
    })
}

pub(crate) fn validate_provider_base_url(
    base_url: Option<&str>,
) -> Result<(), ProviderBaseUrlError> {
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
        let error = provider_base_url_error("api.example.com/v1");

        assert_eq!(error, Some(ProviderBaseUrlError::InvalidUrl));
    }

    #[test]
    fn rejects_url_without_http_scheme() {
        let error = provider_base_url_error("ftp://api.example.com/v1");

        assert_eq!(error, Some(ProviderBaseUrlError::MissingHttpHost));
    }

    #[test]
    fn rejects_chat_completions_url() {
        let error =
            provider_base_url_error("https://api.deepinfra.com/v1/openai/chat/completions/");

        assert_eq!(
            error,
            Some(ProviderBaseUrlError::CompletionEndpoint {
                base_url: "https://api.deepinfra.com/v1/openai/chat/completions".into(),
                suggested_base_url: "https://api.deepinfra.com/v1/openai".into(),
            })
        );
    }

    #[test]
    fn rejects_mixed_case_chat_completions_url() {
        let error =
            provider_base_url_error("https://api.deepinfra.com/v1/openai/Chat/Completions/");

        assert_eq!(
            error,
            Some(ProviderBaseUrlError::CompletionEndpoint {
                base_url: "https://api.deepinfra.com/v1/openai/Chat/Completions".into(),
                suggested_base_url: "https://api.deepinfra.com/v1/openai".into(),
            })
        );
    }

    #[test]
    fn rejects_responses_url() {
        let error = provider_base_url_error("https://api.example.com/v1/responses");

        assert_eq!(
            error,
            Some(ProviderBaseUrlError::CompletionEndpoint {
                base_url: "https://api.example.com/v1/responses".into(),
                suggested_base_url: "https://api.example.com/v1".into(),
            })
        );
    }
}
