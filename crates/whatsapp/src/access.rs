use moltis_channels::gating::{self, DmPolicy, GroupPolicy};

use crate::config::WhatsAppAccountConfig;

/// Determine if an inbound WhatsApp message should be processed.
///
/// Returns `Ok(())` if the message is allowed, or `Err(reason)` if it should
/// be denied. WhatsApp does not have @mention semantics like Telegram bots,
/// so there is no `MentionMode` gating.
pub fn check_access(
    config: &WhatsAppAccountConfig,
    is_group: bool,
    peer_id: &str,
    username: Option<&str>,
    group_id: Option<&str>,
) -> Result<(), AccessDenied> {
    if is_group {
        check_group_access(config, group_id)
    } else {
        check_dm_access(config, peer_id, username)
    }
}

fn check_dm_access(
    config: &WhatsAppAccountConfig,
    peer_id: &str,
    username: Option<&str>,
) -> Result<(), AccessDenied> {
    match config.dm_policy {
        DmPolicy::Disabled => Err(AccessDenied::DmsDisabled),
        DmPolicy::Open => Ok(()),
        DmPolicy::Allowlist => {
            // An empty allowlist with an explicit Allowlist policy means
            // "deny everyone" — not "allow everyone".
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
    config: &WhatsAppAccountConfig,
    group_id: Option<&str>,
) -> Result<(), AccessDenied> {
    match config.group_policy {
        GroupPolicy::Disabled => Err(AccessDenied::GroupsDisabled),
        GroupPolicy::Allowlist => {
            let gid = group_id.unwrap_or("");
            if config.group_allowlist.is_empty()
                || !gating::is_allowed(gid, &config.group_allowlist)
            {
                Err(AccessDenied::GroupNotOnAllowlist)
            } else {
                Ok(())
            }
        },
        GroupPolicy::Open => Ok(()),
    }
}

/// Reason an inbound message was denied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccessDenied {
    DmsDisabled,
    NotOnAllowlist,
    GroupsDisabled,
    GroupNotOnAllowlist,
}

impl std::fmt::Display for AccessDenied {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DmsDisabled => write!(f, "DMs are disabled"),
            Self::NotOnAllowlist => write!(f, "user not on allowlist"),
            Self::GroupsDisabled => write!(f, "groups are disabled"),
            Self::GroupNotOnAllowlist => write!(f, "group not on allowlist"),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn cfg() -> WhatsAppAccountConfig {
        WhatsAppAccountConfig::default()
    }

    #[test]
    fn open_dm_allows_all() {
        let mut c = cfg();
        c.dm_policy = DmPolicy::Open;
        assert!(check_access(&c, false, "anyone", None, None).is_ok());
    }

    #[test]
    fn disabled_dm_rejects() {
        let mut c = cfg();
        c.dm_policy = DmPolicy::Disabled;
        assert_eq!(
            check_access(&c, false, "user", None, None),
            Err(AccessDenied::DmsDisabled)
        );
    }

    #[test]
    fn allowlist_dm_by_peer_id() {
        let mut c = cfg();
        c.dm_policy = DmPolicy::Allowlist;
        c.allowlist = vec!["15551234567".into()];
        assert!(check_access(&c, false, "15551234567", None, None).is_ok());
        assert_eq!(
            check_access(&c, false, "15559876543", None, None),
            Err(AccessDenied::NotOnAllowlist)
        );
    }

    #[test]
    fn allowlist_dm_by_username() {
        let mut c = cfg();
        c.dm_policy = DmPolicy::Allowlist;
        c.allowlist = vec!["alice".into()];
        // JID peer_id doesn't match, but username does
        assert!(check_access(&c, false, "15551234567@s.whatsapp.net", Some("alice"), None).is_ok());
        // Neither matches
        assert_eq!(
            check_access(&c, false, "15551234567@s.whatsapp.net", Some("bob"), None),
            Err(AccessDenied::NotOnAllowlist)
        );
    }

    #[test]
    fn group_open_allows_all() {
        let c = cfg();
        assert!(check_access(&c, true, "user", None, Some("group1")).is_ok());
    }

    #[test]
    fn group_disabled_rejects() {
        let mut c = cfg();
        c.group_policy = GroupPolicy::Disabled;
        assert_eq!(
            check_access(&c, true, "user", None, Some("group1")),
            Err(AccessDenied::GroupsDisabled)
        );
    }

    #[test]
    fn group_allowlist() {
        let mut c = cfg();
        c.group_policy = GroupPolicy::Allowlist;
        c.group_allowlist = vec!["grp1".into()];
        assert!(check_access(&c, true, "user", None, Some("grp1")).is_ok());
        assert_eq!(
            check_access(&c, true, "user", None, Some("grp2")),
            Err(AccessDenied::GroupNotOnAllowlist)
        );
    }

    #[test]
    fn empty_dm_allowlist_denies_all() {
        let mut c = cfg();
        c.dm_policy = DmPolicy::Allowlist;
        assert_eq!(
            check_access(&c, false, "anyone", None, None),
            Err(AccessDenied::NotOnAllowlist)
        );
        assert_eq!(
            check_access(&c, false, "anyone", Some("user"), None),
            Err(AccessDenied::NotOnAllowlist)
        );
    }

    #[test]
    fn empty_group_allowlist_denies_all() {
        let mut c = cfg();
        c.group_policy = GroupPolicy::Allowlist;
        assert_eq!(
            check_access(&c, true, "user", None, Some("grp1")),
            Err(AccessDenied::GroupNotOnAllowlist)
        );
    }

    /// Security regression: removing the last entry from an allowlist must
    /// NOT silently switch to open access.
    #[test]
    fn security_removing_last_allowlist_entry_denies_access() {
        // --- DM ---
        let mut c = cfg();
        c.dm_policy = DmPolicy::Allowlist;
        c.allowlist = vec!["15551234567".into()];

        assert!(check_access(&c, false, "15551234567", Some("alice"), None).is_ok());

        c.allowlist.clear();

        assert_eq!(
            check_access(&c, false, "15551234567", None, None),
            Err(AccessDenied::NotOnAllowlist),
            "empty DM allowlist must deny by peer_id"
        );
        assert_eq!(
            check_access(&c, false, "15551234567", Some("alice"), None),
            Err(AccessDenied::NotOnAllowlist),
            "empty DM allowlist must deny by username"
        );

        // --- Group ---
        let mut g = cfg();
        g.group_policy = GroupPolicy::Allowlist;
        g.group_allowlist = vec!["grp1".into()];

        assert!(check_access(&g, true, "user", None, Some("grp1")).is_ok());

        g.group_allowlist.clear();

        assert_eq!(
            check_access(&g, true, "user", None, Some("grp1")),
            Err(AccessDenied::GroupNotOnAllowlist),
            "empty group allowlist must deny previously-allowed group"
        );
    }
}
