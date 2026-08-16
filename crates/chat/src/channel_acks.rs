//! Channel acknowledgment-reaction plumbing for chat runs.
//!
//! An inbound channel message carries an acknowledgment identity (its "ack
//! key") so its reaction follows the message itself through queueing, replay
//! and collection, rather than following whatever else shares the session.

use std::sync::Arc;

use serde_json::Value;

use crate::runtime::ChatRuntime;

/// Params key carrying the acknowledgment identities a call is responsible for.
pub(crate) use crate::params::ACK_KEYS as ACK_KEYS_PARAM;

/// Read the acknowledgment identities from `chat.send` params.
pub(crate) fn ack_keys_from_params(params: &Value) -> Vec<String> {
    params
        .get(ACK_KEYS_PARAM)
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

/// Union the acknowledgment identities across several queued messages.
///
/// In `collect` queue mode one run answers all of them, so it owns every one of
/// their acknowledgments and each message receives the terminal reaction.
pub(crate) fn merged_ack_keys<'a>(params: impl Iterator<Item = &'a Value>) -> Vec<String> {
    let mut keys = Vec::new();
    for p in params {
        for key in ack_keys_from_params(p) {
            if !keys.contains(&key) {
                keys.push(key);
            }
        }
    }
    keys
}

/// Mark the channel acknowledgment as failed because the reply never reached
/// the user. Terminal protection means this wins over the run's later Success.
pub(crate) async fn note_delivery_failed(state: &Arc<dyn ChatRuntime>, activity_id: &str) {
    state
        .note_channel_activity(
            activity_id,
            moltis_channels::ChannelActivity::Finished(moltis_channels::ChannelAckOutcome::Failure),
        )
        .await;
}

/// Finalize the acknowledgment for a completed run.
pub(crate) async fn note_turn_finished(
    state: &Arc<dyn ChatRuntime>,
    activity_id: &str,
    succeeded: bool,
) {
    let outcome = if succeeded {
        moltis_channels::ChannelAckOutcome::Success
    } else {
        moltis_channels::ChannelAckOutcome::Failure
    };
    state
        .note_channel_activity(
            activity_id,
            moltis_channels::ChannelActivity::Finished(outcome),
        )
        .await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_and_defaults_ack_keys() {
        assert_eq!(
            ack_keys_from_params(&serde_json::json!({})),
            Vec::<String>::new()
        );
        assert_eq!(
            ack_keys_from_params(&serde_json::json!({ "_ack_keys": ["a", "b"] })),
            vec!["a".to_string(), "b".to_string()]
        );
    }

    #[test]
    fn merges_and_dedupes_across_queued_messages() {
        let a = serde_json::json!({ "_ack_keys": ["k1"] });
        let b = serde_json::json!({ "_ack_keys": ["k2", "k1"] });
        let c = serde_json::json!({});
        assert_eq!(merged_ack_keys([&a, &b, &c].into_iter()), vec![
            "k1".to_string(),
            "k2".to_string()
        ]);
    }
}
