/// Summarize old messages when token count approaches model context limit.
pub async fn compact_session(
    _messages: &[serde_json::Value],
) -> crate::Result<Vec<serde_json::Value>> {
    todo!("invoke LLM to summarize old turns, replace with compact summary")
}
