use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex, RwLock},
};

use {tokio_util::sync::CancellationToken, whatsapp_rust::client::Client};

use moltis_channels::{ChannelEventSink, message_log::MessageLog};

use crate::{config::WhatsAppAccountConfig, otp::OtpState};

/// Maximum number of sent message IDs to track for self-chat loop detection.
const SENT_IDS_CAPACITY: usize = 256;

/// Invisible Unicode watermark appended to every bot-sent message.
///
/// A sequence of ZWJ (U+200D) and ZWNJ (U+200C) characters that is:
/// - Invisible to the user
/// - Preserved by WhatsApp (both are required for emoji/script rendering)
/// - Linguistically meaningless as an alternating pattern
///
/// Used as a secondary self-chat loop detection alongside message-ID tracking.
pub(crate) const BOT_WATERMARK: &str = "\u{200D}\u{200C}\u{200D}\u{200C}";

/// Shared account state map.
pub type AccountStateMap = Arc<RwLock<HashMap<String, AccountState>>>;

/// Per-account runtime state.
pub struct AccountState {
    pub client: Arc<Client>,
    pub account_id: String,
    pub config: WhatsAppAccountConfig,
    pub cancel: CancellationToken,
    pub message_log: Option<Arc<dyn MessageLog>>,
    pub event_sink: Option<Arc<dyn ChannelEventSink>>,
    /// Latest QR code data for the pairing flow (updated every ~20s).
    pub latest_qr: RwLock<Option<String>>,
    /// Whether the client is currently connected.
    pub connected: std::sync::atomic::AtomicBool,
    /// In-memory OTP challenges for self-approval (std::sync::Mutex because
    /// all OTP operations are synchronous HashMap lookups, never held across
    /// `.await` points).
    pub otp: Mutex<OtpState>,
    /// Recently sent message IDs, used to distinguish bot echoes from user
    /// messages in self-chat. When the bot sends a message, the ID is recorded
    /// here. Incoming `is_from_me` messages whose ID matches are bot echoes
    /// and get skipped; non-matching ones are genuine user messages from
    /// another device (phone, WhatsApp Web) and get processed.
    pub(crate) recent_sent_ids: Mutex<VecDeque<String>>,
}

impl AccountState {
    /// Record a message ID that was sent by the bot, for self-chat loop detection.
    pub fn record_sent_id(&self, id: &str) {
        let mut ids = self
            .recent_sent_ids
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if ids.len() >= SENT_IDS_CAPACITY {
            ids.pop_front();
        }
        ids.push_back(id.to_string());
    }

    /// Check if a message ID was recently sent by the bot.
    pub fn was_sent_by_us(&self, id: &str) -> bool {
        let ids = self
            .recent_sent_ids
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        ids.iter().any(|sent_id| sent_id == id)
    }

    /// Send a WhatsApp message and record its ID for self-chat loop detection.
    /// Appends an invisible watermark to text messages for secondary loop detection.
    pub async fn send_message(
        &self,
        to: wacore_binary::jid::Jid,
        mut msg: waproto::whatsapp::Message,
    ) -> crate::Result<()> {
        watermark_message(&mut msg);
        let msg_id =
            self.client
                .send_message(to, msg)
                .await
                .map_err(|e| crate::Error::Whatsapp {
                    message: e.to_string(),
                })?;
        self.record_sent_id(&msg_id);
        Ok(())
    }
}

/// Append the invisible bot watermark to a message's text content.
pub(crate) fn watermark_message(msg: &mut waproto::whatsapp::Message) {
    if let Some(ref mut text) = msg.conversation {
        text.push_str(BOT_WATERMARK);
    }
    if let Some(ref mut ext) = msg.extended_text_message
        && let Some(ref mut text) = ext.text
    {
        text.push_str(BOT_WATERMARK);
    }
}

/// Check if a message text contains the bot watermark.
pub(crate) fn has_bot_watermark(text: &str) -> bool {
    text.ends_with(BOT_WATERMARK)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    /// Helper to create a bare-bones sent-ID tracker (same data structure as AccountState).
    fn new_tracker() -> Mutex<VecDeque<String>> {
        Mutex::new(VecDeque::new())
    }

    fn record(tracker: &Mutex<VecDeque<String>>, id: &str) {
        let mut ids = tracker.lock().unwrap();
        if ids.len() >= SENT_IDS_CAPACITY {
            ids.pop_front();
        }
        ids.push_back(id.to_string());
    }

    fn was_sent(tracker: &Mutex<VecDeque<String>>, id: &str) -> bool {
        let ids = tracker.lock().unwrap();
        ids.iter().any(|sent_id| sent_id == id)
    }

    #[test]
    fn sent_id_tracking_basic() {
        let tracker = new_tracker();
        assert!(!was_sent(&tracker, "msg1"));
        record(&tracker, "msg1");
        assert!(was_sent(&tracker, "msg1"));
        assert!(!was_sent(&tracker, "msg2"));
    }

    #[test]
    fn sent_id_tracking_evicts_oldest() {
        let tracker = new_tracker();
        for i in 0..SENT_IDS_CAPACITY {
            record(&tracker, &format!("msg{i}"));
        }
        // All 256 IDs should be present.
        assert!(was_sent(&tracker, "msg0"));
        assert!(was_sent(&tracker, &format!("msg{}", SENT_IDS_CAPACITY - 1)));

        // Adding one more should evict the oldest.
        record(&tracker, "overflow");
        assert!(!was_sent(&tracker, "msg0"));
        assert!(was_sent(&tracker, "msg1"));
        assert!(was_sent(&tracker, "overflow"));
    }

    #[test]
    fn sent_id_tracking_no_false_positives() {
        let tracker = new_tracker();
        record(&tracker, "abc123");
        assert!(!was_sent(&tracker, "abc12"));
        assert!(!was_sent(&tracker, "abc1234"));
        assert!(!was_sent(&tracker, "ABC123"));
    }

    #[test]
    fn watermark_appended_to_conversation() {
        let mut msg = waproto::whatsapp::Message {
            conversation: Some("Hello".into()),
            ..Default::default()
        };
        watermark_message(&mut msg);
        assert_eq!(
            msg.conversation.as_deref(),
            Some("Hello\u{200D}\u{200C}\u{200D}\u{200C}")
        );
        assert!(has_bot_watermark(msg.conversation.as_deref().unwrap()));
    }

    #[test]
    fn watermark_appended_to_extended_text() {
        let mut msg = waproto::whatsapp::Message {
            extended_text_message: Some(Box::new(
                waproto::whatsapp::message::ExtendedTextMessage {
                    text: Some("Hello".into()),
                    ..Default::default()
                },
            )),
            ..Default::default()
        };
        watermark_message(&mut msg);
        let text = msg.extended_text_message.unwrap().text.unwrap();
        assert!(has_bot_watermark(&text));
    }

    #[test]
    fn watermark_not_present_in_plain_text() {
        assert!(!has_bot_watermark("Hello world"));
        assert!(!has_bot_watermark(""));
        assert!(!has_bot_watermark("some \u{200D} text"));
    }

    #[test]
    fn watermark_skips_message_without_text() {
        let mut msg = waproto::whatsapp::Message::default();
        watermark_message(&mut msg);
        assert!(msg.conversation.is_none());
        assert!(msg.extended_text_message.is_none());
    }
}
