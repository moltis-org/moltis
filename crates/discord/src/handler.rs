use {
    serenity::{
        all::{
            Context, CreateMessage, EventHandler, GatewayIntents, Interaction, Message, MessageId,
            ReactionType, Ready,
        },
        async_trait,
        gateway::ActivityData,
        model::user::OnlineStatus as SerenityOnlineStatus,
    },
    tracing::{debug, info, warn},
};

use crate::config::{
    ActivityType as CfgActivityType, DiscordAccountConfig, OnlineStatus as CfgOnlineStatus,
};

use crate::access;

use moltis_channels::{
    ChannelEvent, ChannelType,
    gating::DmPolicy,
    message_log::MessageLogEntry,
    otp::{OtpInitResult, OtpVerifyResult},
    plugin::{ChannelEventSink, ChannelMessageKind, ChannelMessageMeta, ChannelReplyTarget},
};

use crate::state::AccountStateMap;

/// Required gateway intents for the Discord bot.
pub fn required_intents() -> GatewayIntents {
    GatewayIntents::GUILD_MESSAGES
        | GatewayIntents::DIRECT_MESSAGES
        | GatewayIntents::MESSAGE_CONTENT
        | GatewayIntents::GUILDS
        | GatewayIntents::GUILD_MESSAGE_REACTIONS
        | GatewayIntents::DIRECT_MESSAGE_REACTIONS
}

/// Serenity event handler for a Discord bot account.
pub struct Handler {
    pub account_id: String,
    pub accounts: AccountStateMap,
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

fn unix_now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Discord snowflake epoch (2015-01-01T00:00:00.000Z) in Unix milliseconds.
const DISCORD_EPOCH_MS: u64 = 1_420_070_400_000;

fn discord_message_created_ms(message_id: MessageId) -> u64 {
    (message_id.get() >> 22).saturating_add(DISCORD_EPOCH_MS)
}

fn is_valid_lat_lon(latitude: f64, longitude: f64) -> bool {
    (-90.0..=90.0).contains(&latitude) && (-180.0..=180.0).contains(&longitude)
}

fn parse_coordinate_component(input: &str) -> Option<f64> {
    let trimmed = input
        .trim()
        .trim_matches(|c| matches!(c, '(' | ')' | '[' | ']' | '{' | '}'));
    if trimmed.is_empty() {
        return None;
    }

    let mut end = 0usize;
    for (idx, ch) in trimmed.char_indices() {
        if ch.is_ascii_digit() || matches!(ch, '+' | '-' | '.') {
            end = idx + ch.len_utf8();
        } else {
            break;
        }
    }
    if end == 0 {
        return None;
    }
    let token = &trimmed[..end];
    if !token.chars().any(|c| c.is_ascii_digit()) {
        return None;
    }
    token.parse::<f64>().ok()
}

fn parse_coordinate_pair(input: &str) -> Option<(f64, f64)> {
    let mut parts = input.split(',');
    let latitude = parse_coordinate_component(parts.next()?)?;
    let longitude = parse_coordinate_component(parts.next()?)?;
    if is_valid_lat_lon(latitude, longitude) {
        Some((latitude, longitude))
    } else {
        None
    }
}

fn parse_coordinates_from_url(url_str: &str) -> Option<(f64, f64)> {
    let parsed = reqwest::Url::parse(url_str).ok()?;

    for key in ["ll", "q", "query"] {
        if let Some((_, value)) = parsed
            .query_pairs()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            && let Some(coords) = parse_coordinate_pair(value.as_ref())
        {
            return Some(coords);
        }
    }

    for segment in [
        parsed.path(),
        parsed.fragment().unwrap_or_default(),
        url_str,
    ] {
        if let Some(at_pos) = segment.find('@')
            && let Some(coords) = parse_coordinate_pair(&segment[at_pos + 1..])
        {
            return Some(coords);
        }
    }

    None
}

fn parse_map_link_coordinates(text: &str) -> Option<(f64, f64)> {
    for raw in text.split_whitespace() {
        let token = raw.trim_matches(|c: char| {
            matches!(
                c,
                '<' | '>' | '(' | ')' | '[' | ']' | '{' | '}' | ',' | '.' | '!' | '?'
            )
        });
        if !(token.starts_with("http://") || token.starts_with("https://")) {
            continue;
        }
        if let Some(coords) = parse_coordinates_from_url(token) {
            return Some(coords);
        }
    }
    None
}

fn parse_plain_text_coordinates(text: &str) -> Option<(f64, f64)> {
    let trimmed = text.trim();
    if trimmed.is_empty() || !trimmed.contains(',') {
        return None;
    }
    if !trimmed
        .chars()
        .all(|c| c.is_ascii_digit() || matches!(c, '+' | '-' | '.' | ',' | ' ' | '\t' | '(' | ')'))
    {
        return None;
    }
    parse_coordinate_pair(trimmed)
}

fn extract_location_coordinates(text: &str) -> Option<(f64, f64)> {
    parse_map_link_coordinates(text).or_else(|| parse_plain_text_coordinates(text))
}

/// Strip the bot mention (e.g. `<@123456789>`) from the beginning of a message.
pub fn strip_bot_mention(text: &str, bot_id: u64) -> String {
    let mention = format!("<@{bot_id}>");
    let mention_nick = format!("<@!{bot_id}>");
    let stripped = text
        .trim()
        .strip_prefix(&mention)
        .or_else(|| text.trim().strip_prefix(&mention_nick))
        .unwrap_or(text);
    stripped.trim().to_string()
}

/// Set the bot's presence (activity + online status) from config.
fn set_bot_presence(ctx: &Context, account_id: &str, config: &DiscordAccountConfig) {
    let activity = config.activity.as_deref().map(|text| {
        let activity_type = config.activity_type.unwrap_or_default();
        match activity_type {
            CfgActivityType::Playing => ActivityData::playing(text),
            CfgActivityType::Listening => ActivityData::listening(text),
            CfgActivityType::Watching => ActivityData::watching(text),
            CfgActivityType::Competing => ActivityData::competing(text),
            CfgActivityType::Custom => ActivityData::custom(text),
        }
    });

    let online_status = match config.status {
        Some(CfgOnlineStatus::Online) | None => SerenityOnlineStatus::Online,
        Some(CfgOnlineStatus::Idle) => SerenityOnlineStatus::Idle,
        Some(CfgOnlineStatus::Dnd) => SerenityOnlineStatus::DoNotDisturb,
        Some(CfgOnlineStatus::Invisible) => SerenityOnlineStatus::Invisible,
    };

    // Only set presence if there's something to configure.
    if activity.is_some() || config.status.is_some() {
        ctx.set_presence(activity, online_status);
        info!(
            account_id,
            activity_text = ?config.activity,
            activity_type = ?config.activity_type,
            status = ?config.status,
            "Discord bot presence set"
        );
    }
}

#[async_trait]
impl EventHandler for Handler {
    async fn message(&self, ctx: Context, msg: Message) {
        // Ignore messages from bots (including ourselves).
        if msg.author.bot {
            return;
        }

        let accounts_lock_wait_start = std::time::Instant::now();
        let (config, event_sink, message_log, bot_user_id) = {
            let accounts = self.accounts.read().unwrap_or_else(|e| e.into_inner());
            let Some(state) = accounts.get(&self.account_id) else {
                warn!(account_id = %self.account_id, "Discord handler: unknown account");
                return;
            };
            (
                state.config.clone(),
                state.event_sink.clone(),
                state.message_log.clone(),
                state.bot_user_id,
            )
        };
        let accounts_lock_wait_ms = accounts_lock_wait_start.elapsed().as_millis() as u64;

        let is_guild = msg.guild_id.is_some();
        let message_id = msg.id.get();
        let peer_id = msg.author.id.to_string();
        let username = Some(msg.author.name.clone());
        let sender_name = msg.author.global_name.clone().or_else(|| username.clone());
        let chat_id = msg.channel_id.to_string();

        // Check if the bot is mentioned in a guild message.
        let bot_mentioned =
            bot_user_id.is_some_and(|bot_id| msg.mentions.iter().any(|u| u.id == bot_id));

        // Extract and clean message text.
        let text = if let Some(bot_id) = bot_user_id
            && bot_mentioned
        {
            strip_bot_mention(&msg.content, bot_id.get())
        } else {
            msg.content.clone()
        };

        if text.is_empty() {
            return;
        }

        let created_ms = discord_message_created_ms(msg.id);
        let ingress_lag_ms = unix_now_ms().saturating_sub(created_ms);

        info!(
            account_id = %self.account_id,
            message_id,
            chat_id,
            peer_id,
            username = ?username,
            sender_name = ?sender_name,
            is_guild,
            bot_mentioned,
            text_len = text.len(),
            ingress_lag_ms,
            accounts_lock_wait_ms,
            "discord inbound message received"
        );
        if ingress_lag_ms > 2_000 {
            warn!(
                account_id = %self.account_id,
                message_id,
                chat_id,
                peer_id,
                ingress_lag_ms,
                "discord inbound delivery lag exceeds 2s"
            );
        }

        // Check DM / guild / mention policy.
        let chat_type = if is_guild {
            moltis_common::types::ChatType::Group
        } else {
            moltis_common::types::ChatType::Dm
        };
        let guild_id_str = msg.guild_id.map(|g| g.to_string());
        let policy_allowed = access::check_access(
            &config,
            &chat_type,
            &peer_id,
            username.as_deref(),
            guild_id_str.as_deref(),
            bot_mentioned,
        )
        .is_ok();
        let access_granted = policy_allowed;

        // Log the message.
        if let Some(log) = message_log {
            let _ = log
                .log(MessageLogEntry {
                    id: 0,
                    account_id: self.account_id.clone(),
                    channel_type: "discord".into(),
                    peer_id: peer_id.clone(),
                    username: username.clone(),
                    sender_name: sender_name.clone(),
                    chat_id: chat_id.clone(),
                    chat_type: if is_guild {
                        "group".into()
                    } else {
                        "private".into()
                    },
                    body: text.clone(),
                    access_granted,
                    created_at: unix_now(),
                })
                .await;
        }

        // Emit inbound message event.
        if let Some(sink) = event_sink.as_ref() {
            sink.emit(ChannelEvent::InboundMessage {
                channel_type: ChannelType::Discord,
                account_id: self.account_id.clone(),
                peer_id: peer_id.clone(),
                username: username.clone(),
                sender_name: sender_name.clone(),
                message_count: None,
                access_granted,
            })
            .await;
        }

        if !access_granted {
            // OTP self-approval for non-allowlisted DM users.
            if !is_guild
                && !policy_allowed
                && config.otp_self_approval
                && config.dm_policy == DmPolicy::Allowlist
            {
                handle_otp_flow(
                    &self.accounts,
                    &self.account_id,
                    &peer_id,
                    username.as_deref(),
                    sender_name.as_deref(),
                    &text,
                    msg.channel_id,
                    event_sink.as_deref(),
                    &ctx,
                )
                .await;
            }
            return;
        }

        // Add ack reaction to indicate the bot is processing.
        if let Some(ref emoji) = config.ack_reaction {
            let reaction = ReactionType::Unicode(emoji.clone());
            if let Err(e) = msg.react(&ctx, reaction).await {
                debug!(
                    account_id = %self.account_id,
                    emoji,
                    "failed to add ack reaction: {e}"
                );
            }
        }

        let reply_to = ChannelReplyTarget {
            channel_type: ChannelType::Discord,
            account_id: self.account_id.clone(),
            chat_id: chat_id.clone(),
            message_id: Some(msg.id.to_string()),
        };

        let Some(sink) = event_sink else {
            warn!(
                account_id = %self.account_id,
                "Discord inbound message ignored: no channel event sink"
            );
            return;
        };

        // Handle slash commands.
        if let Some(command) = text.strip_prefix('/') {
            let response_text = match sink
                .dispatch_command(command.trim(), reply_to.clone())
                .await
            {
                Ok(response) => response,
                Err(e) => format!("Command failed: {e}"),
            };
            let http = {
                let accounts = self.accounts.read().unwrap_or_else(|e| e.into_inner());
                accounts.get(&self.account_id).and_then(|s| s.http.clone())
            };
            if let Some(http) = http
                && let Err(e) = send_discord_text(&http, msg.channel_id, &response_text).await
            {
                warn!(
                    account_id = %self.account_id,
                    chat_id,
                    "failed to send Discord command response: {e}"
                );
            }
            return;
        }

        let mut inferred_kind = ChannelMessageKind::Text;
        if let Some((latitude, longitude)) = extract_location_coordinates(&text) {
            let resolved = sink
                .resolve_pending_location(&reply_to, latitude, longitude)
                .await;
            if resolved {
                info!(
                    account_id = %self.account_id,
                    chat_id,
                    peer_id,
                    latitude,
                    longitude,
                    "discord location input resolved pending request"
                );
                if let Err(e) =
                    send_discord_text_simple(&ctx, msg.channel_id, "Location updated.").await
                {
                    warn!(
                        account_id = %self.account_id,
                        chat_id,
                        "failed to send location confirmation: {e}"
                    );
                }
                return;
            }
            inferred_kind = ChannelMessageKind::Location;
        }

        // Dispatch to chat.
        info!(
            account_id = %self.account_id,
            chat_id,
            peer_id,
            text_len = text.len(),
            "discord dispatching to chat"
        );
        sink.dispatch_to_chat(&text, reply_to, ChannelMessageMeta {
            channel_type: ChannelType::Discord,
            sender_name,
            username,
            message_kind: Some(inferred_kind),
            model: config.model.clone(),
            audio_filename: None,
        })
        .await;
    }

    async fn ready(&self, ctx: Context, ready: Ready) {
        info!(
            account_id = %self.account_id,
            bot_user = %ready.user.name,
            "Discord bot connected as {}",
            ready.user.name,
        );

        let config = {
            let mut accounts = self.accounts.write().unwrap_or_else(|e| e.into_inner());
            if let Some(state) = accounts.get_mut(&self.account_id) {
                state.bot_user_id = Some(ready.user.id);
            }
            accounts.get(&self.account_id).map(|s| s.config.clone())
        };

        // Set bot presence/activity if configured.
        if let Some(config) = config {
            set_bot_presence(&ctx, &self.account_id, &config);
        }

        // Register slash commands.
        crate::commands::register_global_commands(&ctx, &self.account_id).await;
    }

    async fn interaction_create(&self, ctx: Context, interaction: Interaction) {
        crate::commands::handle_interaction(&ctx, &interaction, &self.account_id, &self.accounts)
            .await;
    }
}

/// OTP challenge message sent to non-allowlisted DM users.
///
/// SECURITY: This message must NEVER contain the OTP code. The code is only
/// visible to the admin in the web UI under Channels → Senders.
const OTP_CHALLENGE_MSG: &str = "To use this bot, please enter the verification code.\n\nAsk the bot owner for the code \u{2014} it is visible in the web UI under **Channels \u{2192} Senders**.\n\nThe code expires in 5 minutes.";

/// Check if a message body looks like a 6-digit OTP code.
fn looks_like_otp_code(text: &str) -> bool {
    text.len() == 6 && text.chars().all(|c| c.is_ascii_digit())
}

/// Handle OTP challenge/verification flow for a non-allowlisted DM user.
///
/// Called when `dm_policy = Allowlist`, the peer is not on the allowlist, and
/// `otp_self_approval` is enabled. Manages the full lifecycle:
/// - First message: issue a 6-digit OTP challenge
/// - Code reply: verify and auto-approve on match
/// - Non-code messages while pending: silently ignored (flood protection)
#[allow(clippy::too_many_arguments)]
async fn handle_otp_flow(
    accounts: &AccountStateMap,
    account_id: &str,
    peer_id: &str,
    username: Option<&str>,
    sender_name: Option<&str>,
    body: &str,
    channel_id: serenity::all::ChannelId,
    event_sink: Option<&dyn ChannelEventSink>,
    ctx: &Context,
) {
    let has_pending = {
        let accts = accounts.read().unwrap_or_else(|e| e.into_inner());
        accts
            .get(account_id)
            .map(|s| {
                let otp = s.otp.lock().unwrap_or_else(|e| e.into_inner());
                otp.has_pending(peer_id)
            })
            .unwrap_or(false)
    };

    if has_pending {
        // Only process messages that look like OTP codes (6 digits).
        let trimmed = body.trim();
        if !looks_like_otp_code(trimmed) {
            return; // Silently ignore non-code messages while pending.
        }

        // Verify the code.
        let result = {
            let accts = accounts.read().unwrap_or_else(|e| e.into_inner());
            match accts.get(account_id) {
                Some(s) => {
                    let mut otp = s.otp.lock().unwrap_or_else(|e| e.into_inner());
                    otp.verify(peer_id, trimmed)
                },
                None => return,
            }
        };

        match result {
            OtpVerifyResult::Approved => {
                // Auto-approve: add to allowlist via the event sink.
                let identifier = peer_id;
                if let Some(sink) = event_sink {
                    sink.request_sender_approval("discord", account_id, identifier)
                        .await;

                    sink.emit(ChannelEvent::OtpResolved {
                        channel_type: ChannelType::Discord,
                        account_id: account_id.to_string(),
                        peer_id: peer_id.to_string(),
                        username: username.map(String::from),
                        resolution: "approved".into(),
                    })
                    .await;
                }

                let _ = send_discord_text_simple(
                    ctx,
                    channel_id,
                    "Approved! You can now use this bot.",
                )
                .await;
            },
            OtpVerifyResult::WrongCode { attempts_left } => {
                let msg = format!(
                    "Incorrect code. {attempts_left} attempt{} remaining.",
                    if attempts_left == 1 {
                        ""
                    } else {
                        "s"
                    }
                );
                let _ = send_discord_text_simple(ctx, channel_id, &msg).await;
            },
            OtpVerifyResult::LockedOut => {
                let _ = send_discord_text_simple(
                    ctx,
                    channel_id,
                    "Too many failed attempts. Please try again later.",
                )
                .await;

                if let Some(sink) = event_sink {
                    sink.emit(ChannelEvent::OtpResolved {
                        channel_type: ChannelType::Discord,
                        account_id: account_id.to_string(),
                        peer_id: peer_id.to_string(),
                        username: username.map(String::from),
                        resolution: "locked_out".into(),
                    })
                    .await;
                }
            },
            OtpVerifyResult::Expired => {
                let _ = send_discord_text_simple(
                    ctx,
                    channel_id,
                    "Your code has expired. Send any message to get a new one.",
                )
                .await;

                if let Some(sink) = event_sink {
                    sink.emit(ChannelEvent::OtpResolved {
                        channel_type: ChannelType::Discord,
                        account_id: account_id.to_string(),
                        peer_id: peer_id.to_string(),
                        username: username.map(String::from),
                        resolution: "expired".into(),
                    })
                    .await;
                }
            },
            OtpVerifyResult::NoPending => {
                // Shouldn't happen since we checked has_pending, but handle gracefully.
            },
        }
    } else {
        // No pending challenge — initiate one.
        let init_result = {
            let accts = accounts.read().unwrap_or_else(|e| e.into_inner());
            match accts.get(account_id) {
                Some(s) => {
                    let mut otp = s.otp.lock().unwrap_or_else(|e| e.into_inner());
                    otp.initiate(
                        peer_id,
                        username.map(String::from),
                        sender_name.map(String::from),
                    )
                },
                None => return,
            }
        };

        match init_result {
            OtpInitResult::Created(code) => {
                let _ = send_discord_text_simple(ctx, channel_id, OTP_CHALLENGE_MSG).await;

                if let Some(sink) = event_sink {
                    let expires_at = unix_now() + 300; // 5 minutes
                    sink.emit(ChannelEvent::OtpChallenge {
                        channel_type: ChannelType::Discord,
                        account_id: account_id.to_string(),
                        peer_id: peer_id.to_string(),
                        username: username.map(String::from),
                        sender_name: sender_name.map(String::from),
                        code,
                        expires_at,
                    })
                    .await;
                }
            },
            OtpInitResult::AlreadyPending | OtpInitResult::LockedOut => {
                // Silent ignore.
            },
        }
    }
}

/// Simple send using the Context's http — used for OTP messages where we don't
/// have the full Http handle from state.
async fn send_discord_text_simple(
    ctx: &Context,
    channel_id: serenity::all::ChannelId,
    text: &str,
) -> Result<(), String> {
    let msg = CreateMessage::new().content(text);
    channel_id
        .send_message(&ctx, msg)
        .await
        .map_err(|e| format!("Discord send: {e}"))?;
    Ok(())
}

/// Send a text message to a Discord channel, chunking at the 2000-character limit.
pub async fn send_discord_text(
    http: &serenity::http::Http,
    channel_id: serenity::all::ChannelId,
    text: &str,
) -> Result<(), String> {
    send_discord_message(http, channel_id, text, None).await?;
    Ok(())
}

/// Send a text message and return the last sent `Message` (needed for
/// edit-in-place streaming).
///
/// When `reference` is `Some`, the first chunk is sent as a Discord reply
/// to that message (using `reference_message`).
pub async fn send_discord_message(
    http: &serenity::http::Http,
    channel_id: serenity::all::ChannelId,
    text: &str,
    reference: Option<MessageId>,
) -> Result<Message, String> {
    if text.is_empty() {
        return Err("empty message".into());
    }

    let chunks = chunk_message(text, 2000);
    let mut last_msg = None;
    for (i, chunk) in chunks.iter().enumerate() {
        let mut create = CreateMessage::new().content(*chunk);
        // Only the first chunk gets the reply reference.
        if i == 0
            && let Some(ref_id) = reference
        {
            create = create.reference_message((channel_id, ref_id));
        }
        last_msg = Some(
            channel_id
                .send_message(http, create)
                .await
                .map_err(|e| format!("Discord send: {e}"))?,
        );
    }
    // `last_msg` is always `Some` because `text` is non-empty.
    last_msg.ok_or_else(|| "no chunks produced".into())
}

/// Split a message into chunks of at most `max_len` characters.
///
/// The chunker is markdown-aware: it avoids splitting inside fenced code blocks
/// (triple-backtick regions) so that Discord renders them correctly.
fn chunk_message(text: &str, max_len: usize) -> Vec<&str> {
    if text.len() <= max_len {
        return vec![text];
    }
    let mut chunks = Vec::new();
    let mut remaining = text;
    while !remaining.is_empty() {
        if remaining.len() <= max_len {
            chunks.push(remaining);
            break;
        }
        let split_at = find_split_point(remaining, max_len);
        let (chunk, rest) = remaining.split_at(split_at);
        chunks.push(chunk);
        remaining = rest;
    }
    chunks
}

/// Find the best position to split `text` within `max_len` bytes.
///
/// Avoids splitting inside fenced code blocks. Prefers newlines outside of code
/// fences, falls back to `max_len` if no better boundary is found.
fn find_split_point(text: &str, max_len: usize) -> usize {
    let window = &text[..max_len];

    // Track whether each newline position is inside a fenced code block.
    let mut in_fence = false;
    let mut best_outside_fence = None;
    let mut best_any_newline = None;

    for (i, line) in window.split('\n').scan(0usize, |pos, line| {
        let start = *pos;
        *pos += line.len() + 1; // +1 for the '\n'
        Some((start, line))
    }) {
        let newline_pos = i + line.len(); // position of the '\n' itself
        if newline_pos >= max_len {
            break;
        }

        if line.trim_start().starts_with("```") {
            in_fence = !in_fence;
        }

        // Record the split position (right after the newline).
        let split = newline_pos + 1;
        best_any_newline = Some(split);
        if !in_fence {
            best_outside_fence = Some(split);
        }
    }

    // Prefer splitting outside a code fence; fall back to any newline; finally
    // fall back to the hard limit.
    best_outside_fence.or(best_any_newline).unwrap_or(max_len)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_mention_at_start() {
        assert_eq!(strip_bot_mention("<@123> hello world", 123), "hello world");
        assert_eq!(strip_bot_mention("<@!123> hello world", 123), "hello world");
    }

    #[test]
    fn strip_mention_no_match() {
        assert_eq!(
            strip_bot_mention("hello <@123> world", 123),
            "hello <@123> world"
        );
        assert_eq!(strip_bot_mention("hello world", 123), "hello world");
    }

    #[test]
    fn strip_mention_different_bot() {
        assert_eq!(strip_bot_mention("<@999> hello", 123), "<@999> hello");
    }

    #[test]
    fn extract_location_coordinates_from_plain_pair() {
        let coords = extract_location_coordinates("48.8566, 2.3522")
            .unwrap_or_else(|| panic!("expected coordinates"));
        assert!((coords.0 - 48.8566).abs() < 1e-6);
        assert!((coords.1 - 2.3522).abs() < 1e-6);
    }

    #[test]
    fn extract_location_coordinates_from_google_query() {
        let coords =
            extract_location_coordinates("https://www.google.com/maps?q=37.7749,-122.4194")
                .unwrap_or_else(|| panic!("expected coordinates"));
        assert!((coords.0 - 37.7749).abs() < 1e-6);
        assert!((coords.1 + 122.4194).abs() < 1e-6);
    }

    #[test]
    fn extract_location_coordinates_from_google_path_marker() {
        let coords = extract_location_coordinates(
            "https://www.google.com/maps/place/test/@48.8566,2.3522,14z/data=!3m1!4b1",
        )
        .unwrap_or_else(|| panic!("expected coordinates"));
        assert!((coords.0 - 48.8566).abs() < 1e-6);
        assert!((coords.1 - 2.3522).abs() < 1e-6);
    }

    #[test]
    fn extract_location_coordinates_from_apple_maps() {
        let coords =
            extract_location_coordinates("https://maps.apple.com/?ll=34.0522,-118.2437&z=12")
                .unwrap_or_else(|| panic!("expected coordinates"));
        assert!((coords.0 - 34.0522).abs() < 1e-6);
        assert!((coords.1 + 118.2437).abs() < 1e-6);
    }

    #[test]
    fn extract_location_coordinates_rejects_non_location_text() {
        assert!(extract_location_coordinates("hey what's up?").is_none());
        assert!(extract_location_coordinates("my score is 1,2 today").is_none());
    }

    #[test]
    fn chunk_short_message() {
        let chunks = chunk_message("hello", 2000);
        assert_eq!(chunks, vec!["hello"]);
    }

    #[test]
    fn chunk_long_message() {
        let text = "a".repeat(4500);
        let chunks = chunk_message(&text, 2000);
        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[0].len(), 2000);
        assert_eq!(chunks[1].len(), 2000);
        assert_eq!(chunks[2].len(), 500);
    }

    #[test]
    fn chunk_splits_at_newline() {
        let mut text = String::new();
        text.push_str(&"a".repeat(1500));
        text.push('\n');
        text.push_str(&"b".repeat(1000));
        let chunks = chunk_message(&text, 2000);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].len(), 1501); // 1500 + newline
        assert_eq!(chunks[1].len(), 1000);
    }

    #[test]
    fn chunk_avoids_splitting_inside_code_fence() {
        // The code fence fits within max_len, so the split should land before
        // or after the fence — not inside it.
        let mut text = String::new();
        text.push_str(&"a".repeat(80));
        text.push('\n');
        text.push_str("```\n");
        text.push_str("code line 1\ncode line 2\n");
        text.push_str("```\n");
        text.push_str(&"b".repeat(80));
        // max_len = 120: the 80+newline prefix is 81 chars, which fits.
        // The code fence block is ~30 chars. A naive newline split at ~120
        // would land inside the fence. The markdown-aware splitter should
        // split at the newline before the fence (position 81).
        let chunks = chunk_message(&text, 120);
        for chunk in &chunks {
            let opens = chunk.matches("```").count();
            assert_eq!(opens % 2, 0, "unbalanced code fence in chunk: {chunk:?}");
        }
    }

    /// Security: the OTP challenge message sent to the Discord user must
    /// NEVER contain the verification code. The code should only be visible
    /// to the admin in the web UI. If this test fails, unauthenticated users
    /// can self-approve without admin involvement.
    #[test]
    fn security_otp_challenge_message_does_not_contain_code() {
        let msg = OTP_CHALLENGE_MSG;

        // Must not contain any 6-digit numeric sequences (OTP codes are 6 digits).
        let has_six_digits = msg
            .as_bytes()
            .windows(6)
            .any(|w| w.iter().all(|b| b.is_ascii_digit()));
        assert!(
            !has_six_digits,
            "SECURITY: OTP_CHALLENGE_MSG must not contain numeric codes"
        );
    }

    #[test]
    fn chunk_code_fence_too_large_falls_back() {
        // When the code fence itself exceeds max_len, we must still split
        // (graceful degradation — can't avoid unbalanced fences here).
        let mut text = String::from("```\n");
        text.push_str(&"x".repeat(300));
        text.push_str("\n```\n");
        let chunks = chunk_message(&text, 100);
        assert!(chunks.len() >= 2, "should split oversized code fence");
        let reassembled: String = chunks.iter().copied().collect();
        assert_eq!(reassembled, text);
    }

    // ── OTP code detection tests ─────────────────────────────────────

    #[test]
    fn looks_like_otp_code_valid() {
        assert!(looks_like_otp_code("123456"));
        assert!(looks_like_otp_code("000000"));
        assert!(looks_like_otp_code("999999"));
    }

    #[test]
    fn looks_like_otp_code_rejects_non_codes() {
        assert!(!looks_like_otp_code("hello"));
        assert!(!looks_like_otp_code("12345")); // too short
        assert!(!looks_like_otp_code("1234567")); // too long
        assert!(!looks_like_otp_code("12345a")); // not all digits
        assert!(!looks_like_otp_code("")); // empty
        assert!(!looks_like_otp_code("abcdef")); // no digits
        assert!(!looks_like_otp_code("12 345")); // space
    }

    #[test]
    fn looks_like_otp_code_rejects_unicode_digits() {
        // Arabic-Indic digits (U+0660..U+0669) should not be accepted.
        assert!(!looks_like_otp_code(
            "\u{0660}\u{0661}\u{0662}\u{0663}\u{0664}\u{0665}"
        ));
    }

    // ── OTP message security tests ───────────────────────────────────

    #[test]
    fn security_otp_message_has_no_format_placeholders() {
        let msg = OTP_CHALLENGE_MSG;
        assert!(
            !msg.contains("{code}") && !msg.contains("{0}") && !msg.contains("%s"),
            "OTP challenge message must not contain format placeholders"
        );
    }

    #[test]
    fn security_otp_message_points_to_web_ui() {
        let msg = OTP_CHALLENGE_MSG;
        assert!(
            msg.contains("Channels") && msg.contains("Senders"),
            "OTP message must tell user where to find the code"
        );
    }

    #[test]
    fn otp_message_uses_discord_markdown_not_html() {
        let msg = OTP_CHALLENGE_MSG;
        // Discord uses ** for bold, not <b>.
        assert!(
            !msg.contains("<b>") && !msg.contains("<i>"),
            "OTP message should use Discord markdown, not HTML tags"
        );
        // Should use ** or nothing, but never HTML.
        assert!(
            !msg.contains("</"),
            "OTP message contains HTML closing tags"
        );
    }

    #[test]
    fn otp_message_mentions_expiry() {
        let msg = OTP_CHALLENGE_MSG;
        assert!(
            msg.contains("5 minutes") || msg.contains("expires"),
            "OTP message should mention the expiry time"
        );
    }

    // ── Presence config mapping tests ────────────────────────────────

    #[test]
    fn required_intents_includes_reactions() {
        let intents = required_intents();
        assert!(
            intents.contains(GatewayIntents::GUILD_MESSAGE_REACTIONS),
            "must include GUILD_MESSAGE_REACTIONS"
        );
        assert!(
            intents.contains(GatewayIntents::DIRECT_MESSAGE_REACTIONS),
            "must include DIRECT_MESSAGE_REACTIONS"
        );
    }

    #[test]
    fn required_intents_includes_message_content() {
        let intents = required_intents();
        assert!(
            intents.contains(GatewayIntents::MESSAGE_CONTENT),
            "must include MESSAGE_CONTENT for reading message text"
        );
        assert!(
            intents.contains(GatewayIntents::GUILD_MESSAGES),
            "must include GUILD_MESSAGES"
        );
        assert!(
            intents.contains(GatewayIntents::DIRECT_MESSAGES),
            "must include DIRECT_MESSAGES"
        );
        assert!(
            intents.contains(GatewayIntents::GUILDS),
            "must include GUILDS"
        );
    }

    #[test]
    fn strip_mention_with_leading_whitespace() {
        assert_eq!(strip_bot_mention("  <@123> hello", 123), "hello");
    }

    #[test]
    fn strip_mention_only_mention() {
        // When the message is just the mention, result should be empty after trim.
        assert_eq!(strip_bot_mention("<@123>", 123), "");
    }

    #[test]
    fn discord_message_created_ms_from_snowflake() {
        // Example snowflake from Discord docs / Serenity tests.
        let id = MessageId::new(175_928_847_299_117_063);
        assert_eq!(discord_message_created_ms(id), 1_462_015_105_796);
    }
}
