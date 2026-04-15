use teloxide::{prelude::*, types::ParseMode};

use moltis_channels::{
    ChannelEventSink, ChannelType,
    otp::{approve_sender_via_otp, emit_otp_challenge, emit_otp_resolution},
};

#[cfg(feature = "metrics")]
use moltis_metrics::{counter, telegram as tg_metrics};

use crate::{
    otp::{OtpInitResult, OtpVerifyResult},
    state::AccountStateMap,
};

pub(super) const OTP_CHALLENGE_MSG: &str = "To use this bot, please enter the verification code.\n\nAsk the bot owner for the code \u{2014} it is visible in the web UI under <b>Channels \u{2192} Senders</b>.\n\nThe code expires in 5 minutes.";

#[allow(clippy::too_many_arguments)]
pub(super) async fn handle_otp_flow(
    accounts: &AccountStateMap,
    account_id: &str,
    peer_id: &str,
    username: Option<&str>,
    sender_name: Option<&str>,
    text: Option<&str>,
    msg: &Message,
    event_sink: Option<&dyn ChannelEventSink>,
) {
    let chat_id = msg.chat.id;

    let bot = {
        let accts = accounts.read().unwrap_or_else(|e| e.into_inner());
        accts.get(account_id).map(|s| s.bot.clone())
    };
    let bot = match bot {
        Some(b) => b,
        None => return,
    };

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
        let body = text.unwrap_or("").trim();
        let is_code = body.len() == 6 && body.chars().all(|c| c.is_ascii_digit());

        if !is_code {
            return;
        }

        let result = {
            let accts = accounts.read().unwrap_or_else(|e| e.into_inner());
            match accts.get(account_id) {
                Some(s) => {
                    let mut otp = s.otp.lock().unwrap_or_else(|e| e.into_inner());
                    otp.verify(peer_id, body)
                },
                None => return,
            }
        };

        match result {
            OtpVerifyResult::Approved => {
                let identifier = username.unwrap_or(peer_id);
                approve_sender_via_otp(
                    event_sink,
                    ChannelType::Telegram,
                    account_id,
                    identifier,
                    peer_id,
                    username,
                )
                .await;

                let _ = bot
                    .send_message(chat_id, "Verified! You now have access to this bot.")
                    .await;

                #[cfg(feature = "metrics")]
                counter!(tg_metrics::OTP_VERIFICATIONS_TOTAL, "result" => "approved").increment(1);
            },
            OtpVerifyResult::WrongCode { attempts_left } => {
                let _ = bot
                    .send_message(
                        chat_id,
                        format!(
                            "Incorrect code. {attempts_left} attempt{} remaining.",
                            if attempts_left == 1 {
                                ""
                            } else {
                                "s"
                            }
                        ),
                    )
                    .await;

                #[cfg(feature = "metrics")]
                counter!(tg_metrics::OTP_VERIFICATIONS_TOTAL, "result" => "wrong_code")
                    .increment(1);
            },
            OtpVerifyResult::LockedOut => {
                let _ = bot
                    .send_message(chat_id, "Too many failed attempts. Please try again later.")
                    .await;

                emit_otp_resolution(
                    event_sink,
                    ChannelType::Telegram,
                    account_id,
                    peer_id,
                    username,
                    "locked_out",
                )
                .await;

                #[cfg(feature = "metrics")]
                counter!(tg_metrics::OTP_VERIFICATIONS_TOTAL, "result" => "locked_out")
                    .increment(1);
            },
            OtpVerifyResult::Expired => {
                let _ = bot
                    .send_message(
                        chat_id,
                        "Your code has expired. Send any message to get a new one.",
                    )
                    .await;

                emit_otp_resolution(
                    event_sink,
                    ChannelType::Telegram,
                    account_id,
                    peer_id,
                    username,
                    "expired",
                )
                .await;

                #[cfg(feature = "metrics")]
                counter!(tg_metrics::OTP_VERIFICATIONS_TOTAL, "result" => "expired").increment(1);
            },
            OtpVerifyResult::NoPending => {},
        }
    } else {
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
                let _ = bot
                    .send_message(chat_id, OTP_CHALLENGE_MSG)
                    .parse_mode(ParseMode::Html)
                    .await;

                let expires_at = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs() as i64
                    + 300;

                emit_otp_challenge(
                    event_sink,
                    ChannelType::Telegram,
                    account_id,
                    peer_id,
                    username,
                    sender_name,
                    code,
                    expires_at,
                )
                .await;

                #[cfg(feature = "metrics")]
                counter!(tg_metrics::OTP_CHALLENGES_TOTAL).increment(1);
            },
            OtpInitResult::AlreadyPending | OtpInitResult::LockedOut => {},
        }
    }
}
