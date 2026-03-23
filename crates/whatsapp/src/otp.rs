//! Re-export OTP module from moltis-channels.
//!
//! The OTP state machine is platform-agnostic and shared across channel plugins.
//! WhatsApp-specific constants (like the challenge message) live here.
pub use moltis_channels::otp::*;

/// Message sent to the WhatsApp user when an OTP challenge is created.
/// The code is NOT included — it is only visible to the admin in the web UI.
pub const OTP_CHALLENGE_MSG: &str = "To use this bot, please enter the verification code.\n\nAsk the bot owner for the code \u{2014} it is visible in the web UI under Channels \u{2192} Senders.\n\nThe code expires in 5 minutes.";

#[cfg(test)]
mod tests {
    use super::*;

    /// Security: the OTP challenge message must NEVER contain the code.
    #[test]
    fn security_otp_challenge_message_does_not_contain_code() {
        let has_six_digits = OTP_CHALLENGE_MSG
            .as_bytes()
            .windows(6)
            .any(|w| w.iter().all(|b| b.is_ascii_digit()));
        assert!(
            !has_six_digits,
            "OTP challenge message must not contain 6-digit code"
        );
    }
}
