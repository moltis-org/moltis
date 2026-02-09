//! Text-to-Speech provider abstraction and implementations.

mod coqui;
mod elevenlabs;
mod google;
mod openai;
mod piper;

pub use {
    coqui::CoquiTts, elevenlabs::ElevenLabsTts, google::GoogleTts, openai::OpenAiTts,
    piper::PiperTts,
};

use {
    anyhow::Result,
    async_trait::async_trait,
    bytes::Bytes,
    serde::{Deserialize, Serialize},
    std::borrow::Cow,
};

/// A voice available from a TTS provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Voice {
    /// Provider-specific voice identifier.
    pub id: String,
    /// Human-readable name.
    pub name: String,
    /// Optional description or tags.
    pub description: Option<String>,
    /// Preview URL if available.
    pub preview_url: Option<String>,
}

/// Audio output format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AudioFormat {
    /// MP3 format (widely compatible).
    #[default]
    Mp3,
    /// Opus in OGG container (good for Telegram voice notes).
    Opus,
    /// AAC format.
    Aac,
    /// PCM (raw audio).
    Pcm,
    /// WebM container (browser MediaRecorder default).
    Webm,
}

impl AudioFormat {
    /// MIME type for this format.
    #[must_use]
    pub fn mime_type(&self) -> &'static str {
        match self {
            Self::Mp3 => "audio/mpeg",
            Self::Opus => "audio/ogg",
            Self::Aac => "audio/aac",
            Self::Pcm => "audio/pcm",
            Self::Webm => "audio/webm",
        }
    }

    /// File extension for this format.
    #[must_use]
    pub fn extension(&self) -> &'static str {
        match self {
            Self::Mp3 => "mp3",
            Self::Opus => "ogg",
            Self::Aac => "aac",
            Self::Pcm => "pcm",
            Self::Webm => "webm",
        }
    }

    /// Parse from a MIME content-type string (e.g. `"audio/webm"`, `"audio/webm;codecs=opus"`).
    #[must_use]
    pub fn from_content_type(ct: &str) -> Option<Self> {
        let base = ct.split(';').next().unwrap_or(ct).trim();
        match base {
            "audio/webm" => Some(Self::Webm),
            "audio/ogg" | "audio/opus" => Some(Self::Opus),
            "audio/mpeg" | "audio/mp3" => Some(Self::Mp3),
            "audio/aac" | "audio/mp4" | "audio/m4a" => Some(Self::Aac),
            "audio/pcm" | "audio/wav" | "audio/x-wav" => Some(Self::Pcm),
            _ => None,
        }
    }

    /// Parse from a short format name (e.g. `"webm"`, `"opus"`, `"ogg"`, `"mp3"`).
    #[must_use]
    pub fn from_short_name(name: &str) -> Self {
        match name {
            "webm" => Self::Webm,
            "opus" | "ogg" => Self::Opus,
            "aac" | "m4a" => Self::Aac,
            "pcm" | "wav" => Self::Pcm,
            _ => Self::Mp3,
        }
    }
}

/// Request to synthesize speech from text.
#[derive(Debug, Clone, Default)]
pub struct SynthesizeRequest {
    /// Text to convert to speech.
    pub text: String,
    /// Voice ID (provider-specific).
    pub voice_id: Option<String>,
    /// Model to use (provider-specific).
    pub model: Option<String>,
    /// Output audio format.
    pub output_format: AudioFormat,
    /// Speed multiplier (0.5 - 2.0).
    pub speed: Option<f32>,
    /// Stability setting (ElevenLabs-specific, 0.0 - 1.0).
    pub stability: Option<f32>,
    /// Similarity boost (ElevenLabs-specific, 0.0 - 1.0).
    pub similarity_boost: Option<f32>,
}

/// Audio output from TTS synthesis.
#[derive(Debug, Clone)]
pub struct AudioOutput {
    /// Raw audio data.
    pub data: Bytes,
    /// Audio format.
    pub format: AudioFormat,
    /// Duration in milliseconds (if known).
    pub duration_ms: Option<u64>,
}

/// Text-to-Speech provider trait.
///
/// Implementations provide speech synthesis from text using various services.
#[async_trait]
pub trait TtsProvider: Send + Sync {
    /// Provider identifier (e.g., "elevenlabs", "openai").
    fn id(&self) -> &'static str;

    /// Human-readable provider name.
    fn name(&self) -> &'static str;

    /// Check if the provider is configured and ready.
    fn is_configured(&self) -> bool;

    /// Whether this provider supports SSML tags natively.
    ///
    /// Providers that return `true` will receive SSML-tagged text as-is.
    /// Providers that return `false` will have SSML tags stripped before
    /// synthesis (handled centrally in the gateway's `convert()` handler).
    fn supports_ssml(&self) -> bool {
        false
    }

    /// List available voices from this provider.
    async fn voices(&self) -> Result<Vec<Voice>>;

    /// Convert text to speech.
    async fn synthesize(&self, request: SynthesizeRequest) -> Result<AudioOutput>;
}

/// Check whether text contains SSML tags (currently `<break`).
#[must_use]
pub fn contains_ssml(text: &str) -> bool {
    text.contains("<break")
}

/// Strip SSML tags (`<break .../>`) so non-SSML providers don't speak them
/// literally. Returns the input unchanged (no allocation) when no tags are
/// present.
#[must_use]
pub fn strip_ssml_tags(text: &str) -> Cow<'_, str> {
    if !contains_ssml(text) {
        return Cow::Borrowed(text);
    }

    let mut result = String::with_capacity(text.len());
    let mut rest = text;

    while let Some(start) = rest.find("<break") {
        result.push_str(&rest[..start]);
        // Find the closing `/>` or `>`
        if let Some(end) = rest[start..].find("/>") {
            rest = &rest[start + end + 2..];
        } else if let Some(end) = rest[start..].find('>') {
            rest = &rest[start + end + 1..];
        } else {
            // Malformed tag — keep the rest as-is
            result.push_str(&rest[start..]);
            rest = "";
            break;
        }
    }
    result.push_str(rest);

    Cow::Owned(result)
}

/// Sanitize LLM output for text-to-speech consumption.
///
/// Strips markdown formatting, raw URLs, code fences, and other visual syntax
/// that TTS engines would read literally. Returns `Cow::Borrowed` when the
/// input needs no changes (zero-alloc fast path).
///
/// Processing order:
/// 1. Remove fenced code blocks (``` ... ```)
/// 2. Remove URLs (`https?://…`)
/// 3. Strip markdown headers (`# `, `## `, etc.)
/// 4. Strip bold/italic markers (`**`, `__`, word-boundary `*`/`_`)
/// 5. Strip inline code backticks
/// 6. Strip bullet/list prefixes (`- `, `* `, `1. `)
/// 7. Collapse runs of whitespace / blank lines
/// 8. Strip SSML tags (via [`strip_ssml_tags`])
#[must_use]
pub fn sanitize_text_for_tts(text: &str) -> Cow<'_, str> {
    // Fast path: if none of the markers we strip are present, return as-is.
    if !needs_sanitization(text) {
        return Cow::Borrowed(text);
    }

    let mut out = String::with_capacity(text.len());

    // --- Pass 1: remove fenced code blocks ---
    let after_fences = remove_code_fences(text);

    // --- Process line by line ---
    for line in after_fences.lines() {
        let mut l: &str = line;

        // Strip markdown headers (e.g. "## Title" → "Title")
        l = strip_leading_hashes(l);

        // Strip bullet/list prefixes ("- ", "* " at line start, "1. " etc.)
        l = strip_bullet_prefix(l);

        // Strip inline code backticks
        let l_owned;
        if l.contains('`') {
            l_owned = l.replace('`', "");
            l = &l_owned;
        } else {
            l_owned = String::new();
            let _ = &l_owned; // suppress unused warning
        }

        // Remove URLs (http:// or https://)
        let after_urls = remove_urls(l);

        // Strip bold/italic markers
        let after_bold = strip_bold_italic(&after_urls);

        // Push the cleaned line
        let trimmed = after_bold.trim();
        if !trimmed.is_empty() {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(trimmed);
        }
    }

    // Collapse any remaining multiple blank lines into one
    let collapsed = collapse_whitespace(&out);

    // Final pass: strip SSML tags
    match strip_ssml_tags(&collapsed) {
        Cow::Borrowed(_) => Cow::Owned(collapsed),
        Cow::Owned(s) => Cow::Owned(s),
    }
}

/// Quick check for whether the text contains any markers we strip.
fn needs_sanitization(text: &str) -> bool {
    text.contains("```")
        || text.contains("http://")
        || text.contains("https://")
        || text.contains('#')
        || text.contains('*')
        || text.contains('_')
        || text.contains('`')
        || text.contains("\n- ")
        || text.starts_with("- ")
        || has_ordered_list(text)
        || text.contains("<break")
}

/// Check if the text contains an ordered list pattern (`\n1. `, `\n2. `, etc.).
fn has_ordered_list(text: &str) -> bool {
    for (i, ch) in text.char_indices() {
        if ch == '\n' {
            // Check if next chars are digits followed by ". "
            let rest = &text[i + 1..];
            let digit_len = rest.chars().take_while(|c| c.is_ascii_digit()).count();
            if digit_len > 0 && rest[digit_len..].starts_with(". ") {
                return true;
            }
        }
    }
    // Also check start of text
    let digit_len = text.chars().take_while(|c| c.is_ascii_digit()).count();
    digit_len > 0 && text[digit_len..].starts_with(". ")
}

/// Remove fenced code blocks (` ``` ... ``` `).
fn remove_code_fences(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut in_fence = false;

    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        result.push_str(line);
        result.push('\n');
    }

    // Remove trailing newline added by the loop
    if result.ends_with('\n') && !text.ends_with('\n') {
        result.pop();
    }

    result
}

/// Strip leading `#` characters (markdown headers) from a line.
fn strip_leading_hashes(line: &str) -> &str {
    let trimmed = line.trim_start();
    if !trimmed.starts_with('#') {
        return line;
    }
    let after_hashes = trimmed.trim_start_matches('#');
    // Only strip if followed by a space (avoids false positives like "#hashtag")
    after_hashes.strip_prefix(' ').unwrap_or(line)
}

/// Strip bullet/list prefixes from a line start.
fn strip_bullet_prefix(line: &str) -> &str {
    let trimmed = line.trim_start();
    // Unordered: "- " or "* "
    if let Some(rest) = trimmed.strip_prefix("- ") {
        return rest;
    }
    if let Some(rest) = trimmed.strip_prefix("* ") {
        return rest;
    }
    // Ordered: "1. ", "2. ", etc.
    if let Some(pos) = trimmed.find(". ") {
        let prefix = &trimmed[..pos];
        if !prefix.is_empty() && prefix.chars().all(|c| c.is_ascii_digit()) {
            return &trimmed[pos + 2..];
        }
    }
    line
}

/// Remove URLs matching `https?://\S+`.
fn remove_urls(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut rest = text;

    loop {
        // Find the next URL start
        let pos = rest.find("https://").or_else(|| rest.find("http://"));

        match pos {
            Some(start) => {
                result.push_str(&rest[..start]);
                // Skip the URL (until whitespace or end)
                let url_start = &rest[start..];
                let url_end = url_start
                    .find(|c: char| c.is_whitespace() || c == ')' || c == ']' || c == '>')
                    .unwrap_or(url_start.len());
                rest = &rest[start + url_end..];
            },
            None => {
                result.push_str(rest);
                break;
            },
        }
    }

    result
}

/// Strip bold/italic markers (`**`, `__`, and word-boundary `*`/`_`).
///
/// Preserves contractions like "don't" by only stripping `_` at word boundaries
/// (i.e. preceded/followed by whitespace or at string boundaries).
fn strip_bold_italic(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        // Double markers: ** or __
        if i + 1 < len && chars[i] == chars[i + 1] && (chars[i] == '*' || chars[i] == '_') {
            i += 2;
            continue;
        }

        // Single * is always a markdown marker (less common in prose)
        if chars[i] == '*' {
            i += 1;
            continue;
        }

        // Single _ only at word boundaries to preserve contractions
        if chars[i] == '_' {
            let at_start = i == 0 || chars[i - 1].is_whitespace();
            let at_end = i + 1 >= len || chars[i + 1].is_whitespace();
            if at_start || at_end {
                i += 1;
                continue;
            }
        }

        result.push(chars[i]);
        i += 1;
    }

    result
}

/// Collapse runs of multiple blank lines into a single newline.
fn collapse_whitespace(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut prev_was_newline = false;

    for ch in text.chars() {
        if ch == '\n' {
            if !prev_was_newline {
                result.push('\n');
            }
            prev_was_newline = true;
        } else {
            prev_was_newline = false;
            result.push(ch);
        }
    }

    result.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_noop_plain_text() {
        let text = "Hello, this is a plain response with no formatting.";
        let result = sanitize_text_for_tts(text);
        assert!(
            matches!(result, Cow::Borrowed(_)),
            "plain text should return Cow::Borrowed"
        );
        assert_eq!(result, text);
    }

    #[test]
    fn test_sanitize_strips_urls() {
        let text = "Check out https://example.com/foo for details.";
        let result = sanitize_text_for_tts(text);
        assert!(!result.contains("https://"));
        assert!(result.contains("Check out"));
        assert!(result.contains("for details."));
    }

    #[test]
    fn test_sanitize_strips_http_urls() {
        let text = "Visit http://example.com for more.";
        let result = sanitize_text_for_tts(text);
        assert!(!result.contains("http://"));
    }

    #[test]
    fn test_sanitize_strips_markdown_headers() {
        let text = "## Title\nSome content";
        let result = sanitize_text_for_tts(text);
        assert!(result.contains("Title"));
        assert!(!result.contains("##"));
    }

    #[test]
    fn test_sanitize_strips_code_fences() {
        let text = "Before\n```rust\nfn main() {}\n```\nAfter";
        let result = sanitize_text_for_tts(text);
        assert!(result.contains("Before"));
        assert!(result.contains("After"));
        assert!(!result.contains("fn main"));
        assert!(!result.contains("```"));
    }

    #[test]
    fn test_sanitize_strips_inline_code() {
        let text = "Run `cargo build` to compile.";
        let result = sanitize_text_for_tts(text);
        assert_eq!(result.as_ref(), "Run cargo build to compile.");
    }

    #[test]
    fn test_sanitize_strips_bold_italic() {
        let text = "This is **bold** and *italic* text.";
        let result = sanitize_text_for_tts(text);
        assert_eq!(result.as_ref(), "This is bold and italic text.");
    }

    #[test]
    fn test_sanitize_strips_underscore_bold_italic() {
        let text = "This is __bold__ and _italic_ text.";
        let result = sanitize_text_for_tts(text);
        assert!(result.contains("bold"));
        assert!(result.contains("italic"));
        assert!(!result.contains("__"));
    }

    #[test]
    fn test_sanitize_strips_bullet_lists() {
        let text = "Items:\n- First item\n- Second item";
        let result = sanitize_text_for_tts(text);
        assert!(result.contains("First item"));
        assert!(result.contains("Second item"));
        assert!(!result.contains("\n- "));
    }

    #[test]
    fn test_sanitize_strips_ordered_lists() {
        let text = "Steps:\n1. First step\n2. Second step";
        let result = sanitize_text_for_tts(text);
        assert!(result.contains("First step"));
        assert!(result.contains("Second step"));
        assert!(!result.contains("1. "));
    }

    #[test]
    fn test_sanitize_preserves_contractions() {
        let text = "I don't think it's a problem.";
        let result = sanitize_text_for_tts(text);
        assert_eq!(result.as_ref(), "I don't think it's a problem.");
    }

    #[test]
    fn test_sanitize_realistic_llm_output() {
        let text = "\
## Getting Started with Rust

Here's how to set up your project:

1. Install Rust from https://rustup.rs/
2. Run `cargo new my_project`
3. Edit `src/main.rs`

```rust
fn main() {
    println!(\"Hello, world!\");
}
```

For more details, check the **official documentation** at https://doc.rust-lang.org/book/.

- Use `cargo build` to compile
- Use `cargo test` to run tests";

        let result = sanitize_text_for_tts(text);

        // No URLs
        assert!(!result.contains("https://"));
        // No code fences or code content
        assert!(!result.contains("```"));
        assert!(!result.contains("println"));
        // No markdown formatting
        assert!(!result.contains("##"));
        assert!(!result.contains("**"));
        assert!(!result.contains('`'));
        // Content is preserved
        assert!(result.contains("Getting Started with Rust"));
        assert!(result.contains("Install Rust from"));
        assert!(result.contains("official documentation"));
    }

    #[test]
    fn test_sanitize_strips_ssml_tags() {
        let text = "Hello<break time=\"0.5s\"/> world";
        let result = sanitize_text_for_tts(text);
        assert!(!result.contains("<break"));
    }

    #[test]
    fn test_audio_format_mime_type() {
        assert_eq!(AudioFormat::Mp3.mime_type(), "audio/mpeg");
        assert_eq!(AudioFormat::Opus.mime_type(), "audio/ogg");
        assert_eq!(AudioFormat::Webm.mime_type(), "audio/webm");
        assert_eq!(AudioFormat::Aac.mime_type(), "audio/aac");
        assert_eq!(AudioFormat::Pcm.mime_type(), "audio/pcm");
    }

    #[test]
    fn test_audio_format_extension() {
        assert_eq!(AudioFormat::Mp3.extension(), "mp3");
        assert_eq!(AudioFormat::Opus.extension(), "ogg");
        assert_eq!(AudioFormat::Webm.extension(), "webm");
        assert_eq!(AudioFormat::Aac.extension(), "aac");
        assert_eq!(AudioFormat::Pcm.extension(), "pcm");
    }

    #[test]
    fn test_audio_format_from_content_type() {
        assert_eq!(
            AudioFormat::from_content_type("audio/webm"),
            Some(AudioFormat::Webm)
        );
        assert_eq!(
            AudioFormat::from_content_type("audio/webm;codecs=opus"),
            Some(AudioFormat::Webm)
        );
        assert_eq!(
            AudioFormat::from_content_type("audio/ogg"),
            Some(AudioFormat::Opus)
        );
        assert_eq!(
            AudioFormat::from_content_type("audio/mpeg"),
            Some(AudioFormat::Mp3)
        );
        assert_eq!(
            AudioFormat::from_content_type("audio/mp3"),
            Some(AudioFormat::Mp3)
        );
        assert_eq!(
            AudioFormat::from_content_type("audio/aac"),
            Some(AudioFormat::Aac)
        );
        assert_eq!(
            AudioFormat::from_content_type("audio/mp4"),
            Some(AudioFormat::Aac)
        );
        assert_eq!(
            AudioFormat::from_content_type("audio/pcm"),
            Some(AudioFormat::Pcm)
        );
        assert_eq!(
            AudioFormat::from_content_type("audio/wav"),
            Some(AudioFormat::Pcm)
        );
        assert_eq!(AudioFormat::from_content_type("image/png"), None);
        assert_eq!(AudioFormat::from_content_type("text/plain"), None);
    }

    #[test]
    fn test_audio_format_from_short_name() {
        assert_eq!(AudioFormat::from_short_name("webm"), AudioFormat::Webm);
        assert_eq!(AudioFormat::from_short_name("opus"), AudioFormat::Opus);
        assert_eq!(AudioFormat::from_short_name("ogg"), AudioFormat::Opus);
        assert_eq!(AudioFormat::from_short_name("aac"), AudioFormat::Aac);
        assert_eq!(AudioFormat::from_short_name("pcm"), AudioFormat::Pcm);
        assert_eq!(AudioFormat::from_short_name("wav"), AudioFormat::Pcm);
        assert_eq!(AudioFormat::from_short_name("mp3"), AudioFormat::Mp3);
        assert_eq!(AudioFormat::from_short_name("unknown"), AudioFormat::Mp3);
    }

    #[test]
    fn test_synthesize_request_default() {
        let req = SynthesizeRequest::default();
        assert!(req.text.is_empty());
        assert_eq!(req.output_format, AudioFormat::Mp3);
    }

    #[test]
    fn test_contains_ssml() {
        assert!(contains_ssml("Hello <break time=\"0.5s\"/> world"));
        assert!(contains_ssml("text<break/>more"));
        assert!(!contains_ssml("Hello world"));
        assert!(!contains_ssml("a <b>bold</b> tag"));
    }

    #[test]
    fn test_strip_ssml_tags_no_tags() {
        let text = "Hello world, no tags here";
        let result = strip_ssml_tags(text);
        assert!(matches!(result, Cow::Borrowed(_)));
        assert_eq!(result, "Hello world, no tags here");
    }

    #[test]
    fn test_strip_ssml_tags_self_closing() {
        assert_eq!(
            strip_ssml_tags("Hello...<break time=\"0.5s\"/>...world"),
            "Hello......world"
        );
    }

    #[test]
    fn test_strip_ssml_tags_multiple() {
        assert_eq!(
            strip_ssml_tags("A<break time=\"0.5s\"/> B<break time=\"0.7s\"/> C"),
            "A B C"
        );
    }

    #[test]
    fn test_strip_ssml_tags_at_boundaries() {
        assert_eq!(strip_ssml_tags("<break time=\"0.3s\"/>Hello"), "Hello");
        assert_eq!(strip_ssml_tags("Hello<break time=\"0.3s\"/>"), "Hello");
    }

    #[test]
    fn test_strip_ssml_tags_malformed() {
        // Malformed tag with no closing — kept as-is
        assert_eq!(
            strip_ssml_tags("Hello <break time=\"0.5s\""),
            "Hello <break time=\"0.5s\""
        );
    }
}
