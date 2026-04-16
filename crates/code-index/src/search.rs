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
/// the file path and line number.
#[cfg(feature = "qmd")]
pub fn from_qmd(result: &QmdSearchResult, project_id: &str) -> SearchResult {
    SearchResult {
        chunk_id: format!("{}:{}:{}", project_id, result.file, result.line),
        path: result.file.clone(),
        start_line: result.line as usize,
        end_line: result
            .snippet
            .as_ref()
            .map(|s| result.line as usize + s.lines().count().saturating_sub(1))
            .unwrap_or(result.line as usize),
        score: result.score,
        text: result.text(),
        source: "qmd".to_string(),
    }
}

/// Convert multiple QMD results.
#[cfg(feature = "qmd")]
pub fn from_qmd_results(results: &[QmdSearchResult], project_id: &str) -> Vec<SearchResult> {
    results.iter().map(|r| from_qmd(r, project_id)).collect()
}

#[cfg(test)]
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
        assert_eq!(result.score, 0.95);
        assert!(result.chunk_id.contains("my-project"));
        assert_eq!(result.source, "qmd");
    }
}