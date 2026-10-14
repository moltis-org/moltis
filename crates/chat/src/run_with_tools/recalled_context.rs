/// Format memory search results into a `<recalled_context>` XML block
/// suitable for injection into the system prompt.
///
/// XML metacharacters in paths and text are escaped to prevent prompt
/// injection via crafted memory content.
pub(crate) fn format_recalled_context(results: &[moltis_memory::search::SearchResult]) -> String {
    if results.is_empty() {
        return String::new();
    }
    let mut out = String::from(
        "<recalled_context>\nRecalled from long-term memory as potentially relevant:\n\n",
    );
    for r in results {
        // Truncate long chunks to avoid prompt bloat.
        let text = if r.text.len() > 300 {
            format!("{}…", &r.text[..r.text.floor_char_boundary(300)])
        } else {
            r.text.clone()
        };
        // Escape XML metacharacters to prevent injection through memory content.
        let safe_path = escape_xml(&r.path);
        let safe_text = escape_xml(&text.replace('\n', " "));
        out.push_str(&format!("- [{safe_path}] {safe_text}\n"));
    }
    out.push_str("</recalled_context>");
    out
}

#[cfg(feature = "metrics")]
pub(super) fn record_prefetch_metric(status: &'static str, start: std::time::Instant) {
    use moltis_metrics::{counter, histogram, labels, memory as mem_metrics};
    counter!(mem_metrics::PREFETCH_TOTAL, labels::STATUS => status).increment(1);
    histogram!(mem_metrics::PREFETCH_DURATION_SECONDS).record(start.elapsed().as_secs_f64());
}

/// Escape XML metacharacters that could break prompt structure.
fn escape_xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
