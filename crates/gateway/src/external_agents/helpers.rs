use std::time::SystemTime;

use moltis_sessions::MessageContent;

pub(super) fn message_content_text(content: &MessageContent) -> String {
    match content {
        MessageContent::Text(text) => text.clone(),
        MessageContent::Multimodal(blocks) => blocks
            .iter()
            .filter_map(|block| match block {
                moltis_sessions::ContentBlock::Text { text } => Some(text.as_str()),
                moltis_sessions::ContentBlock::ImageUrl { .. } => None,
            })
            .collect::<Vec<_>>()
            .join("\n"),
    }
}

pub(super) fn now_ms() -> u64 {
    match SystemTime::now().duration_since(SystemTime::UNIX_EPOCH) {
        Ok(duration) => duration.as_millis() as u64,
        Err(error) => {
            tracing::warn!(%error, "system clock is before UNIX_EPOCH");
            0
        },
    }
}
