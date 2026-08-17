//! Host-owned `chat.send` parameters that untrusted callers must not supply.
//!
//! Most request-scoped security parameters (`_tool_policy`, `_tool_audience`,
//! `_private_context`) can only ever *narrow* what a turn may do, so a caller
//! passing one is harmless. The names here are different: they assert something
//! the caller is not entitled to assert — that the request came from the channel
//! gateway, and where its reply should be delivered.
//!
//! Any transport that accepts caller-supplied JSON and forwards it to
//! `ChatService::send`/`send_sync` must strip these first. The list lives here,
//! next to the code that reads them, so a new marker cannot be added on the
//! reading side without the stripping side picking it up.

/// Parameters that only the channel gateway may set.
///
/// - `channel` — inbound sender metadata; drives per-sender tool policy and the
///   `private_context` history marker.
/// - `_channel_reply_target` — where the turn's output is delivered. A forged
///   value would let a caller address any chat the bot can reach.
/// - `_native_channel_request` — asserts the gateway already authorized this
///   turn, which skips the channel-bound public ceiling in
///   [`crate::service`]. This is the only one of the three that *widens*
///   privilege, so it is the one that most needs stripping.
pub const GATEWAY_OWNED_REQUEST_PARAMS: [&str; 3] = [
    "channel",
    "_channel_reply_target",
    "_native_channel_request",
];

/// Remove every [`GATEWAY_OWNED_REQUEST_PARAMS`] entry from a request.
///
/// A no-op for anything that is not a JSON object.
pub fn strip_gateway_owned_params(params: &mut serde_json::Value) {
    if let Some(object) = params.as_object_mut() {
        for name in GATEWAY_OWNED_REQUEST_PARAMS {
            object.remove(name);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_every_gateway_owned_param_and_keeps_the_rest() {
        let mut params = serde_json::json!({
            "text": "hello",
            "channel": {"sender_id": "forged"},
            "_channel_reply_target": {"chat_id": "forged"},
            "_native_channel_request": true,
            "_tool_policy": {"allow": ["calc"]},
        });

        strip_gateway_owned_params(&mut params);

        for name in GATEWAY_OWNED_REQUEST_PARAMS {
            assert!(params.get(name).is_none(), "{name} survived stripping");
        }
        // Restrictions a caller applies to itself are not privilege, so they stay.
        assert_eq!(params["_tool_policy"]["allow"][0], "calc");
        assert_eq!(params["text"], "hello");
    }

    #[test]
    fn non_object_params_are_left_alone() {
        let mut params = serde_json::json!("not an object");
        strip_gateway_owned_params(&mut params);
        assert_eq!(params, serde_json::json!("not an object"));
    }

    /// The reason this module exists: every name the chat service reads as a
    /// gateway assertion must be on the strip list. `_native_channel_request` is
    /// checked here explicitly because it is the only one that grants rather
    /// than restricts, so omitting it would be a privilege bypass rather than a
    /// routing bug.
    #[test]
    fn every_gateway_trust_marker_read_by_the_service_is_listed() {
        const CHANNEL_SECURITY: &str = include_str!("service/chat_impl/channel_security.rs");

        assert!(
            CHANNEL_SECURITY.contains("_native_channel_request"),
            "channel_security.rs no longer reads `_native_channel_request`; if the \
             trust marker was renamed, update GATEWAY_OWNED_REQUEST_PARAMS to match"
        );
        assert!(
            GATEWAY_OWNED_REQUEST_PARAMS.contains(&"_native_channel_request"),
            "the gateway trust marker must be stripped from caller-supplied params"
        );
    }
}
