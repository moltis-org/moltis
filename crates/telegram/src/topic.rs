//! Forum-topic (thread) routing helpers for Telegram supergroups.

use std::num::ParseIntError;

use teloxide::types::{ChatId, MessageId, ThreadId};

/// Parse a composite `to` address into `(ChatId, Option<ThreadId>)`.
///
/// Forum-topic sends encode the thread as `"chat_id:thread_id"`.
/// Plain sends are just `"chat_id"`.
pub(crate) fn parse_chat_target(to: &str) -> Result<(ChatId, Option<ThreadId>), ParseIntError> {
    if let Some((chat_part, thread_part)) = to.split_once(':') {
        let chat_id = ChatId(chat_part.parse::<i64>()?);
        let thread_id = Some(ThreadId(MessageId(thread_part.parse::<i32>()?)));
        Ok((chat_id, thread_id))
    } else {
        Ok((ChatId(to.parse::<i64>()?), None))
    }
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_plain_chat_id() {
        let (chat, thread) = parse_chat_target("12345").unwrap();
        assert_eq!(chat, ChatId(12345));
        assert!(thread.is_none());
    }

    #[test]
    fn parse_negative_chat_id() {
        let (chat, thread) = parse_chat_target("-100999").unwrap();
        assert_eq!(chat, ChatId(-100999));
        assert!(thread.is_none());
    }

    #[test]
    fn parse_chat_with_thread() {
        let (chat, thread) = parse_chat_target("-100999:42").unwrap();
        assert_eq!(chat, ChatId(-100999));
        assert_eq!(thread.unwrap().0.0, 42);
    }

    #[test]
    fn parse_invalid_chat_id_is_err() {
        assert!(parse_chat_target("not_a_number").is_err());
    }

    #[test]
    fn parse_invalid_thread_id_is_err() {
        assert!(parse_chat_target("-100999:not_a_number").is_err());
    }
}
