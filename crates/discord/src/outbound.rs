use std::time::Duration;

use {
    async_trait::async_trait,
    base64::Engine,
    tracing::{debug, info, warn},
};

use {
    moltis_channels::{
        Error as ChannelError, Result as ChannelResult,
        plugin::{ChannelOutbound, ChannelStreamOutbound, StreamEvent, StreamReceiver},
    },
    moltis_common::types::ReplyPayload,
    serenity::all::{
        ChannelId, CreateAttachment, CreateEmbed, CreateMessage, EditMessage, MessageId,
        ReactionType,
    },
};

use crate::{
    handler::{send_discord_message, send_discord_text},
    state::AccountStateMap,
};

// ── Constants ────────────────────────────────────────────────────────

/// Discord enforces a 2 000-character limit per message.
const DISCORD_MAX_MESSAGE_LEN: usize = 2000;
/// Discord embed description character limit.
const DISCORD_MAX_EMBED_DESCRIPTION_LEN: usize = 4096;

/// Minimum chars before the first message is sent during streaming.
const STREAM_MIN_INITIAL_CHARS: usize = 30;

/// Throttle interval between edit-in-place updates during streaming.
const STREAM_EDIT_THROTTLE: Duration = Duration::from_millis(500);

/// How often to re-send the typing indicator while waiting for stream events.
/// Discord typing indicators expire after ~10 s; refresh well before that.
const TYPING_REFRESH_INTERVAL: Duration = Duration::from_secs(8);

/// Send a lightweight preview first for larger images so users on slower
/// links get visual feedback quickly while the full upload is still in flight.
const DISCORD_IMAGE_PREVIEW_TRIGGER_BYTES: usize = 400 * 1024;
const DISCORD_IMAGE_PREVIEW_MAX_WIDTH: u32 = 1024;
const DISCORD_IMAGE_PREVIEW_MAX_HEIGHT: u32 = 1024;
const DISCORD_IMAGE_PREVIEW_TEXT: &str = "Preview while full image uploads...";

// ── HTML-to-Discord conversion ───────────────────────────────────────

/// Convert the HTML activity-log suffix (Telegram-flavoured) into Discord
/// markdown.  Handles `<blockquote expandable>`, `<b>`, `<i>`, `<code>`, and
/// HTML entities produced by `format_logbook_html`.
fn html_suffix_to_discord(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_blockquote = false;
    let mut chars = html.chars().peekable();

    while let Some(&ch) = chars.peek() {
        if ch == '<' {
            // Consume the tag.
            let mut tag = String::new();
            for c in chars.by_ref() {
                tag.push(c);
                if c == '>' {
                    break;
                }
            }
            let lower = tag
                .trim_start_matches('<')
                .trim_end_matches('>')
                .trim()
                .to_ascii_lowercase();

            match lower.as_str() {
                s if s.starts_with("blockquote") => in_blockquote = true,
                "/blockquote" => in_blockquote = false,
                "b" | "strong" | "/b" | "/strong" => out.push_str("**"),
                "i" | "em" | "/i" | "/em" => out.push('*'),
                "code" | "/code" => out.push('`'),
                "br" | "br/" | "br /" => out.push('\n'),
                _ => {}, // Other tags are silently dropped.
            }
        } else if ch == '&' {
            // Decode HTML entities.
            let mut entity = String::new();
            for c in chars.by_ref() {
                entity.push(c);
                if c == ';' || entity.len() > 10 {
                    break;
                }
            }
            match entity.as_str() {
                "&amp;" => out.push('&'),
                "&lt;" => out.push('<'),
                "&gt;" => out.push('>'),
                "&quot;" => out.push('"'),
                "&#39;" | "&apos;" => out.push('\''),
                _ => out.push_str(&entity),
            }
        } else {
            chars.next();
            if ch == '\n' && in_blockquote {
                out.push('\n');
                if let Some(&next) = chars.peek()
                    && (next != '<' || !peek_closing_blockquote(&chars))
                    && next != '\n'
                {
                    out.push_str("> ");
                }
            } else {
                out.push(ch);
            }
        }
    }
    out
}

/// Peek ahead to check if the next tag is `</blockquote>`.
fn peek_closing_blockquote(chars: &std::iter::Peekable<std::str::Chars<'_>>) -> bool {
    let rest: String = chars.clone().take(14).collect();
    rest.to_ascii_lowercase().starts_with("</blockquote>")
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ActivityLogRender {
    description: String,
    has_errors: bool,
}

fn render_activity_log_for_discord(html: &str) -> Option<ActivityLogRender> {
    let converted = html_suffix_to_discord(html);
    let mut lines: Vec<String> = converted
        .lines()
        .map(str::trim)
        .map(|line| {
            let no_quote = if let Some(stripped) = line.strip_prefix("> ") {
                stripped
            } else if let Some(stripped) = line.strip_prefix('>') {
                stripped.trim_start()
            } else {
                line
            };
            no_quote.trim().to_string()
        })
        .filter(|line| !line.is_empty())
        .collect();

    if lines.is_empty() {
        return None;
    }

    if lines
        .first()
        .is_some_and(|line| line.to_ascii_lowercase().contains("activity log"))
    {
        lines.remove(0);
    }

    if lines.is_empty() {
        return None;
    }

    let has_errors = lines.iter().any(|line| line.contains('\u{274C}'));
    let mut entries = Vec::with_capacity(lines.len());
    for line in lines {
        let normalized = line
            .strip_prefix('\u{2022}')
            .map(str::trim_start)
            .unwrap_or(line.trim());
        if normalized.is_empty() {
            continue;
        }
        entries.push(format!("\u{2022} {normalized}"));
    }

    if entries.is_empty() {
        return None;
    }

    let description = entries.join("\n");
    let description = if description.len() > DISCORD_MAX_EMBED_DESCRIPTION_LEN {
        let max = DISCORD_MAX_EMBED_DESCRIPTION_LEN.saturating_sub("\n...".len());
        format!("{}\n...", truncate_at_char_boundary(&description, max))
    } else {
        description
    };

    Some(ActivityLogRender {
        description,
        has_errors,
    })
}

// ── Media helpers ────────────────────────────────────────────────────

/// Decode a `data:<mime>;base64,<payload>` URI into raw bytes.
fn decode_data_url(url: &str) -> ChannelResult<Vec<u8>> {
    let comma_pos = url
        .find(',')
        .ok_or_else(|| ChannelError::invalid_input("invalid data URI: no comma separator"))?;
    let base64_data = &url[comma_pos + 1..];
    base64::engine::general_purpose::STANDARD
        .decode(base64_data)
        .map_err(|e| ChannelError::invalid_input(format!("failed to decode base64: {e}")))
}

/// Pick a sensible filename extension from a MIME type.
fn extension_for_mime(mime: &str) -> &'static str {
    match mime {
        "image/png" => "png",
        "image/jpeg" | "image/jpg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/x-portable-pixmap" => "ppm",
        "audio/ogg" => "ogg",
        "audio/mpeg" | "audio/mp3" => "mp3",
        "video/mp4" => "mp4",
        "application/pdf" => "pdf",
        _ => "bin",
    }
}

fn is_image_mime(mime: &str) -> bool {
    mime.starts_with("image/")
}

fn build_upload_preview(data: &[u8], mime: &str) -> Option<(Vec<u8>, String)> {
    if !is_image_mime(mime) || data.len() < DISCORD_IMAGE_PREVIEW_TRIGGER_BYTES {
        return None;
    }

    let resized = moltis_media::image_ops::resize_image(
        data,
        DISCORD_IMAGE_PREVIEW_MAX_WIDTH,
        DISCORD_IMAGE_PREVIEW_MAX_HEIGHT,
    )
    .ok()?;

    if resized == data || resized.len() >= data.len() {
        return None;
    }

    // `resize_image` re-encodes resized outputs as JPEG.
    Some((resized, "image/jpeg".to_string()))
}

// ── Outbound sender ──────────────────────────────────────────────────

/// Outbound sender for Discord channel accounts.
pub struct DiscordOutbound {
    pub(crate) accounts: AccountStateMap,
}

impl DiscordOutbound {
    fn resolve_http(
        &self,
        account_id: &str,
    ) -> ChannelResult<std::sync::Arc<serenity::http::Http>> {
        let accounts = self.accounts.read().unwrap_or_else(|e| e.into_inner());
        let state = accounts
            .get(account_id)
            .ok_or_else(|| ChannelError::unknown_account(account_id))?;
        state.http.clone().ok_or_else(|| {
            ChannelError::unavailable(format!(
                "Discord bot for account '{account_id}' is not connected yet"
            ))
        })
    }

    fn parse_channel_id(to: &str) -> ChannelResult<ChannelId> {
        to.parse::<u64>()
            .map(ChannelId::new)
            .map_err(|_| ChannelError::invalid_input(format!("invalid Discord channel ID: {to}")))
    }

    /// Parse the `reply_to` message ID into a `MessageId` when `reply_to_message`
    /// is enabled for the account.
    fn resolve_reference(&self, account_id: &str, reply_to: Option<&str>) -> Option<MessageId> {
        let accounts = self.accounts.read().unwrap_or_else(|e| e.into_inner());
        let enabled = accounts
            .get(account_id)
            .is_some_and(|s| s.config.reply_to_message);
        if enabled {
            reply_to
                .and_then(|id| id.parse::<u64>().ok())
                .map(MessageId::new)
        } else {
            None
        }
    }

    /// Remove the ack reaction from the original message after the bot's
    /// response is complete.
    ///
    /// Accepts the already-resolved `http` handle to avoid re-acquiring the
    /// account state lock.
    async fn remove_ack_reaction(
        &self,
        account_id: &str,
        http: &serenity::http::Http,
        channel_id: ChannelId,
        reply_to: Option<&str>,
    ) {
        let emoji = {
            let accounts = self.accounts.read().unwrap_or_else(|e| e.into_inner());
            let Some(state) = accounts.get(account_id) else {
                return;
            };
            let Some(ref emoji) = state.config.ack_reaction else {
                return;
            };
            emoji.clone()
        };
        let Some(msg_id) = reply_to.and_then(|id| id.parse::<u64>().ok()) else {
            return;
        };
        let reaction = ReactionType::Unicode(emoji);
        if let Err(e) = http
            .delete_reaction_me(channel_id, MessageId::new(msg_id), &reaction)
            .await
        {
            debug!(account_id, "failed to remove ack reaction: {e}");
        }
    }

    async fn send_activity_log_embed(
        &self,
        account_id: &str,
        http: &serenity::http::Http,
        channel_id: ChannelId,
        suffix_html: &str,
        reply_to: Option<&str>,
    ) -> ChannelResult<()> {
        let Some(rendered) = render_activity_log_for_discord(suffix_html) else {
            return Ok(());
        };

        let color: u32 = if rendered.has_errors {
            0xED4245 // Discord red for error entries
        } else {
            0x5865F2 // Discord blurple for normal activity
        };
        let embed = CreateEmbed::new()
            .title("Tool activity")
            .description(rendered.description)
            .color(color);
        let mut msg = CreateMessage::new().embed(embed);
        if let Some(reference) = self.resolve_reference(account_id, reply_to) {
            msg = msg.reference_message((channel_id, reference));
        }

        channel_id.send_message(http, msg).await.map_err(|e| {
            ChannelError::external("Discord send embed", std::io::Error::other(e.to_string()))
        })?;
        Ok(())
    }

    /// Inner implementation for `send_text_with_suffix` that does not handle
    /// ack reaction removal -- the caller is responsible for that.
    async fn send_text_with_suffix_inner(
        &self,
        account_id: &str,
        http: &serenity::http::Http,
        channel_id: ChannelId,
        text: &str,
        suffix_html: &str,
        reply_to: Option<&str>,
    ) -> ChannelResult<()> {
        // Send main response text.
        let reference = self.resolve_reference(account_id, reply_to);
        send_discord_message(http, channel_id, text, reference)
            .await
            .map_err(|e| ChannelError::external("Discord send", std::io::Error::other(e)))?;

        // Send the activity log as a separate embed message.
        self.send_activity_log_embed(account_id, http, channel_id, suffix_html, None)
            .await?;

        Ok(())
    }
}

#[async_trait]
impl ChannelOutbound for DiscordOutbound {
    async fn send_text(
        &self,
        account_id: &str,
        to: &str,
        text: &str,
        reply_to: Option<&str>,
    ) -> ChannelResult<()> {
        let http = self.resolve_http(account_id)?;
        let channel_id = Self::parse_channel_id(to)?;
        let reference = self.resolve_reference(account_id, reply_to);
        info!(
            account_id,
            chat_id = to,
            text_len = text.len(),
            reply_ref = reference.is_some(),
            "discord outbound text send"
        );
        send_discord_message(&http, channel_id, text, reference)
            .await
            .map_err(|e| ChannelError::external("Discord send", std::io::Error::other(e)))?;
        Ok(())
    }

    async fn send_media(
        &self,
        account_id: &str,
        to: &str,
        payload: &ReplyPayload,
        reply_to: Option<&str>,
    ) -> ChannelResult<()> {
        let Some(media) = payload.media.as_ref() else {
            return self
                .send_text(account_id, to, &payload.text, reply_to)
                .await;
        };

        info!(
            account_id,
            chat_id = to,
            media_mime = %media.mime_type,
            caption_len = payload.text.len(),
            is_data_url = media.url.starts_with("data:"),
            "discord outbound media send"
        );

        if media.url.starts_with("data:") {
            let bytes = decode_data_url(&media.url)?;
            let ext = extension_for_mime(&media.mime_type);
            let filename = format!("attachment.{ext}");
            let preview = build_upload_preview(&bytes, &media.mime_type);

            debug!(
                account_id,
                bytes = bytes.len(),
                mime_type = %media.mime_type,
                filename,
                "sending base64 media to discord"
            );

            let http = self.resolve_http(account_id)?;
            let channel_id = Self::parse_channel_id(to)?;
            let reference = self.resolve_reference(account_id, reply_to);
            if let Some((preview_bytes, preview_mime)) = preview {
                let preview_filename = format!("preview.{}", extension_for_mime(&preview_mime));
                info!(
                    account_id,
                    chat_id = to,
                    preview_bytes = preview_bytes.len(),
                    preview_mime = %preview_mime,
                    "discord outbound media preview send"
                );

                let preview_attachment = CreateAttachment::bytes(preview_bytes, preview_filename);
                let mut preview_msg = CreateMessage::new()
                    .content(DISCORD_IMAGE_PREVIEW_TEXT)
                    .add_file(preview_attachment);
                if let Some(ref_id) = reference {
                    preview_msg = preview_msg.reference_message((channel_id, ref_id));
                }

                if let Err(e) = channel_id.send_message(&http, preview_msg).await {
                    warn!(
                        account_id,
                        chat_id = to,
                        error = %e,
                        "failed to send discord image preview (continuing with full image)"
                    );
                } else {
                    info!(
                        account_id,
                        chat_id = to,
                        preview_mime = %preview_mime,
                        "discord outbound media preview sent"
                    );
                }
            }

            let attachment = CreateAttachment::bytes(bytes, filename);
            let mut msg = CreateMessage::new().add_file(attachment);
            if !payload.text.is_empty() {
                msg = msg.content(&payload.text);
            }
            if let Some(ref_id) = reference {
                msg = msg.reference_message((channel_id, ref_id));
            }
            channel_id.send_message(&http, msg).await.map_err(|e| {
                ChannelError::external("Discord send media", std::io::Error::other(e.to_string()))
            })?;

            info!(
                account_id,
                chat_id = to,
                media_mime = %media.mime_type,
                "discord outbound media sent"
            );
            return Ok(());
        }

        // Regular URL — include inline.
        let mut text = payload.text.clone();
        if !text.is_empty() {
            text.push_str("\n\n");
        }
        text.push_str(&media.url);
        self.send_text(account_id, to, &text, reply_to).await
    }

    async fn send_typing(&self, account_id: &str, to: &str) -> ChannelResult<()> {
        let http = self.resolve_http(account_id)?;
        let channel_id = Self::parse_channel_id(to)?;
        channel_id.broadcast_typing(&http).await.map_err(|e| {
            ChannelError::external("Discord typing", std::io::Error::other(e.to_string()))
        })
    }

    async fn send_text_with_suffix(
        &self,
        account_id: &str,
        to: &str,
        text: &str,
        suffix_html: &str,
        reply_to: Option<&str>,
    ) -> ChannelResult<()> {
        let http = self.resolve_http(account_id)?;
        let channel_id = Self::parse_channel_id(to)?;

        info!(
            account_id,
            chat_id = to,
            text_len = text.len(),
            suffix_len = suffix_html.len(),
            "discord outbound text+suffix send"
        );

        let result = self
            .send_text_with_suffix_inner(account_id, &http, channel_id, text, suffix_html, reply_to)
            .await;

        // Always remove the ack reaction, even if sending failed.
        self.remove_ack_reaction(account_id, &http, channel_id, reply_to)
            .await;

        result
    }

    async fn send_html(
        &self,
        account_id: &str,
        to: &str,
        html: &str,
        reply_to: Option<&str>,
    ) -> ChannelResult<()> {
        let http = self.resolve_http(account_id)?;
        let channel_id = Self::parse_channel_id(to)?;
        self.send_activity_log_embed(account_id, &http, channel_id, html, reply_to)
            .await
    }

    async fn send_location(
        &self,
        account_id: &str,
        to: &str,
        latitude: f64,
        longitude: f64,
        title: Option<&str>,
        reply_to: Option<&str>,
    ) -> ChannelResult<()> {
        let mut text = String::new();
        if let Some(title) = title {
            text.push_str(title);
            text.push('\n');
        }
        text.push_str(&format!(
            "https://www.google.com/maps?q={latitude:.6},{longitude:.6}"
        ));
        info!(
            account_id,
            chat_id = to,
            latitude,
            longitude,
            "discord outbound location send"
        );
        self.send_text(account_id, to, &text, reply_to).await
    }
}

// ── Streaming ────────────────────────────────────────────────────────

/// Truncate a string to at most `max` characters on a char boundary.
fn truncate_at_char_boundary(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

#[async_trait]
impl ChannelStreamOutbound for DiscordOutbound {
    async fn send_stream(
        &self,
        account_id: &str,
        to: &str,
        reply_to: Option<&str>,
        mut stream: StreamReceiver,
    ) -> ChannelResult<()> {
        let http = self.resolve_http(account_id)?;
        let channel_id = Self::parse_channel_id(to)?;
        let reference = self.resolve_reference(account_id, reply_to);

        // Send typing indicator.
        let _ = channel_id.broadcast_typing(&http).await;

        let mut accumulated = String::new();
        let mut sent_message_id: Option<MessageId> = None;
        let mut last_edit = tokio::time::Instant::now();
        let mut typing_interval = tokio::time::interval(TYPING_REFRESH_INTERVAL);
        typing_interval.tick().await; // consume the immediate first tick

        info!(account_id, chat_id = to, "discord stream started");

        loop {
            tokio::select! {
                event = stream.recv() => {
                    let Some(event) = event else { break };
                    match event {
                        StreamEvent::Delta(delta) => {
                            accumulated.push_str(&delta);

                            // Phase 1: initial send once we have enough text.
                            if sent_message_id.is_none() {
                                if accumulated.chars().count() >= STREAM_MIN_INITIAL_CHARS {
                                    let display =
                                        truncate_at_char_boundary(&accumulated, DISCORD_MAX_MESSAGE_LEN);
                                    match send_discord_message(&http, channel_id, display, reference).await
                                    {
                                        Ok(msg) => {
                                            sent_message_id = Some(msg.id);
                                            last_edit = tokio::time::Instant::now();
                                        },
                                        Err(e) => {
                                            warn!(
                                                account_id,
                                                chat_id = to,
                                                error = %e,
                                                "discord stream initial send failed"
                                            );
                                        },
                                    }
                                }
                                continue;
                            }

                            // Phase 2: throttled in-place edits.
                            if last_edit.elapsed() >= STREAM_EDIT_THROTTLE
                                && let Some(msg_id) = sent_message_id
                            {
                                let display =
                                    truncate_at_char_boundary(&accumulated, DISCORD_MAX_MESSAGE_LEN);
                                let edit = EditMessage::new().content(display);
                                if let Err(e) = channel_id.edit_message(&http, msg_id, edit).await {
                                    debug!(
                                        account_id,
                                        chat_id = to,
                                        error = %e,
                                        "discord stream edit failed (non-fatal)"
                                    );
                                }
                                last_edit = tokio::time::Instant::now();
                            }
                        },
                        StreamEvent::Done => break,
                        StreamEvent::Error(err) => {
                            warn!(account_id, chat_id = to, error = %err, "discord stream error");
                            if accumulated.is_empty() {
                                accumulated = err;
                            }
                            break;
                        },
                    }
                }
                _ = typing_interval.tick() => {
                    // Re-send typing indicator to keep it visible during
                    // long-running tool execution or pauses in the stream.
                    let _ = channel_id.broadcast_typing(&http).await;
                }
            }
        }

        // Phase 3: final update with the complete text.
        if !accumulated.is_empty() {
            if accumulated.len() <= DISCORD_MAX_MESSAGE_LEN {
                // Content fits in one message -- edit or send.
                if let Some(msg_id) = sent_message_id {
                    let edit = EditMessage::new().content(&accumulated);
                    if let Err(e) = channel_id.edit_message(&http, msg_id, edit).await {
                        warn!(
                            account_id,
                            chat_id = to,
                            error = %e,
                            "discord stream final edit failed"
                        );
                    }
                } else {
                    send_discord_message(&http, channel_id, &accumulated, None)
                        .await
                        .map_err(|e| {
                            ChannelError::external("Discord send", std::io::Error::other(e))
                        })?;
                }
            } else {
                // Content overflows -- edit the first message with the first 2000 chars,
                // then send the rest as new messages.
                let first = truncate_at_char_boundary(&accumulated, DISCORD_MAX_MESSAGE_LEN);
                if let Some(msg_id) = sent_message_id {
                    let edit = EditMessage::new().content(first);
                    let _ = channel_id.edit_message(&http, msg_id, edit).await;
                } else {
                    let _ = send_discord_text(&http, channel_id, first).await;
                }

                let rest = &accumulated[first.len()..];
                if !rest.is_empty() {
                    send_discord_text(&http, channel_id, rest)
                        .await
                        .map_err(|e| {
                            ChannelError::external(
                                "Discord stream overflow",
                                std::io::Error::other(e),
                            )
                        })?;
                }
            }
        }

        // Always remove ack reaction, even if the stream produced no content.
        self.remove_ack_reaction(account_id, &http, channel_id, reply_to)
            .await;

        info!(
            account_id,
            chat_id = to,
            total_len = accumulated.len(),
            streamed = sent_message_id.is_some(),
            "discord stream completed"
        );
        Ok(())
    }

    async fn is_stream_enabled(&self, _account_id: &str) -> bool {
        true
    }
}

// ── Tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use {super::*, std::io::Cursor};

    #[test]
    fn logbook_html_to_discord_blockquote() {
        let html = "<blockquote expandable>\n\
                     \u{1f4cb} <b>Activity log</b>\n\
                     \u{2022} Using GPT 5.2 Codex (Codex/OAuth). Use /model to change.\n\
                     </blockquote>";
        let result = html_suffix_to_discord(html);
        let trimmed = result.trim();
        assert_eq!(
            trimmed,
            "> \u{1f4cb} **Activity log**\n> \u{2022} Using GPT 5.2 Codex (Codex/OAuth). Use /model to change."
        );
    }

    #[test]
    fn logbook_html_multiple_entries() {
        let html = "<blockquote expandable>\n\
                     \u{1f4cb} <b>Activity log</b>\n\
                     \u{2022} First entry\n\
                     \u{2022} Second entry\n\
                     </blockquote>";
        let result = html_suffix_to_discord(html);
        let trimmed = result.trim();
        assert!(trimmed.starts_with("> "));
        assert!(trimmed.contains("> \u{2022} First entry\n> \u{2022} Second entry"));
    }

    #[test]
    fn html_entities_decoded() {
        let html = "foo &amp; bar &lt;baz&gt;";
        assert_eq!(html_suffix_to_discord(html), "foo & bar <baz>");
    }

    #[test]
    fn bold_and_italic_converted() {
        let html = "<b>bold</b> and <i>italic</i>";
        assert_eq!(html_suffix_to_discord(html), "**bold** and *italic*");
    }

    #[test]
    fn code_converted() {
        let html = "use <code>/model</code> to change";
        assert_eq!(html_suffix_to_discord(html), "use `/model` to change");
    }

    #[test]
    fn plain_text_unchanged() {
        let html = "Hello world";
        assert_eq!(html_suffix_to_discord(html), "Hello world");
    }

    #[test]
    fn empty_string() {
        assert_eq!(html_suffix_to_discord(""), "");
    }

    #[test]
    fn render_activity_log_strips_header_and_quotes() {
        let html = "<blockquote expandable>\n\
                     \u{1f4cb} <b>Activity log</b>\n\
                     \u{2022} First entry\n\
                     \u{2022} Second entry\n\
                     </blockquote>";
        let rendered = render_activity_log_for_discord(html)
            .unwrap_or_else(|| panic!("expected rendered activity log"));
        assert_eq!(
            rendered.description,
            "\u{2022} First entry\n\u{2022} Second entry"
        );
        assert!(!rendered.has_errors);
    }

    #[test]
    fn render_activity_log_detects_error_entries() {
        let html = "<blockquote expandable>\n\
                     \u{1f4cb} <b>Activity log</b>\n\
                     \u{2022} \u{274C} exit 1 - command failed\n\
                     </blockquote>";
        let rendered = render_activity_log_for_discord(html)
            .unwrap_or_else(|| panic!("expected rendered activity log"));
        assert!(rendered.has_errors);
    }

    #[test]
    fn render_activity_log_truncates_long_descriptions() {
        let long = "x".repeat(DISCORD_MAX_EMBED_DESCRIPTION_LEN + 200);
        let html = format!(
            "<blockquote expandable>\n\
             \u{1f4cb} <b>Activity log</b>\n\
             \u{2022} {long}\n\
             </blockquote>"
        );
        let rendered = render_activity_log_for_discord(&html)
            .unwrap_or_else(|| panic!("expected rendered activity log"));
        assert!(rendered.description.len() <= DISCORD_MAX_EMBED_DESCRIPTION_LEN);
        assert!(rendered.description.ends_with("\n..."));
    }

    #[test]
    fn decode_data_url_png() {
        // Tiny 1-byte payload for test
        let b64 = base64::engine::general_purpose::STANDARD.encode([0xAB]);
        let url = format!("data:image/png;base64,{b64}");
        let bytes = decode_data_url(&url).unwrap_or_else(|e| panic!("decode failed: {e}"));
        assert_eq!(bytes, vec![0xAB]);
    }

    #[test]
    fn decode_data_url_invalid() {
        assert!(decode_data_url("data:image/png;base64").is_err());
    }

    #[test]
    fn extension_for_common_mimes() {
        assert_eq!(extension_for_mime("image/png"), "png");
        assert_eq!(extension_for_mime("image/jpeg"), "jpg");
        assert_eq!(extension_for_mime("image/x-portable-pixmap"), "ppm");
        assert_eq!(extension_for_mime("audio/ogg"), "ogg");
        assert_eq!(extension_for_mime("application/octet-stream"), "bin");
    }

    #[test]
    fn build_upload_preview_skips_non_images() {
        let data = vec![0_u8; DISCORD_IMAGE_PREVIEW_TRIGGER_BYTES + 10];
        assert!(build_upload_preview(&data, "application/pdf").is_none());
    }

    #[test]
    fn build_upload_preview_skips_small_images() {
        let b64 = base64::engine::general_purpose::STANDARD.encode([0x89, b'P', b'N', b'G']);
        let url = format!("data:image/png;base64,{b64}");
        let bytes = decode_data_url(&url).unwrap_or_else(|e| panic!("decode failed: {e}"));
        assert!(build_upload_preview(&bytes, "image/png").is_none());
    }

    #[test]
    fn build_upload_preview_resizes_large_png() {
        let width = 2400_u32;
        let height = 1400_u32;
        let image = image::ImageBuffer::from_fn(width, height, |x, y| {
            image::Rgb([
                (x.wrapping_mul(31) as u8).wrapping_add(y as u8),
                (y.wrapping_mul(17) as u8).wrapping_add(x as u8),
                (x.wrapping_mul(7) as u8) ^ (y.wrapping_mul(13) as u8),
            ])
        });
        let dynamic = image::DynamicImage::ImageRgb8(image);
        let mut png = Vec::new();
        dynamic
            .write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap_or_else(|e| panic!("failed to encode png fixture: {e}"));

        assert!(
            png.len() > DISCORD_IMAGE_PREVIEW_TRIGGER_BYTES,
            "fixture must be large enough to trigger preview generation (size={})",
            png.len()
        );

        let preview = build_upload_preview(&png, "image/png")
            .unwrap_or_else(|| panic!("expected preview to be generated"));
        assert_eq!(preview.1, "image/jpeg");
        assert!(preview.0.len() < png.len());
    }

    #[test]
    fn truncate_ascii() {
        assert_eq!(truncate_at_char_boundary("hello world", 5), "hello");
        assert_eq!(truncate_at_char_boundary("hi", 10), "hi");
    }

    #[test]
    fn truncate_multibyte() {
        // Each emoji is 4 bytes; truncating at byte 5 should back up to byte 4
        let s = "\u{1f600}\u{1f600}"; // 8 bytes
        let t = truncate_at_char_boundary(s, 5);
        assert_eq!(t, "\u{1f600}");
    }
}
