use {
    moltis_channels::gating::{DmPolicy, GroupPolicy, MentionMode, is_allowed},
    moltis_common::types::ChatType,
};

use crate::config::MatrixAccountConfig;

#[derive(Debug)]
pub enum AccessDenied {
    DmDisabled,
    DmNotAllowed,
    GroupDisabled,
    GroupNotAllowed,
    NotMentioned,
}

impl std::fmt::Display for AccessDenied {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DmDisabled => write!(f, "DMs disabled"),
            Self::DmNotAllowed => write!(f, "sender not on allowlist"),
            Self::GroupDisabled => write!(f, "groups disabled"),
            Self::GroupNotAllowed => write!(f, "room not on allowlist"),
            Self::NotMentioned => write!(f, "bot not mentioned"),
        }
    }
}

pub fn check_access(
    config: &MatrixAccountConfig,
    chat_type: &ChatType,
    sender_id: &str,
    username: Option<&str>,
    room_id: Option<&str>,
    bot_mentioned: bool,
) -> Result<(), AccessDenied> {
    match chat_type {
        ChatType::Dm => check_dm_access(config, sender_id, username),
        ChatType::Group | ChatType::Channel => {
            check_group_access(config, sender_id, username, room_id, bot_mentioned)
        },
    }
}

fn check_dm_access(
    config: &MatrixAccountConfig,
    sender_id: &str,
    username: Option<&str>,
) -> Result<(), AccessDenied> {
    match config.dm_policy {
        DmPolicy::Disabled => Err(AccessDenied::DmDisabled),
        DmPolicy::Open => Ok(()),
        DmPolicy::Allowlist => {
            if is_allowed(sender_id, &config.allowlist) {
                return Ok(());
            }
            if let Some(name) = username {
                if is_allowed(name, &config.allowlist) {
                    return Ok(());
                }
            }
            Err(AccessDenied::DmNotAllowed)
        },
    }
}

fn check_group_access(
    config: &MatrixAccountConfig,
    sender_id: &str,
    _username: Option<&str>,
    room_id: Option<&str>,
    bot_mentioned: bool,
) -> Result<(), AccessDenied> {
    match config.group_policy {
        GroupPolicy::Disabled => return Err(AccessDenied::GroupDisabled),
        GroupPolicy::Allowlist => {
            let rid = room_id.unwrap_or("");
            if !is_allowed(rid, &config.room_allowlist) && !is_allowed(sender_id, &config.allowlist)
            {
                return Err(AccessDenied::GroupNotAllowed);
            }
        },
        GroupPolicy::Open => {},
    }

    match config.mention_mode {
        MentionMode::Mention => {
            if !bot_mentioned {
                return Err(AccessDenied::NotMentioned);
            }
        },
        MentionMode::Always => {},
        MentionMode::None => return Err(AccessDenied::NotMentioned),
    }

    Ok(())
}

#[allow(clippy::unwrap_used)]
#[cfg(test)]
mod tests {
    use super::*;

    fn base_config() -> MatrixAccountConfig {
        MatrixAccountConfig::default()
    }

    #[test]
    fn dm_open_allows_anyone() {
        let mut cfg = base_config();
        cfg.dm_policy = DmPolicy::Open;
        assert!(check_access(&cfg, &ChatType::Dm, "@anyone:srv", None, None, false).is_ok());
    }

    #[test]
    fn dm_disabled_blocks_all() {
        let mut cfg = base_config();
        cfg.dm_policy = DmPolicy::Disabled;
        assert!(check_access(&cfg, &ChatType::Dm, "@anyone:srv", None, None, false).is_err());
    }

    #[test]
    fn dm_allowlist_allows_listed() {
        let mut cfg = base_config();
        cfg.dm_policy = DmPolicy::Allowlist;
        cfg.allowlist = vec!["@alice:srv".into()];
        assert!(check_access(&cfg, &ChatType::Dm, "@alice:srv", None, None, false).is_ok());
        assert!(check_access(&cfg, &ChatType::Dm, "@bob:srv", None, None, false).is_err());
    }

    #[test]
    fn group_open_mention_required() {
        let cfg = base_config();
        assert!(
            check_access(
                &cfg,
                &ChatType::Group,
                "@alice:srv",
                None,
                Some("!room:srv"),
                false
            )
            .is_err()
        );
        assert!(
            check_access(
                &cfg,
                &ChatType::Group,
                "@alice:srv",
                None,
                Some("!room:srv"),
                true
            )
            .is_ok()
        );
    }

    #[test]
    fn group_disabled_blocks_all() {
        let mut cfg = base_config();
        cfg.group_policy = GroupPolicy::Disabled;
        assert!(
            check_access(
                &cfg,
                &ChatType::Group,
                "@alice:srv",
                None,
                Some("!r:srv"),
                true
            )
            .is_err()
        );
    }
}
