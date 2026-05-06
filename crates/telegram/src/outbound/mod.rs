//! Outbound message sender for Telegram.
//!
//! Split into submodules by domain:
//! - `formatting` -- HTML-to-plain-text fallback conversion
//! - `media` -- image, audio, document, and URL-based media sending
//! - `retry` -- rate-limit retry logic and request error helpers
//! - `send` -- `ChannelOutbound` trait implementation (text, HTML, location)
//! - `stream` -- `ChannelStreamOutbound` trait implementation (edit-in-place)

mod formatting;
mod media;
mod retry;
mod send;
mod stream;
#[cfg(test)]
mod tests;

use std::time::Duration;

use {
    moltis_channels::{Error as ChannelError, Result},
    teloxide::{prelude::*, types::ReplyParameters},
};

use crate::state::AccountStateMap;

/// Outbound message sender for Telegram.
pub struct TelegramOutbound {
    pub(crate) accounts: AccountStateMap,
}

const TELEGRAM_RETRY_AFTER_MAX_RETRIES: usize = 4;

/// How often to re-send the typing indicator while waiting for stream events.
/// Telegram typing indicators expire after ~5 s; refresh well before that.
const TYPING_REFRESH_INTERVAL: Duration = Duration::from_secs(4);

#[derive(Debug, Clone, Copy)]
struct StreamSendConfig {
    edit_throttle_ms: u64,
    notify_on_complete: bool,
    min_initial_chars: usize,
}

impl Default for StreamSendConfig {
    fn default() -> Self {
        Self {
            edit_throttle_ms: 300,
            notify_on_complete: false,
            min_initial_chars: 30,
        }
    }
}

impl TelegramOutbound {
    pub(crate) fn get_bot(&self, account_id: &str) -> Result<Bot> {
        let accounts = self.accounts.read().unwrap_or_else(|e| e.into_inner());
        accounts
            .get(account_id)
            .map(|s| s.bot.clone())
            .ok_or_else(|| ChannelError::unknown_account(account_id))
    }

    /// Build reply parameters only when `reply_to_message` is enabled for this account.
    pub(crate) fn reply_params(
        &self,
        account_id: &str,
        reply_to: Option<&str>,
    ) -> Option<ReplyParameters> {
        let accounts = self.accounts.read().unwrap_or_else(|e| e.into_inner());
        let enabled = accounts
            .get(account_id)
            .is_some_and(|s| s.config.reply_to_message);
        if enabled {
            parse_reply_params(reply_to)
        } else {
            None
        }
    }

    fn stream_send_config(&self, account_id: &str) -> StreamSendConfig {
        let accounts = self.accounts.read().unwrap_or_else(|e| e.into_inner());
        accounts
            .get(account_id)
            .map(|s| StreamSendConfig {
                edit_throttle_ms: s.config.edit_throttle_ms,
                notify_on_complete: s.config.stream_notify_on_complete,
                min_initial_chars: s.config.stream_min_initial_chars,
            })
            .unwrap_or_default()
    }
}

/// Parse a platform message ID string into Telegram `ReplyParameters`.
/// Returns `None` if the string is not a valid i32 (Telegram message IDs are i32).
fn parse_reply_params(reply_to: Option<&str>) -> Option<ReplyParameters> {
    reply_to.and_then(|id| id.parse::<i32>().ok()).map(|id| {
        ReplyParameters::new(teloxide::types::MessageId(id)).allow_sending_without_reply()
    })
}
