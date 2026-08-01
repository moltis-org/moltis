use serde::{Deserialize, Serialize};

/// Check if a peer is allowed to interact with the bot.
///
/// An empty allowlist means everyone is allowed (open policy).
/// Entries are matched case-insensitively against the peer ID.
/// Supports exact match and glob-style `*` wildcards.
pub fn is_allowed(peer_id: &str, allowlist: &[String]) -> bool {
    if allowlist.is_empty() {
        return true;
    }
    let peer_lower = peer_id.to_lowercase();
    allowlist.iter().any(|pattern| {
        let pat = pattern.to_lowercase();
        if pat.contains('*') {
            glob_match_lower(&pat, &peer_lower)
        } else {
            pat == peer_lower
        }
    })
}

/// Whether a message sender matches an allowlist, by identifier text.
///
/// Sits on top of [`is_allowed`] and additionally tries the user part of an
/// `@`-qualified id, because some channels report a fully qualified address
/// where the allowlist holds the bare user (WhatsApp JIDs are e.g.
/// `15551234567@s.whatsapp.net` against an allowlist of plain phone numbers).
///
/// This is the textual fallback. A channel whose identifiers have more than one
/// valid spelling should override
/// [`ChannelConfigView::sender_on_allowlist`](crate::config_view::ChannelConfigView::sender_on_allowlist)
/// and compare parsed identities instead.
pub fn sender_matches_allowlist(sender_id: &str, allowlist: &[String]) -> bool {
    is_allowed(sender_id, allowlist)
        || sender_id
            .split_once('@')
            .is_some_and(|(user, _)| is_allowed(user, allowlist))
}

/// Simple glob matching supporting `*` as a wildcard for any sequence of chars.
///
/// Both arguments must already be lowercased by the caller.
fn glob_match_lower(pattern: &str, text: &str) -> bool {
    let parts: Vec<&str> = pattern.split('*').collect();
    if parts.len() == 1 {
        return pattern == text;
    }

    let mut pos = 0;
    for (i, part) in parts.iter().enumerate() {
        if part.is_empty() {
            continue;
        }
        match text[pos..].find(part) {
            Some(idx) => {
                // First segment must match at start
                if i == 0 && idx != 0 {
                    return false;
                }
                pos += idx + part.len();
            },
            None => return false,
        }
    }
    // Last segment must match at end (unless pattern ends with *)
    if !parts.last().unwrap_or(&"").is_empty() {
        pos == text.len()
    } else {
        true
    }
}

/// Mention activation mode for group chats.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MentionMode {
    /// Bot must be @mentioned to respond.
    #[default]
    Mention,
    /// Bot responds to all messages.
    Always,
    /// Bot does not respond in groups.
    None,
}

/// DM access policy.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DmPolicy {
    /// Anyone can DM the bot.
    Open,
    /// Only users on the allowlist.
    #[default]
    Allowlist,
    /// DMs disabled.
    Disabled,
}

/// Group access policy.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum GroupPolicy {
    /// Bot responds in all groups.
    #[default]
    Open,
    /// Only in groups on the allowlist.
    Allowlist,
    /// Groups disabled.
    Disabled,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_allowlist_allows_everyone() {
        assert!(is_allowed("anyone", &[]));
    }

    #[test]
    fn exact_match() {
        let list = vec!["alice".into(), "bob".into()];
        assert!(is_allowed("alice", &list));
        assert!(is_allowed("Alice", &list));
        assert!(!is_allowed("charlie", &list));
    }

    #[test]
    fn glob_wildcard() {
        let list = vec!["admin_*".into()];
        assert!(is_allowed("admin_alice", &list));
        assert!(!is_allowed("user_bob", &list));
    }

    #[test]
    fn glob_suffix() {
        let list = vec!["*@example.com".into()];
        assert!(is_allowed("user@example.com", &list));
        assert!(!is_allowed("user@other.com", &list));
    }

    #[test]
    fn glob_middle() {
        let list = vec!["user_*_admin".into()];
        assert!(is_allowed("user_123_admin", &list));
        assert!(!is_allowed("user_123_mod", &list));
    }

    /// WhatsApp reports the sender as a JID; allowlists hold the bare number.
    #[test]
    fn sender_match_falls_back_to_the_user_part_of_a_jid() {
        let list = vec!["15551234567".to_string()];
        assert!(sender_matches_allowlist(
            "15551234567@s.whatsapp.net",
            &list
        ));
        assert!(!sender_matches_allowlist(
            "15559999999@s.whatsapp.net",
            &list
        ));
    }

    #[test]
    fn sender_match_accepts_the_full_id_too() {
        let list = vec!["user@example.com".to_string()];
        assert!(sender_matches_allowlist("user@example.com", &list));
    }
}
