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
            glob_match(&pat, &peer_lower)
        } else {
            pat == peer_lower
        }
    })
}

/// Simple glob matching supporting `*` as a wildcard for any sequence of chars.
fn glob_match(pattern: &str, text: &str) -> bool {
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
}
