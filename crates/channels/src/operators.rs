//! Operator (privileged sender) resolution for channel accounts.
//!
//! Channel access gating (`crate::gating`) answers "may this peer talk to the
//! bot at all". That is deliberately permissive: in a guild or group chat every
//! member who passes the group policy can talk to the bot.
//!
//! Operator resolution answers a narrower question: "is this sender eligible
//! for privileged actions". The gateway additionally requires a proven direct
//! conversation before exposing `/sh`, shell command mode, or tools that reach
//! the host (`exec`, file writes, …). Resolution is **fail-closed**: when
//! nothing is configured, nobody is an operator.

/// Privilege level of a channel sender for a single inbound message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ChannelSenderRole {
    /// Explicitly trusted; privileged access still requires a direct chat.
    Operator,
    /// Everyone else, including anonymous/unattributed senders.
    #[default]
    Guest,
}

impl ChannelSenderRole {
    /// Whether this role may perform privileged actions.
    #[must_use]
    pub fn is_operator(self) -> bool {
        matches!(self, Self::Operator)
    }
}

/// Check whether a sender matches an entry in a privilege list.
///
/// Unlike [`crate::gating::is_allowed`], an **empty list matches nobody** —
/// "no operators configured" must never mean "everyone is an operator".
///
/// Privileged identities are exact, case-sensitive platform sender IDs. Globs,
/// usernames, and generic suffix stripping are intentionally unsupported: the
/// semantics differ between platforms and can conflate distinct principals.
#[must_use]
pub fn is_listed(sender_id: &str, list: &[String]) -> bool {
    if list.is_empty() || sender_id.is_empty() {
        return false;
    }
    list.iter().any(|listed| listed == sender_id)
}

/// Resolve a sender's privilege level for an account.
///
/// Only the explicit `operators` list grants privilege. The conversational
/// allowlist is intentionally not consulted: OTP approval mutates that list,
/// and access to the bot must not imply access to the host.
///
/// An absent `sender_id` (e.g. an unattributed button callback) is always a
/// guest — privilege is never granted to an unidentified sender.
#[must_use]
pub fn resolve_sender_role(sender_id: Option<&str>, operators: &[String]) -> ChannelSenderRole {
    let Some(sender_id) = sender_id.filter(|s| !s.is_empty()) else {
        return ChannelSenderRole::Guest;
    };

    if is_listed(sender_id, operators) {
        ChannelSenderRole::Operator
    } else {
        ChannelSenderRole::Guest
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn empty_list_matches_nobody() {
        assert!(!is_listed("alice", &[]));
    }

    #[test]
    fn empty_sender_matches_nothing() {
        assert!(!is_listed("", &list(&["alice"])));
        assert!(!is_listed("", &list(&["*"])));
    }

    #[test]
    fn exact_platform_id_match() {
        let ops = list(&["Alice", "400347514466992128"]);
        assert!(is_listed("Alice", &ops));
        assert!(!is_listed("alice", &ops));
        assert!(!is_listed("ALICE", &ops));
        assert!(is_listed("400347514466992128", &ops));
        assert!(!is_listed("mallory", &ops));
    }

    #[test]
    fn wildcard_is_not_a_privileged_identity() {
        let ops = list(&["admin_*"]);
        assert!(!is_listed("admin_alice", &ops));
        assert!(!is_listed("user_bob", &ops));
    }

    #[test]
    fn suffixed_ids_require_the_full_platform_identity() {
        assert!(!is_listed(
            "15551234567@s.whatsapp.net",
            &list(&["15551234567"])
        ));
        assert!(is_listed(
            "@alice:example.org",
            &list(&["@alice:example.org"])
        ));
        assert!(!is_listed(
            "15559999999@s.whatsapp.net",
            &list(&["15551234567"])
        ));
    }

    #[test]
    fn only_operators_grant_privilege() {
        let operators = list(&["owner"]);
        assert_eq!(
            resolve_sender_role(Some("owner"), &operators),
            ChannelSenderRole::Operator
        );
        assert_eq!(
            resolve_sender_role(Some("helper"), &operators),
            ChannelSenderRole::Guest
        );
    }

    #[test]
    fn sender_ids_are_not_normalized_before_matching() {
        let operators = list(&["owner"]);
        assert_eq!(
            resolve_sender_role(Some(" owner "), &operators),
            ChannelSenderRole::Guest
        );
    }

    #[test]
    fn no_operators_fails_closed() {
        assert_eq!(
            resolve_sender_role(Some("owner"), &[]),
            ChannelSenderRole::Guest
        );
    }

    #[test]
    fn fails_closed_with_no_lists() {
        // Open account (no allowlist, no operators): nobody is privileged.
        assert_eq!(
            resolve_sender_role(Some("anyone"), &[]),
            ChannelSenderRole::Guest
        );
    }

    #[test]
    fn missing_or_blank_sender_is_guest() {
        let operators = list(&["owner"]);
        assert_eq!(
            resolve_sender_role(None, &operators),
            ChannelSenderRole::Guest
        );
        assert_eq!(
            resolve_sender_role(Some("   "), &operators),
            ChannelSenderRole::Guest
        );
    }

    #[test]
    fn wildcard_operator_entry_grants_nobody() {
        assert_eq!(
            resolve_sender_role(Some("anyone"), &list(&["*"])),
            ChannelSenderRole::Guest
        );
    }
}
