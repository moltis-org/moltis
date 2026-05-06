use {
    moltis_channels::gating::{self, DmPolicy, GroupPolicy, MentionMode},
    moltis_common::types::ChatType,
};

use crate::config::MatrixAccountConfig;

/// Determine if an inbound Matrix message should be processed.
pub fn check_access(
    config: &MatrixAccountConfig,
    chat_type: &ChatType,
    sender_id: &str,
    room_id: &str,
    bot_mentioned: bool,
) -> Result<(), AccessDenied> {
    match chat_type {
        ChatType::Dm => check_dm_access(config, sender_id),
        ChatType::Group | ChatType::Channel => check_room_access(config, room_id, bot_mentioned),
    }
}

fn check_dm_access(config: &MatrixAccountConfig, sender_id: &str) -> Result<(), AccessDenied> {
    match config.dm_policy {
        DmPolicy::Disabled => Err(AccessDenied::DmsDisabled),
        DmPolicy::Open => Ok(()),
        DmPolicy::Allowlist => {
            if config.user_allowlist.is_empty() {
                return Err(AccessDenied::NotOnAllowlist);
            }
            if gating::is_allowed(sender_id, &config.user_allowlist) {
                Ok(())
            } else {
                Err(AccessDenied::NotOnAllowlist)
            }
        },
    }
}

fn check_room_access(
    config: &MatrixAccountConfig,
    room_id: &str,
    bot_mentioned: bool,
) -> Result<(), AccessDenied> {
    match config.room_policy {
        GroupPolicy::Disabled => Err(AccessDenied::RoomsDisabled),
        GroupPolicy::Open => Ok(()),
        GroupPolicy::Allowlist => {
            if config.room_allowlist.is_empty()
                || !gating::is_allowed(room_id, &config.room_allowlist)
            {
                Err(AccessDenied::RoomNotOnAllowlist)
            } else {
                Ok(())
            }
        },
    }?;

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccessDenied {
    DmsDisabled,
    NotOnAllowlist,
    RoomsDisabled,
    RoomNotOnAllowlist,
    MentionModeNone,
    NotMentioned,
}

impl std::fmt::Display for AccessDenied {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DmsDisabled => write!(f, "DMs are disabled"),
            Self::NotOnAllowlist => write!(f, "user not on allowlist"),
            Self::RoomsDisabled => write!(f, "rooms are disabled"),
            Self::RoomNotOnAllowlist => write!(f, "room not on allowlist"),
            Self::MentionModeNone => write!(f, "bot does not respond in rooms"),
            Self::NotMentioned => write!(f, "bot was not mentioned"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> MatrixAccountConfig {
        MatrixAccountConfig::default()
    }

    #[test]
    fn open_dm_allows_all() {
        let mut c = cfg();
        c.dm_policy = DmPolicy::Open;
        assert!(check_access(&c, &ChatType::Dm, "@anyone:example.com", "", false).is_ok());
    }

    #[test]
    fn disabled_dm_rejects() {
        let mut c = cfg();
        c.dm_policy = DmPolicy::Disabled;
        assert_eq!(
            check_access(&c, &ChatType::Dm, "@user:example.com", "", false),
            Err(AccessDenied::DmsDisabled)
        );
    }

    #[test]
    fn allowlist_dm() {
        let mut c = cfg();
        c.dm_policy = DmPolicy::Allowlist;
        c.user_allowlist = vec!["@alice:example.com".into()];
        assert!(check_access(&c, &ChatType::Dm, "@alice:example.com", "", false).is_ok());
        assert_eq!(
            check_access(&c, &ChatType::Dm, "@bob:example.com", "", false),
            Err(AccessDenied::NotOnAllowlist)
        );
    }

    #[test]
    fn room_allowlist() {
        let mut c = cfg();
        c.room_policy = GroupPolicy::Allowlist;
        c.room_allowlist = vec!["!room1:example.com".into()];
        assert!(
            check_access(
                &c,
                &ChatType::Group,
                "@user:example.com",
                "!room1:example.com",
                true
            )
            .is_ok()
        );
        assert_eq!(
            check_access(
                &c,
                &ChatType::Group,
                "@user:example.com",
                "!room2:example.com",
                true
            ),
            Err(AccessDenied::RoomNotOnAllowlist)
        );
    }

    #[test]
    fn room_requires_mention_by_default() {
        let mut c = cfg();
        c.room_policy = GroupPolicy::Open;

        assert_eq!(
            check_access(
                &c,
                &ChatType::Group,
                "@user:example.com",
                "!room:example.com",
                false
            ),
            Err(AccessDenied::NotMentioned)
        );
        assert!(
            check_access(
                &c,
                &ChatType::Group,
                "@user:example.com",
                "!room:example.com",
                true
            )
            .is_ok()
        );
    }

    #[test]
    fn room_none_mode_rejects_even_if_mentioned() {
        let mut c = cfg();
        c.room_policy = GroupPolicy::Open;
        c.mention_mode = MentionMode::None;

        assert_eq!(
            check_access(
                &c,
                &ChatType::Group,
                "@user:example.com",
                "!room:example.com",
                true
            ),
            Err(AccessDenied::MentionModeNone)
        );
    }

    #[test]
    fn room_always_mode_accepts_without_mention() {
        let mut c = cfg();
        c.room_policy = GroupPolicy::Open;
        c.mention_mode = MentionMode::Always;

        assert!(
            check_access(
                &c,
                &ChatType::Group,
                "@user:example.com",
                "!room:example.com",
                false
            )
            .is_ok()
        );
    }
}
