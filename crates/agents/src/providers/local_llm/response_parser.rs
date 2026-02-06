//! Response parsing for local LLM backends.
//!
//! Different backends and models may output responses in different formats.
//! This module provides a trait-based approach to parse raw output into
//! structured responses, making it easy to add new parsers without modifying
//! existing code.

/// Parsed response from an LLM backend.
#[derive(Debug, Clone, Default)]
pub struct ParsedResponse {
    /// The actual response text, cleaned of any formatting artifacts.
    pub text: String,
    /// Number of input/prompt tokens (if available).
    pub input_tokens: Option<u32>,
    /// Number of output/generation tokens (if available).
    pub output_tokens: Option<u32>,
}

/// Trait for parsing raw LLM output into structured responses.
///
/// Implement this trait for different output formats (JSON, CLI decorators, etc.).
pub trait ResponseParser: Send + Sync {
    /// Parse raw output into a structured response.
    fn parse(&self, raw: &str) -> ParsedResponse;

    /// Human-readable name for this parser (for logging/debugging).
    fn name(&self) -> &'static str;
}

// ── JSON Response Parser ─────────────────────────────────────────────────────

/// Parser for JSON-formatted responses from Python API.
///
/// Expects format: `{"text": "...", "input_tokens": N, "output_tokens": M}`
#[derive(Debug, Default)]
pub struct JsonResponseParser;

impl ResponseParser for JsonResponseParser {
    fn name(&self) -> &'static str {
        "json"
    }

    fn parse(&self, raw: &str) -> ParsedResponse {
        // Try to parse as JSON
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(raw) {
            return ParsedResponse {
                text: value["text"].as_str().unwrap_or("").to_string(),
                input_tokens: value["input_tokens"].as_u64().map(|n| n as u32),
                output_tokens: value["output_tokens"].as_u64().map(|n| n as u32),
            };
        }

        // Fallback: return raw as text
        ParsedResponse {
            text: raw.trim().to_string(),
            input_tokens: None,
            output_tokens: None,
        }
    }
}

// ── MLX CLI Response Parser ──────────────────────────────────────────────────

/// Parser for mlx_lm CLI output with `==========` delimiters.
///
/// The CLI outputs in this format:
/// ```text
/// ==========
/// Response text here
/// ==========
/// Prompt: 346 tokens, 502.788 tokens-per-sec
/// Generation: 35 tokens, 448.124 tokens-per-sec
/// Peak memory: 1.042 GB
/// ```
#[derive(Debug, Default)]
pub struct MlxCliResponseParser;

impl ResponseParser for MlxCliResponseParser {
    fn name(&self) -> &'static str {
        "mlx-cli"
    }

    fn parse(&self, raw: &str) -> ParsedResponse {
        let mut text = String::new();
        let mut input_tokens = None;
        let mut output_tokens = None;

        // Split by the separator line
        let parts: Vec<&str> = raw.split("==========").collect();

        // The response text is between the first and second separator
        if parts.len() >= 2 {
            text = parts[1].trim().to_string();
        }

        // Parse token counts from the stats lines after the second separator
        if parts.len() >= 3 {
            for line in parts[2].lines() {
                let line = line.trim();
                // "Prompt: 346 tokens, 502.788 tokens-per-sec"
                if let Some(tokens_part) = line.strip_prefix("Prompt:")
                    && let Some(tokens_str) = tokens_part.split_whitespace().next()
                {
                    input_tokens = tokens_str.parse().ok();
                }
                // "Generation: 35 tokens, 448.124 tokens-per-sec"
                else if let Some(tokens_part) = line.strip_prefix("Generation:")
                    && let Some(tokens_str) = tokens_part.split_whitespace().next()
                {
                    output_tokens = tokens_str.parse().ok();
                }
            }
        }

        ParsedResponse {
            text,
            input_tokens,
            output_tokens,
        }
    }
}

// ── Passthrough Parser ───────────────────────────────────────────────────────

/// Parser that returns raw output unchanged.
///
/// Useful as a fallback or for outputs that don't need cleaning.
#[derive(Debug, Default)]
pub struct PassthroughParser;

impl ResponseParser for PassthroughParser {
    fn name(&self) -> &'static str {
        "passthrough"
    }

    fn parse(&self, raw: &str) -> ParsedResponse {
        ParsedResponse {
            text: raw.to_string(),
            input_tokens: None,
            output_tokens: None,
        }
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── JSON Parser Tests ────────────────────────────────────────────────────

    #[test]
    fn test_json_parser_valid() {
        let parser = JsonResponseParser;
        let raw = r#"{"text": "Hello world!", "input_tokens": 10, "output_tokens": 3}"#;
        let result = parser.parse(raw);

        assert_eq!(result.text, "Hello world!");
        assert_eq!(result.input_tokens, Some(10));
        assert_eq!(result.output_tokens, Some(3));
    }

    #[test]
    fn test_json_parser_missing_tokens() {
        let parser = JsonResponseParser;
        let raw = r#"{"text": "Hello"}"#;
        let result = parser.parse(raw);

        assert_eq!(result.text, "Hello");
        assert_eq!(result.input_tokens, None);
        assert_eq!(result.output_tokens, None);
    }

    #[test]
    fn test_json_parser_invalid_json_fallback() {
        let parser = JsonResponseParser;
        let raw = "Not valid JSON";
        let result = parser.parse(raw);

        assert_eq!(result.text, "Not valid JSON");
        assert_eq!(result.input_tokens, None);
        assert_eq!(result.output_tokens, None);
    }

    // ── MLX CLI Parser Tests ─────────────────────────────────────────────────

    #[test]
    fn test_mlx_cli_parser_full_output() {
        let parser = MlxCliResponseParser;
        let raw = r#"==========
I'd be happy to help you with a joke.

Here's one:

What do you call a fake noodle?

An impasta!

I hope you find it amusing!
==========
Prompt: 449 tokens, 1635.874 tokens-per-sec
Generation: 36 tokens, 453.661 tokens-per-sec
Peak memory: 1.138 GB
"#;
        let result = parser.parse(raw);

        assert!(result.text.starts_with("I'd be happy"));
        assert!(result.text.contains("An impasta!"));
        assert!(result.text.ends_with("amusing!"));
        assert_eq!(result.input_tokens, Some(449));
        assert_eq!(result.output_tokens, Some(36));
    }

    #[test]
    fn test_mlx_cli_parser_simple() {
        let parser = MlxCliResponseParser;
        let raw = "==========\nHello world!\n==========\nPrompt: 10 tokens, 100.0 tokens-per-sec\nGeneration: 2 tokens, 50.0 tokens-per-sec\n";
        let result = parser.parse(raw);

        assert_eq!(result.text, "Hello world!");
        assert_eq!(result.input_tokens, Some(10));
        assert_eq!(result.output_tokens, Some(2));
    }

    #[test]
    fn test_mlx_cli_parser_no_stats() {
        let parser = MlxCliResponseParser;
        let raw = "==========\nHello!\n==========\n";
        let result = parser.parse(raw);

        assert_eq!(result.text, "Hello!");
        assert_eq!(result.input_tokens, None);
        assert_eq!(result.output_tokens, None);
    }

    #[test]
    fn test_mlx_cli_parser_unexpected_format() {
        let parser = MlxCliResponseParser;
        let raw = "Some unexpected output format";
        let result = parser.parse(raw);

        // Should return empty text, no tokens
        assert!(result.text.is_empty());
        assert_eq!(result.input_tokens, None);
        assert_eq!(result.output_tokens, None);
    }

    // ── Passthrough Parser Tests ─────────────────────────────────────────────

    #[test]
    fn test_passthrough_parser() {
        let parser = PassthroughParser;
        let raw = "Whatever\nraw\noutput";
        let result = parser.parse(raw);

        assert_eq!(result.text, raw);
        assert_eq!(result.input_tokens, None);
        assert_eq!(result.output_tokens, None);
    }

    // ── Parser Name Tests ────────────────────────────────────────────────────

    #[test]
    fn test_parser_names() {
        assert_eq!(JsonResponseParser.name(), "json");
        assert_eq!(MlxCliResponseParser.name(), "mlx-cli");
        assert_eq!(PassthroughParser.name(), "passthrough");
    }
}
