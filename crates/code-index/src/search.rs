//! Code index search result adapter.
//!
//! Converts QMD search results into the crate's own [`SearchResult`] type,
//! keeping the public API decoupled from the QMD wire format.

use crate::types::SearchResult;

#[cfg(feature = "qmd")]
use moltis_qmd::QmdSearchResult;

/// Convert a [`QmdSearchResult`] into our crate-level [`SearchResult`].
///
/// Maps QMD fields to code-index fields, deriving the chunk ID from
/// the file path and line number.  The `end_line` is computed from
/// whichever text payload `text()` would return (preferring `body`
/// over `snippet`), so the line range is consistent with the text.
#[cfg(feature = "qmd")]
pub fn from_qmd(result: &QmdSearchResult, project_id: &str) -> SearchResult {
    let text = result.text();
    let start_line = (result.line as usize).max(1);
    let end_line = if text.is_empty() {
        start_line
    } else {
        start_line + text.lines().count().saturating_sub(1)
    };

    SearchResult {
        chunk_id: format!("{}:{}:{}", project_id, result.file, start_line),
        path: result.file.clone(),
        start_line,
        end_line,
        score: result.score,
        text,
        source: "qmd".to_string(),
    }
}

/// Convert multiple QMD results.
#[cfg(feature = "qmd")]
pub fn from_qmd_results(results: &[QmdSearchResult], project_id: &str) -> Vec<SearchResult> {
    results.iter().map(|r| from_qmd(r, project_id)).collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[cfg(feature = "qmd")]
    #[test]
    fn test_from_qmd_maps_fields() {
        let qmd = QmdSearchResult {
            docid: "test.rs#42".to_string(),
            file: "src/test.rs".to_string(),
            line: 42,
            score: 0.95,
            title: None,
            context: None,
            snippet: Some("fn main() {}".to_string()),
            body: None,
        };

        let result = from_qmd(&qmd, "my-project");
        assert_eq!(result.path, "src/test.rs");
        assert_eq!(result.start_line, 42);
        assert_eq!(result.end_line, 42); // single-line snippet
        assert_eq!(result.score, 0.95);
        assert!(result.chunk_id.contains("my-project"));
        assert_eq!(result.source, "qmd");
        assert_eq!(result.text, "fn main() {}"); // text prefers snippet when body is None
    }

    #[cfg(feature = "qmd")]
    #[test]
    fn test_from_qmd_uses_body_over_snippet() {
        let qmd = QmdSearchResult {
            docid: "test.rs#10".to_string(),
            file: "src/lib.rs".to_string(),
            line: 10,
            score: 0.8,
            title: None,
            context: None,
            snippet: Some("short".to_string()),
            body: Some("line one\nline two\nline three".to_string()),
        };

        let result = from_qmd(&qmd, "proj");
        // text() prefers body over snippet
        assert_eq!(result.text, "line one\nline two\nline three");
        // end_line should be derived from body (3 lines), not snippet (1 line)
        assert_eq!(result.start_line, 10);
        assert_eq!(result.end_line, 12); // line 10 + 3 lines - 1
    }

    #[cfg(feature = "qmd")]
    #[test]
    fn test_from_qmd_empty_text() {
        let qmd = QmdSearchResult {
            docid: "test.rs#5".to_string(),
            file: "src/mod.rs".to_string(),
            line: 5,
            score: 0.5,
            title: None,
            context: None,
            snippet: None,
            body: None,
        };

        let result = from_qmd(&qmd, "proj");
        assert_eq!(result.text, "");
        assert_eq!(result.start_line, 5);
        assert_eq!(result.end_line, 5); // single line when no text
    }
}
