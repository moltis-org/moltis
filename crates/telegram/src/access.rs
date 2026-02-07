use {
    moltis_channels::gating::{self, DmPolicy, GroupPolicy, MentionMode},
    moltis_common::types::ChatType,
};

use crate::config::TelegramAccountConfig;

/// Determine if an inbound message should be processed.
///
/// Returns `Ok(())` if the message is allowed, or `Err(reason)` if it should
/// be silently dropped.
pub fn check_access(
    config: &TelegramAccountConfig,
    chat_type: &ChatType,
    peer_id: &str,
    username: Option<&str>,
    group_id: Option<&str>,
    bot_mentioned: bool,
) -> Result<(), AccessDenied> {
    match chat_type {
        ChatType::Dm => check_dm_access(config, peer_id, username),
        ChatType::Group | ChatType::Channel => {
            check_group_access(config, peer_id, group_id, bot_mentioned)
        },
    }
}

fn check_dm_access(
    config: &TelegramAccountConfig,
    peer_id: &str,
    username: Option<&str>,
) -> Result<(), AccessDenied> {
    match config.dm_policy {
        DmPolicy::Disabled => Err(AccessDenied::DmsDisabled),
        DmPolicy::Open => Ok(()),
        DmPolicy::Allowlist => {
            // An empty allowlist with an explicit Allowlist policy means
            // "deny everyone" — not "allow everyone".  The generic
            // `is_allowed()` treats empty lists as open, so we
            // short-circuit here.
            if config.allowlist.is_empty() {
                return Err(AccessDenied::NotOnAllowlist);
            }
            if gating::is_allowed(peer_id, &config.allowlist)
                || username.is_some_and(|u| gating::is_allowed(u, &config.allowlist))
            {
                Ok(())
            } else {
                Err(AccessDenied::NotOnAllowlist)
            }
        },
    }
}

fn check_group_access(
    config: &TelegramAccountConfig,
    _peer_id: &str,
    group_id: Option<&str>,
    bot_mentioned: bool,
) -> Result<(), AccessDenied> {
    match config.group_policy {
        GroupPolicy::Disabled => return Err(AccessDenied::GroupsDisabled),
        GroupPolicy::Allowlist => {
            let gid = group_id.unwrap_or("");
            if config.group_allowlist.is_empty()
                || !gating::is_allowed(gid, &config.group_allowlist)
            {
                return Err(AccessDenied::GroupNotOnAllowlist);
            }
        },
        GroupPolicy::Open => {},
    }

    // Mention gating
    match config.mention_mode {
        MentionMode::Always => Ok(()),
        MentionMode::None => Err(AccessDenied::MentionModeNone),
        MentionMode::Mention => {
            if bot_mentioned {
                Ok(())
            } else {
                Err(AccessDenied::NotMentioned)
            }
        },
    }
}

/// Reason an inbound message was denied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccessDenied {
    DmsDisabled,
    NotOnAllowlist,
    GroupsDisabled,
    GroupNotOnAllowlist,
    MentionModeNone,
    NotMentioned,
}

impl std::fmt::Display for AccessDenied {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DmsDisabled => write!(f, "DMs are disabled"),
            Self::NotOnAllowlist => write!(f, "user not on allowlist"),
            Self::GroupsDisabled => write!(f, "groups are disabled"),
            Self::GroupNotOnAllowlist => write!(f, "group not on allowlist"),
            Self::MentionModeNone => write!(f, "bot does not respond in groups"),
            Self::NotMentioned => write!(f, "bot was not mentioned"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> TelegramAccountConfig {
        TelegramAccountConfig::default()
    }

    #[test]
    fn open_dm_allows_all() {
        let c = cfg();
        assert!(check_access(&c, &ChatType::Dm, "anyone", None, None, false).is_ok());
    }

    #[test]
    fn disabled_dm_rejects() {
        let mut c = cfg();
        c.dm_policy = DmPolicy::Disabled;
        assert_eq!(
            check_access(&c, &ChatType::Dm, "user", None, None, false),
            Err(AccessDenied::DmsDisabled)
        );
    }

    #[test]
    fn allowlist_dm_by_peer_id() {
        let mut c = cfg();
        c.dm_policy = DmPolicy::Allowlist;
        c.allowlist = vec!["alice".into()];
        assert!(check_access(&c, &ChatType::Dm, "alice", None, None, false).is_ok());
        assert_eq!(
            check_access(&c, &ChatType::Dm, "bob", None, None, false),
            Err(AccessDenied::NotOnAllowlist)
        );
    }

    #[test]
    fn allowlist_dm_by_username() {
        let mut c = cfg();
        c.dm_policy = DmPolicy::Allowlist;
        c.allowlist = vec!["fabienpenso".into()];
        // Numeric peer_id doesn't match, but username does
        assert!(
            check_access(
                &c,
                &ChatType::Dm,
                "377114917",
                Some("fabienpenso"),
                None,
                false
            )
            .is_ok()
        );
        // Neither matches
        assert_eq!(
            check_access(&c, &ChatType::Dm, "377114917", Some("other"), None, false),
            Err(AccessDenied::NotOnAllowlist)
        );
        // No username provided, peer_id doesn't match
        assert_eq!(
            check_access(&c, &ChatType::Dm, "377114917", None, None, false),
            Err(AccessDenied::NotOnAllowlist)
        );
    }

    #[test]
    fn group_mention_required() {
        let c = cfg(); // mention_mode=Mention by default
        assert_eq!(
            check_access(&c, &ChatType::Group, "user", None, Some("grp1"), false),
            Err(AccessDenied::NotMentioned)
        );
        assert!(check_access(&c, &ChatType::Group, "user", None, Some("grp1"), true).is_ok());
    }

    #[test]
    fn group_always_mode() {
        let mut c = cfg();
        c.mention_mode = MentionMode::Always;
        assert!(check_access(&c, &ChatType::Group, "user", None, Some("grp1"), false).is_ok());
    }

    #[test]
    fn group_disabled() {
        let mut c = cfg();
        c.group_policy = GroupPolicy::Disabled;
        assert_eq!(
            check_access(&c, &ChatType::Group, "user", None, Some("grp1"), true),
            Err(AccessDenied::GroupsDisabled)
        );
    }

    #[test]
    fn group_allowlist() {
        let mut c = cfg();
        c.group_policy = GroupPolicy::Allowlist;
        c.group_allowlist = vec!["grp1".into()];
        c.mention_mode = MentionMode::Always;
        assert!(check_access(&c, &ChatType::Group, "user", None, Some("grp1"), false).is_ok());
        assert_eq!(
            check_access(&c, &ChatType::Group, "user", None, Some("grp2"), false),
            Err(AccessDenied::GroupNotOnAllowlist)
        );
    }

    #[test]
    fn empty_dm_allowlist_denies_all() {
        let mut c = cfg();
        c.dm_policy = DmPolicy::Allowlist;
        // allowlist is empty — should deny, not allow
        assert_eq!(
            check_access(&c, &ChatType::Dm, "anyone", None, None, false),
            Err(AccessDenied::NotOnAllowlist)
        );
        assert_eq!(
            check_access(&c, &ChatType::Dm, "anyone", Some("user"), None, false),
            Err(AccessDenied::NotOnAllowlist)
        );
    }

    #[test]
    fn empty_group_allowlist_denies_all() {
        let mut c = cfg();
        c.group_policy = GroupPolicy::Allowlist;
        c.mention_mode = MentionMode::Always;
        // group_allowlist is empty — should deny, not allow
        assert_eq!(
            check_access(&c, &ChatType::Group, "user", None, Some("grp1"), true),
            Err(AccessDenied::GroupNotOnAllowlist)
        );
    }

    /// Security regression: removing the last entry from an allowlist must
    /// NOT silently switch to open access.  An explicit Allowlist policy with
    /// an empty list must deny every peer — by peer ID, by username, and in
    /// groups alike.  Failure here means unauthenticated users can bypass the
    /// allowlist by convincing an admin to remove all entries.
    #[test]
    fn security_removing_last_allowlist_entry_denies_access() {
        // --- DM: user is on the list, gets removed, must be denied ---
        let mut c = cfg();
        c.dm_policy = DmPolicy::Allowlist;
        c.allowlist = vec!["377114917".into()];

        // While on the list: allowed
        assert!(check_access(&c, &ChatType::Dm, "377114917", Some("alice"), None, false).is_ok());

        // Simulate admin removing the sole entry via the UI
        c.allowlist.clear();

        // After removal: denied by peer ID alone
        assert_eq!(
            check_access(&c, &ChatType::Dm, "377114917", None, None, false),
            Err(AccessDenied::NotOnAllowlist),
            "empty DM allowlist must deny by peer_id"
        );
        // After removal: denied even when username is provided
        assert_eq!(
            check_access(&c, &ChatType::Dm, "377114917", Some("alice"), None, false),
            Err(AccessDenied::NotOnAllowlist),
            "empty DM allowlist must deny by username"
        );
        // After removal: other users also denied
        assert_eq!(
            check_access(&c, &ChatType::Dm, "999", Some("eve"), None, false),
            Err(AccessDenied::NotOnAllowlist),
            "empty DM allowlist must deny unknown users"
        );

        // --- Group: same invariant ---
        let mut g = cfg();
        g.group_policy = GroupPolicy::Allowlist;
        g.group_allowlist = vec!["grp1".into()];
        g.mention_mode = MentionMode::Always;

        assert!(check_access(&g, &ChatType::Group, "user", None, Some("grp1"), true).is_ok());

        g.group_allowlist.clear();

        assert_eq!(
            check_access(&g, &ChatType::Group, "user", None, Some("grp1"), true),
            Err(AccessDenied::GroupNotOnAllowlist),
            "empty group allowlist must deny previously-allowed group"
        );
        assert_eq!(
            check_access(&g, &ChatType::Group, "user", None, Some("grp2"), true),
            Err(AccessDenied::GroupNotOnAllowlist),
            "empty group allowlist must deny unknown groups"
        );
    }
}
