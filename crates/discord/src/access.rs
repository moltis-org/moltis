use {
    moltis_channels::gating::{self, DmPolicy, GroupPolicy, MentionMode},
    moltis_common::types::ChatType,
};

use crate::config::DiscordAccountConfig;

/// Determine if an inbound message should be processed.
///
/// Returns `Ok(())` if the message is allowed, or `Err(reason)` if it should
/// be silently dropped.
pub fn check_access(
    config: &DiscordAccountConfig,
    chat_type: &ChatType,
    peer_id: &str,
    username: Option<&str>,
    guild_id: Option<&str>,
    bot_mentioned: bool,
) -> Result<(), AccessDenied> {
    match chat_type {
        ChatType::Dm => check_dm_access(config, peer_id, username),
        ChatType::Group | ChatType::Channel => check_guild_access(config, guild_id, bot_mentioned),
    }
}

fn check_dm_access(
    config: &DiscordAccountConfig,
    peer_id: &str,
    username: Option<&str>,
) -> Result<(), AccessDenied> {
    match config.dm_policy {
        DmPolicy::Disabled => Err(AccessDenied::DmsDisabled),
        DmPolicy::Open => Ok(()),
        DmPolicy::Allowlist => {
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

fn check_guild_access(
    config: &DiscordAccountConfig,
    guild_id: Option<&str>,
    bot_mentioned: bool,
) -> Result<(), AccessDenied> {
    match config.group_policy {
        GroupPolicy::Disabled => return Err(AccessDenied::GuildsDisabled),
        GroupPolicy::Allowlist => {
            let gid = guild_id.unwrap_or("");
            if config.guild_allowlist.is_empty()
                || !gating::is_allowed(gid, &config.guild_allowlist)
            {
                return Err(AccessDenied::GuildNotOnAllowlist);
            }
        },
        GroupPolicy::Open => {},
    }

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
    GuildsDisabled,
    GuildNotOnAllowlist,
    MentionModeNone,
    NotMentioned,
}

impl std::fmt::Display for AccessDenied {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DmsDisabled => write!(f, "DMs are disabled"),
            Self::NotOnAllowlist => write!(f, "user not on allowlist"),
            Self::GuildsDisabled => write!(f, "guilds are disabled"),
            Self::GuildNotOnAllowlist => write!(f, "guild not on allowlist"),
            Self::MentionModeNone => write!(f, "bot does not respond in guilds"),
            Self::NotMentioned => write!(f, "bot was not mentioned"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> DiscordAccountConfig {
        DiscordAccountConfig::default()
    }

    #[test]
    fn open_dm_allows_all() {
        let mut c = cfg();
        c.dm_policy = DmPolicy::Open;
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
        c.allowlist = vec!["400347514466992128".into()];
        assert!(check_access(&c, &ChatType::Dm, "400347514466992128", None, None, false).is_ok());
        assert_eq!(
            check_access(&c, &ChatType::Dm, "999999999", None, None, false),
            Err(AccessDenied::NotOnAllowlist)
        );
    }

    #[test]
    fn allowlist_dm_by_username() {
        let mut c = cfg();
        c.dm_policy = DmPolicy::Allowlist;
        c.allowlist = vec!["fabienpenso".into()];
        // Numeric peer_id doesn't match, but username does.
        assert!(
            check_access(
                &c,
                &ChatType::Dm,
                "400347514466992128",
                Some("fabienpenso"),
                None,
                false
            )
            .is_ok()
        );
        // Neither matches.
        assert_eq!(
            check_access(
                &c,
                &ChatType::Dm,
                "400347514466992128",
                Some("other"),
                None,
                false
            ),
            Err(AccessDenied::NotOnAllowlist)
        );
        // No username provided, peer_id doesn't match.
        assert_eq!(
            check_access(&c, &ChatType::Dm, "400347514466992128", None, None, false),
            Err(AccessDenied::NotOnAllowlist)
        );
    }

    #[test]
    fn allowlist_dm_matches_peer_id_or_username() {
        let mut c = cfg();
        c.dm_policy = DmPolicy::Allowlist;
        c.allowlist = vec!["400347514466992128".into()];
        // Peer ID matches, username is different — allowed.
        assert!(
            check_access(
                &c,
                &ChatType::Dm,
                "400347514466992128",
                Some("fabienpenso"),
                None,
                false
            )
            .is_ok()
        );
        // Username is in list but peer_id is not — also allowed.
        let mut c2 = cfg();
        c2.dm_policy = DmPolicy::Allowlist;
        c2.allowlist = vec!["fabienpenso".into()];
        assert!(
            check_access(
                &c2,
                &ChatType::Dm,
                "400347514466992128",
                Some("fabienpenso"),
                None,
                false
            )
            .is_ok()
        );
    }

    #[test]
    fn guild_mention_required() {
        let c = cfg(); // mention_mode=Mention by default
        assert_eq!(
            check_access(&c, &ChatType::Group, "user", None, Some("grp1"), false),
            Err(AccessDenied::NotMentioned)
        );
        assert!(check_access(&c, &ChatType::Group, "user", None, Some("grp1"), true).is_ok());
    }

    #[test]
    fn guild_always_mode() {
        let mut c = cfg();
        c.mention_mode = MentionMode::Always;
        assert!(check_access(&c, &ChatType::Group, "user", None, Some("grp1"), false).is_ok());
    }

    #[test]
    fn guild_disabled() {
        let mut c = cfg();
        c.group_policy = GroupPolicy::Disabled;
        assert_eq!(
            check_access(&c, &ChatType::Group, "user", None, Some("grp1"), true),
            Err(AccessDenied::GuildsDisabled)
        );
    }

    #[test]
    fn guild_allowlist() {
        let mut c = cfg();
        c.group_policy = GroupPolicy::Allowlist;
        c.guild_allowlist = vec!["grp1".into()];
        c.mention_mode = MentionMode::Always;
        assert!(check_access(&c, &ChatType::Group, "user", None, Some("grp1"), false).is_ok());
        assert_eq!(
            check_access(&c, &ChatType::Group, "user", None, Some("grp2"), false),
            Err(AccessDenied::GuildNotOnAllowlist)
        );
    }

    #[test]
    fn empty_dm_allowlist_denies_all() {
        let mut c = cfg();
        c.dm_policy = DmPolicy::Allowlist;
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
    fn empty_guild_allowlist_denies_all() {
        let mut c = cfg();
        c.group_policy = GroupPolicy::Allowlist;
        c.mention_mode = MentionMode::Always;
        assert_eq!(
            check_access(&c, &ChatType::Group, "user", None, Some("grp1"), true),
            Err(AccessDenied::GuildNotOnAllowlist)
        );
    }

    #[test]
    fn security_removing_last_allowlist_entry_denies_access() {
        let mut c = cfg();
        c.dm_policy = DmPolicy::Allowlist;
        c.allowlist = vec!["400347514466992128".into()];

        assert!(
            check_access(
                &c,
                &ChatType::Dm,
                "400347514466992128",
                Some("fabienpenso"),
                None,
                false
            )
            .is_ok()
        );

        c.allowlist.clear();

        assert_eq!(
            check_access(&c, &ChatType::Dm, "400347514466992128", None, None, false),
            Err(AccessDenied::NotOnAllowlist),
            "empty DM allowlist must deny by peer_id"
        );
        assert_eq!(
            check_access(
                &c,
                &ChatType::Dm,
                "400347514466992128",
                Some("fabienpenso"),
                None,
                false
            ),
            Err(AccessDenied::NotOnAllowlist),
            "empty DM allowlist must deny by username"
        );

        let mut g = cfg();
        g.group_policy = GroupPolicy::Allowlist;
        g.guild_allowlist = vec!["grp1".into()];
        g.mention_mode = MentionMode::Always;

        assert!(check_access(&g, &ChatType::Group, "user", None, Some("grp1"), true).is_ok());

        g.guild_allowlist.clear();

        assert_eq!(
            check_access(&g, &ChatType::Group, "user", None, Some("grp1"), true),
            Err(AccessDenied::GuildNotOnAllowlist),
            "empty guild allowlist must deny previously-allowed guild"
        );
    }
}
