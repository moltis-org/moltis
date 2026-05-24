const COMPLETION_ENDPOINT_SUFFIXES: &[&str] = &["/chat/completions", "/responses"];

#[must_use]
pub(crate) fn openai_compatible_base_url_error(base_url: &str) -> Option<String> {
    let trimmed = base_url.trim().trim_end_matches('/');
    let lower = trimmed.to_ascii_lowercase();
    let suffix = COMPLETION_ENDPOINT_SUFFIXES
        .iter()
        .find(|suffix| lower.ends_with(**suffix))?;
    let base = trimmed
        .get(..trimmed.len().saturating_sub(suffix.len()))
        .filter(|base| !base.is_empty())
        .unwrap_or(trimmed);

    Some(format!(
        "Endpoint should be the API base URL, not the completion path. Use '{base}' instead of '{trimmed}'."
    ))
}

pub(crate) fn validate_openai_compatible_base_url(base_url: Option<&str>) -> Result<(), String> {
    let Some(base_url) = base_url else {
        return Ok(());
    };
    if let Some(error) = openai_compatible_base_url_error(base_url) {
        return Err(error);
    }
    Ok(())
}

#[must_use]
pub(crate) fn is_openai_compatible_provider_name(provider_name: &str) -> bool {
    matches!(
        provider_name,
        "openai"
            | "mistral"
            | "openrouter"
            | "cerebras"
            | "minimax"
            | "moonshot"
            | "zai"
            | "zai-code"
            | "venice"
            | "deepinfra"
            | "deepseek"
            | "fireworks"
            | "ollama"
            | "lmstudio"
            | "alibaba-coding"
            | "gemini"
            | "groq"
            | "xai"
            | "kimi-code"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_api_base_url() {
        assert!(openai_compatible_base_url_error("https://api.example.com/v1").is_none());
    }

    #[test]
    fn rejects_chat_completions_url() {
        let error = openai_compatible_base_url_error(
            "https://api.deepinfra.com/v1/openai/chat/completions/",
        )
        .unwrap_or_default();

        assert!(error.contains("https://api.deepinfra.com/v1/openai"));
        assert!(error.contains("chat/completions"));
    }

    #[test]
    fn rejects_mixed_case_chat_completions_url() {
        let error = openai_compatible_base_url_error(
            "https://api.deepinfra.com/v1/openai/Chat/Completions/",
        )
        .unwrap_or_default();

        assert!(error.contains("https://api.deepinfra.com/v1/openai"));
        assert!(error.contains("Chat/Completions"));
    }

    #[test]
    fn rejects_responses_url() {
        let error = openai_compatible_base_url_error("https://api.example.com/v1/responses")
            .unwrap_or_default();

        assert!(error.contains("https://api.example.com/v1"));
        assert!(error.contains("/responses"));
    }
}
