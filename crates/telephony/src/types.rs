//! Core telephony types: call state machine, events, and records.

use {
    serde::{Deserialize, Serialize},
    time::OffsetDateTime,
};

/// Unique identifier for an internal call.
pub type CallId = String;

/// How a call should behave once connected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CallMode {
    /// Deliver a message and hang up after a short delay. One-way notification.
    Notify,
    /// Full multi-turn agent conversation until explicit end or timeout.
    #[default]
    Conversation,
}

/// Call lifecycle states.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CallState {
    /// Call has been placed but not yet ringing.
    Initiated,
    /// Remote phone is ringing.
    Ringing,
    /// Call has been answered / connected.
    Answered,
    /// Active conversation in progress.
    Active,
    /// Bot is currently speaking (TTS playback).
    Speaking,
    /// Bot is listening for user speech (STT active).
    Listening,
    // ── Terminal states ──
    /// Call ended normally.
    Completed,
    /// User hung up.
    HangupUser,
    /// Bot hung up.
    HangupBot,
    /// Max duration exceeded.
    Timeout,
    /// An error terminated the call.
    Error,
    /// No answer from the remote party.
    NoAnswer,
    /// Remote party was busy.
    Busy,
}

impl CallState {
    /// Whether this state is terminal (call is over).
    #[must_use]
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Completed
                | Self::HangupUser
                | Self::HangupBot
                | Self::Timeout
                | Self::Error
                | Self::NoAnswer
                | Self::Busy
        )
    }
}

/// Provider-agnostic call event emitted by webhook parsing.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CallEvent {
    /// Call was placed.
    Initiated { provider_call_id: String },
    /// Remote phone is ringing.
    Ringing { provider_call_id: String },
    /// Call was answered.
    Answered { provider_call_id: String },
    /// Bot started speaking.
    Speaking { provider_call_id: String },
    /// User speech recognized.
    Speech {
        provider_call_id: String,
        text: String,
        confidence: Option<f32>,
    },
    /// Silence detected on the line.
    Silence { provider_call_id: String },
    /// DTMF digit(s) received.
    Dtmf {
        provider_call_id: String,
        digits: String,
    },
    /// Call ended.
    Ended {
        provider_call_id: String,
        reason: CallEndReason,
    },
    /// Provider-level error.
    Error {
        provider_call_id: String,
        message: String,
    },
}

impl CallEvent {
    /// The provider-specific call ID from this event.
    #[must_use]
    pub fn provider_call_id(&self) -> &str {
        match self {
            Self::Initiated { provider_call_id }
            | Self::Ringing { provider_call_id }
            | Self::Answered { provider_call_id }
            | Self::Speaking { provider_call_id }
            | Self::Speech {
                provider_call_id, ..
            }
            | Self::Silence { provider_call_id }
            | Self::Dtmf {
                provider_call_id, ..
            }
            | Self::Ended {
                provider_call_id, ..
            }
            | Self::Error {
                provider_call_id, ..
            } => provider_call_id,
        }
    }
}

/// Why a call ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CallEndReason {
    Completed,
    HangupUser,
    HangupBot,
    Timeout,
    Error,
    NoAnswer,
    Busy,
}

impl CallEndReason {
    /// Map to the corresponding terminal `CallState`.
    #[must_use]
    pub fn to_state(self) -> CallState {
        match self {
            Self::Completed => CallState::Completed,
            Self::HangupUser => CallState::HangupUser,
            Self::HangupBot => CallState::HangupBot,
            Self::Timeout => CallState::Timeout,
            Self::Error => CallState::Error,
            Self::NoAnswer => CallState::NoAnswer,
            Self::Busy => CallState::Busy,
        }
    }
}

/// Direction of the call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CallDirection {
    Inbound,
    Outbound,
}

/// A single transcript entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptEntry {
    pub speaker: Speaker,
    pub text: String,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

/// Who spoke a transcript entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Speaker {
    Bot,
    User,
}

/// Persistent record of a single call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallRecord {
    pub call_id: CallId,
    pub provider_call_id: Option<String>,
    pub direction: CallDirection,
    pub from: String,
    pub to: String,
    pub mode: CallMode,
    pub state: CallState,
    /// Initial message to speak when the call connects (outbound only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initial_message: Option<String>,
    pub session_key: Option<String>,
    pub account_id: String,
    pub transcript: Vec<TranscriptEntry>,
    #[serde(with = "time::serde::rfc3339")]
    pub started_at: OffsetDateTime,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "time::serde::rfc3339::option"
    )]
    pub ended_at: Option<OffsetDateTime>,
    pub end_reason: Option<CallEndReason>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_states_are_correct() {
        assert!(!CallState::Initiated.is_terminal());
        assert!(!CallState::Ringing.is_terminal());
        assert!(!CallState::Answered.is_terminal());
        assert!(!CallState::Active.is_terminal());
        assert!(!CallState::Speaking.is_terminal());
        assert!(!CallState::Listening.is_terminal());
        assert!(CallState::Completed.is_terminal());
        assert!(CallState::HangupUser.is_terminal());
        assert!(CallState::HangupBot.is_terminal());
        assert!(CallState::Timeout.is_terminal());
        assert!(CallState::Error.is_terminal());
        assert!(CallState::NoAnswer.is_terminal());
        assert!(CallState::Busy.is_terminal());
    }

    #[test]
    fn end_reason_maps_to_state() {
        assert_eq!(CallEndReason::Completed.to_state(), CallState::Completed);
        assert_eq!(CallEndReason::HangupUser.to_state(), CallState::HangupUser);
        assert_eq!(CallEndReason::Busy.to_state(), CallState::Busy);
    }

    #[test]
    fn call_event_provider_id_extraction() {
        let evt = CallEvent::Speech {
            provider_call_id: "CA123".into(),
            text: "hello".into(),
            confidence: Some(0.95),
        };
        assert_eq!(evt.provider_call_id(), "CA123");
    }

    #[test]
    fn call_mode_default_is_conversation() {
        assert_eq!(CallMode::default(), CallMode::Conversation);
    }

    #[test]
    fn call_state_roundtrip_serde() {
        let json = serde_json::to_string(&CallState::HangupUser).unwrap_or_default();
        assert_eq!(json, "\"hangup_user\"");
        let parsed: CallState = serde_json::from_str(&json).unwrap_or(CallState::Error);
        assert_eq!(parsed, CallState::HangupUser);
    }

    #[test]
    fn call_record_initial_message_serde() {
        let record = CallRecord {
            call_id: "test".into(),
            provider_call_id: None,
            direction: CallDirection::Outbound,
            from: "+1".into(),
            to: "+2".into(),
            mode: CallMode::Notify,
            state: CallState::Initiated,
            initial_message: Some("Hello there".into()),
            session_key: None,
            account_id: "acct".into(),
            transcript: vec![],
            started_at: OffsetDateTime::now_utc(),
            ended_at: None,
            end_reason: None,
        };
        let json = serde_json::to_value(&record).unwrap_or_default();
        assert_eq!(json["initial_message"], "Hello there");
        assert_eq!(json["mode"], "notify");
    }
}
