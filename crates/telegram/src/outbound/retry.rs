//! Telegram API retry logic and request error handling.
//!
//! Provides retry-after handling for rate-limited requests, HTML-to-plain-text
//! fallback for send/edit operations, and the `RequestResultExt` trait for
//! ergonomic error conversion.

use {
    std::{future::Future, time::Duration},
    teloxide::{
        ApiError, RequestError,
        payloads::SendMessageSetters,
        prelude::*,
        types::{ChatId, MessageId, ParseMode, ThreadId},
    },
    tracing::warn,
};

use moltis_channels::{Error as ChannelError, Result};

use super::{
    TELEGRAM_RETRY_AFTER_MAX_RETRIES, TelegramOutbound, formatting::telegram_html_to_plain_text,
};

pub(crate) trait RequestResultExt<T> {
    fn channel_context(self, context: &'static str) -> Result<T>;
}

impl<T> RequestResultExt<T> for std::result::Result<T, RequestError> {
    fn channel_context(self, context: &'static str) -> Result<T> {
        self.map_err(|e| ChannelError::external(context, e))
    }
}

pub(crate) fn retry_after_duration(error: &RequestError) -> Option<Duration> {
    match error {
        RequestError::RetryAfter(wait) => Some(wait.duration()),
        _ => None,
    }
}

pub(crate) fn is_message_not_modified_error(error: &RequestError) -> bool {
    matches!(error, RequestError::Api(ApiError::MessageNotModified))
}

impl TelegramOutbound {
    pub(crate) async fn send_chunk_with_fallback(
        &self,
        bot: &Bot,
        account_id: &str,
        to: &str,
        chat_id: ChatId,
        thread_id: Option<ThreadId>,
        chunk: &str,
        reply_params: Option<&teloxide::types::ReplyParameters>,
        silent: bool,
    ) -> Result<MessageId> {
        match self
            .run_telegram_request_with_retry(account_id, to, "send message (html)", || {
                let mut html_req = bot.send_message(chat_id, chunk).parse_mode(ParseMode::Html);
                if silent {
                    html_req = html_req.disable_notification(true);
                }
                if let Some(tid) = thread_id {
                    html_req = html_req.message_thread_id(tid);
                }
                if let Some(rp) = reply_params {
                    html_req = html_req.reply_parameters(rp.clone());
                }
                async move { html_req.await }
            })
            .await
        {
            Ok(message) => Ok(message.id),
            Err(e) => {
                let plain_chunk = telegram_html_to_plain_text(chunk);
                warn!(
                    account_id,
                    chat_id = to,
                    error = %e,
                    "telegram HTML send failed, retrying as plain text"
                );
                let message = self
                    .run_telegram_request_with_retry(account_id, to, "send message (plain)", || {
                        let mut plain_req = bot.send_message(chat_id, &plain_chunk);
                        if silent {
                            plain_req = plain_req.disable_notification(true);
                        }
                        if let Some(tid) = thread_id {
                            plain_req = plain_req.message_thread_id(tid);
                        }
                        if let Some(rp) = reply_params {
                            plain_req = plain_req.reply_parameters(rp.clone());
                        }
                        async move { plain_req.await }
                    })
                    .await
                    .channel_context("send message (plain)")?;
                Ok(message.id)
            },
        }
    }

    pub(crate) async fn edit_chunk_with_fallback(
        &self,
        bot: &Bot,
        account_id: &str,
        to: &str,
        chat_id: ChatId,
        message_id: MessageId,
        chunk: &str,
    ) -> Result<()> {
        match self
            .run_telegram_request_with_retry(account_id, to, "edit message (html)", || {
                let html_req = bot
                    .edit_message_text(chat_id, message_id, chunk)
                    .parse_mode(ParseMode::Html);
                async move { html_req.await }
            })
            .await
        {
            Ok(_) => Ok(()),
            Err(e) => {
                if is_message_not_modified_error(&e) {
                    return Ok(());
                }
                let plain_chunk = telegram_html_to_plain_text(chunk);
                warn!(
                    account_id,
                    chat_id = to,
                    error = %e,
                    "telegram HTML edit failed, retrying as plain text"
                );
                match self
                    .run_telegram_request_with_retry(account_id, to, "edit message (plain)", || {
                        let plain_req = bot.edit_message_text(chat_id, message_id, &plain_chunk);
                        async move { plain_req.await }
                    })
                    .await
                {
                    Ok(_) => Ok(()),
                    Err(plain_err) => {
                        if is_message_not_modified_error(&plain_err) {
                            Ok(())
                        } else {
                            Err(ChannelError::external("edit message (plain)", plain_err))
                        }
                    },
                }
            },
        }
    }

    pub(crate) async fn run_telegram_request_with_retry<T, F, Fut>(
        &self,
        account_id: &str,
        to: &str,
        operation: &'static str,
        mut request: F,
    ) -> std::result::Result<T, RequestError>
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = std::result::Result<T, RequestError>>,
    {
        let mut retries = 0usize;

        loop {
            match request().await {
                Ok(value) => return Ok(value),
                Err(err) => {
                    let Some(wait) = retry_after_duration(&err) else {
                        return Err(err);
                    };

                    if retries >= TELEGRAM_RETRY_AFTER_MAX_RETRIES {
                        warn!(
                            account_id,
                            chat_id = to,
                            operation,
                            retries,
                            max_retries = TELEGRAM_RETRY_AFTER_MAX_RETRIES,
                            retry_after_secs = wait.as_secs(),
                            "telegram rate limit persisted after retries"
                        );
                        return Err(err);
                    }

                    retries += 1;
                    warn!(
                        account_id,
                        chat_id = to,
                        operation,
                        retries,
                        max_retries = TELEGRAM_RETRY_AFTER_MAX_RETRIES,
                        retry_after_secs = wait.as_secs(),
                        "telegram rate limited, waiting before retry"
                    );
                    tokio::time::sleep(wait).await;
                },
            }
        }
    }
}
