//! Keys for the `_`-prefixed internal params carried alongside a `chat.send`.
//!
//! The underscore prefix marks a param as plumbing rather than user input, but
//! it does **not** mean "server-only": the web UI legitimately sets
//! `_session_key`, `_audio_filename` and `_document_files` on its own calls.
//!
//! [`SERVER_ONLY`] is the subset that a client must never be able to supply,
//! because each one names a resource the caller would otherwise be asserting
//! authority over. The RPC boundary strips these from client params; the server
//! re-derives them from state it trusts. Defined here — rather than as string
//! literals at each call site — so the code that sets a key, the code that
//! reads it, and the code that strips it cannot drift apart.

/// Acknowledgment identities a call is responsible for.
///
/// Minted by the channel dispatch path, which mints one per inbound message and
/// carries it through queueing and replay so the reaction follows the message
/// itself. A client-supplied value would let one user drive the ✅/❌ reaction on
/// an unrelated inbound channel message.
pub const ACK_KEYS: &str = "_ack_keys";

/// Channel destination a reply should be echoed to.
///
/// Derived server-side in `chat_impl::send` from the session's persisted
/// `channel_binding`, and only when that session is still the active one for
/// the chat. A client-supplied value would let one user address an outbound
/// message to any chat the bot can reach.
pub const CHANNEL_REPLY_TARGET: &str = "_channel_reply_target";

/// Params that only the server may set, stripped from client-supplied params at
/// the RPC boundary.
///
/// Add a key here when it confers authority the caller does not inherently
/// have — naming another message, another chat, or another session's resources.
pub const SERVER_ONLY: &[&str] = &[ACK_KEYS, CHANNEL_REPLY_TARGET];

/// Remove every [`SERVER_ONLY`] key from client-supplied params.
///
/// A non-object value has no params to strip and is left alone; the chat
/// service rejects it later on its own terms.
pub fn strip_server_only(params: &mut serde_json::Value) {
    let Some(object) = params.as_object_mut() else {
        return;
    };
    for key in SERVER_ONLY {
        object.remove(*key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_every_server_only_key_and_keeps_the_rest() {
        let mut params = serde_json::json!({
            "text": "hi",
            "_session_key": "session:42",
            ACK_KEYS: ["acct:C1:1.0"],
            CHANNEL_REPLY_TARGET: { "chat_id": "C999" },
        });
        strip_server_only(&mut params);

        assert_eq!(params["text"], "hi");
        assert_eq!(params["_session_key"], "session:42");
        for key in SERVER_ONLY {
            assert!(params.get(*key).is_none(), "{key} survived");
        }
    }

    #[test]
    fn non_object_params_are_left_alone() {
        let mut params = serde_json::json!("just a string");
        strip_server_only(&mut params);
        assert_eq!(params, serde_json::json!("just a string"));
    }
}
