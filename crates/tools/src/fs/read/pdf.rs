//! PDF text extraction for the `Read` tool.
//!
//! Intercepts `.pdf` files before the binary rejection and extracts
//! text per-page using the pure-Rust `pdf-extract` crate.

use {
    serde_json::{Value, json},
    tokio::fs,
};

use crate::{Result, error::Error, fs::shared::require_absolute};

/// Maximum pages allowed in a single PDF Read call.
const MAX_PDF_PAGES_PER_REQUEST: usize = 20;

/// Parse a page-range string like `"1-5"`, `"3"`, or `"10-20"` into a
/// `(start, end)` pair (1-indexed, inclusive).
pub(crate) fn parse_page_range(raw: &str) -> Result<(usize, usize)> {
    let raw = raw.trim();
    if let Some((start_str, end_str)) = raw.split_once('-') {
        let start: usize = start_str
            .trim()
            .parse()
            .map_err(|_| Error::message(format!("invalid page range start: '{start_str}'")))?;
        let end: usize = end_str
            .trim()
            .parse()
            .map_err(|_| Error::message(format!("invalid page range end: '{end_str}'")))?;
        if start == 0 || end == 0 {
            return Err(Error::message(
                "page numbers are 1-indexed (0 is not valid)",
            ));
        }
        if start > end {
            return Err(Error::message(format!(
                "page range start ({start}) must be <= end ({end})"
            )));
        }
        Ok((start, end))
    } else {
        let page: usize = raw
            .parse()
            .map_err(|_| Error::message(format!("invalid page number: '{raw}'")))?;
        if page == 0 {
            return Err(Error::message(
                "page numbers are 1-indexed (0 is not valid)",
            ));
        }
        Ok((page, page))
    }
}

/// Read a PDF file and extract text by pages. Returns a structured
/// payload with `kind: "pdf"`.
///
/// Uses the `pdf-extract` crate which is pure Rust and works on all
/// platforms without native dependencies.
pub(crate) async fn read_pdf(file_path: &str, pages: Option<&str>) -> Result<Value> {
    require_absolute(file_path, "file_path")?;

    let path = std::path::PathBuf::from(file_path);
    if !fs::try_exists(&path).await.unwrap_or(false) {
        return Ok(json!({
            "kind": "not_found",
            "file_path": file_path,
            "error": "file does not exist",
            "detail": "",
        }));
    }

    let path_for_blocking = path.clone();
    let pages_owned = pages.map(str::to_string);

    tokio::task::spawn_blocking(move || -> Result<Value> {
        let all_pages = match pdf_extract::extract_text_by_pages(&path_for_blocking) {
            Ok(pages) => pages,
            Err(e) => {
                return Ok(json!({
                    "kind": "pdf_error",
                    "file_path": path_for_blocking.to_string_lossy(),
                    "error": format!("failed to extract text from PDF: {e}"),
                    "detail": "The file may be encrypted, corrupted, or use an unsupported PDF feature.",
                }));
            },
        };

        let total_pages = all_pages.len();

        let (selected_pages, start_page, end_page) = if let Some(ref range_str) = pages_owned {
            let (start, end) = parse_page_range(range_str)?;
            let end = end.min(total_pages);
            if start > total_pages {
                return Ok(json!({
                    "kind": "pdf",
                    "file_path": path_for_blocking.to_string_lossy(),
                    "total_pages": total_pages,
                    "pages_returned": 0,
                    "start_page": start,
                    "end_page": end,
                    "content": "",
                    "error": format!("start page {start} exceeds total pages {total_pages}"),
                }));
            }
            let page_count = end - start + 1;
            if page_count > MAX_PDF_PAGES_PER_REQUEST {
                return Err(Error::message(format!(
                    "page range {start}-{end} spans {page_count} pages; maximum is {MAX_PDF_PAGES_PER_REQUEST} per request"
                )));
            }
            let selected: Vec<&str> = all_pages[start - 1..end]
                .iter()
                .map(String::as_str)
                .collect();
            (selected, start, end)
        } else {
            // No pages specified: return all, capped at MAX_PDF_PAGES_PER_REQUEST.
            let end = total_pages.min(MAX_PDF_PAGES_PER_REQUEST);
            let selected: Vec<&str> = all_pages[..end]
                .iter()
                .map(String::as_str)
                .collect();
            (selected, 1, end)
        };

        // Build content with page markers.
        let mut content = String::new();
        for (idx, page_text) in selected_pages.iter().enumerate() {
            let page_num = start_page + idx;
            if !content.is_empty() {
                content.push_str("\n\n");
            }
            content.push_str(&format!("--- Page {page_num} ---\n"));
            content.push_str(page_text.trim());
        }

        Ok(json!({
            "kind": "pdf",
            "file_path": path_for_blocking.to_string_lossy(),
            "total_pages": total_pages,
            "pages_returned": selected_pages.len(),
            "start_page": start_page,
            "end_page": end_page,
            "truncated": end_page < total_pages,
            "content": content,
        }))
    })
    .await
    .map_err(|e| Error::message(format!("PDF extraction task failed: {e}")))?
}
