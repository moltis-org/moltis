//! Inbound reaction updates.
//!
//! Telegram reports reactions as a whole-set diff rather than an add/remove
//! event: each update carries the reaction lists before and after the change.
//! The interesting part is therefore the difference between them.

use std::sync::Arc;

use {
    moltis_channels::{ChannelEvent, ChannelType},
    moltis_common::types::ChatType,
    teloxide::types::{MessageReactionUpdated, ReactionType},
    tracing::debug,
};

use crate::{access, state::AccountStateMap};

/// Emoji for a reaction, or `None` for kinds a vocabulary cannot match.
fn reaction_token(reaction: &ReactionType) -> Option<String> {
    match reaction {
        ReactionType::Emoji { emoji } => Some(emoji.clone()),
        // Custom emoji are identified by an opaque id with no readable name,
        // so there is nothing a feedback vocabulary could match against.
        _ => None,
    }
}

/// Reactions present in `after` but not `before`, and vice versa.
fn diff(before: &[ReactionType], after: &[ReactionType]) -> (Vec<String>, Vec<String>) {
    let old: Vec<String> = before.iter().filter_map(reaction_token).collect();
    let new: Vec<String> = after.iter().filter_map(reaction_token).collect();

    let added = new
        .iter()
        .filter(|e| !old.contains(e))
        .cloned()
        .collect::<Vec<_>>();
    let removed = old
        .iter()
        .filter(|e| !new.contains(e))
        .cloned()
        .collect::<Vec<_>>();
    (added, removed)
}

fn ordered_changes(before: &[ReactionType], after: &[ReactionType]) -> Vec<(String, bool)> {
    let (added, removed) = diff(before, after);
    removed
        .into_iter()
        .map(|emoji| (emoji, false))
        .chain(added.into_iter().map(|emoji| (emoji, true)))
        .collect()
}

/// Handle a `message_reaction` update.
pub async fn handle_message_reaction(
    update: MessageReactionUpdated,
    account_id: &str,
    accounts: &AccountStateMap,
) {
    // Anonymous channel reactions carry no user; without one, two different
    // people's votes would collapse onto a single score id.
    let Some(user) = update.actor.user() else {
        return;
    };

    let (config, event_sink) = {
        let accts = accounts.read().unwrap_or_else(|e| e.into_inner());
        match accts.get(account_id) {
            Some(state) => (state.config.clone(), state.event_sink.clone()),
            None => return,
        }
    };
    let Some(sink) = event_sink else {
        return;
    };

    let (chat_type, group_id) = match update.chat.kind {
        teloxide::types::ChatKind::Private(_) => (ChatType::Dm, None),
        teloxide::types::ChatKind::Public(ref public) => {
            let group_id = Some(update.chat.id.0.to_string());
            match public.kind {
                teloxide::types::PublicChatKind::Channel(_) => (ChatType::Channel, group_id),
                _ => (ChatType::Group, group_id),
            }
        },
    };
    let peer_id = user.id.0.to_string();
    let access_granted = access::check_access(
        &config,
        &chat_type,
        &peer_id,
        user.username.as_deref(),
        group_id.as_deref(),
        // Reactions do not need a fresh mention, but all other DM/group policy
        // and allowlist checks still apply.
        true,
    )
    .is_ok();

    let changes = ordered_changes(&update.old_reaction, &update.new_reaction);
    if changes.is_empty() {
        return;
    }

    debug!(
        account_id,
        changes = changes.len(),
        "telegram reaction update"
    );

    let emit = |emoji: String, was_added: bool| {
        let sink = Arc::clone(&sink);
        let account_id = account_id.to_string();
        let chat_id = update.chat.id.0.to_string();
        let message_id = update.message_id.0.to_string();
        let user_id = peer_id.clone();
        async move {
            sink.emit(ChannelEvent::ReactionChange {
                channel_type: ChannelType::Telegram,
                account_id,
                chat_id,
                message_id,
                user_id,
                emoji,
                added: was_added,
            })
            .await;
        }
    };

    for (emoji, added) in changes {
        // A removal can only retract this user's own score, so preserve source
        // truth even if an allowlist changed after the score was recorded.
        if added && !access_granted {
            debug!(account_id, user_id = %peer_id, "telegram reaction denied by access control");
            continue;
        }
        emit(emoji, added).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn emoji(raw: &str) -> ReactionType {
        ReactionType::Emoji {
            emoji: raw.to_string(),
        }
    }

    #[test]
    fn a_new_reaction_is_reported_as_added() {
        let (added, removed) = diff(&[], &[emoji("\u{1f44d}")]);

        assert_eq!(added, vec!["\u{1f44d}".to_string()]);
        assert!(removed.is_empty());
    }

    #[test]
    fn clearing_a_reaction_is_reported_as_removed() {
        let (added, removed) = diff(&[emoji("\u{1f44d}")], &[]);

        assert!(added.is_empty());
        assert_eq!(removed, vec!["\u{1f44d}".to_string()]);
    }

    #[test]
    fn switching_reactions_reports_both_sides() {
        // Telegram sends one update for a swap; scoring it needs the retract
        // and the new vote, not just whichever happened to be last.
        let (added, removed) = diff(&[emoji("\u{1f44d}")], &[emoji("\u{1f44e}")]);

        assert_eq!(added, vec!["\u{1f44e}".to_string()]);
        assert_eq!(removed, vec!["\u{1f44d}".to_string()]);
    }

    #[test]
    fn switching_reactions_retracts_before_setting_the_replacement() {
        assert_eq!(
            ordered_changes(&[emoji("\u{1f44d}")], &[emoji("\u{1f44e}")]),
            vec![
                ("\u{1f44d}".to_string(), false),
                ("\u{1f44e}".to_string(), true)
            ]
        );
    }

    #[test]
    fn an_unchanged_reaction_set_produces_nothing() {
        let (added, removed) = diff(&[emoji("\u{1f44d}")], &[emoji("\u{1f44d}")]);

        assert!(added.is_empty());
        assert!(removed.is_empty());
    }

    #[test]
    fn adding_alongside_an_existing_reaction_only_reports_the_new_one() {
        let (added, removed) = diff(&[emoji("\u{1f44d}")], &[
            emoji("\u{1f44d}"),
            emoji("\u{2764}"),
        ]);

        assert_eq!(added, vec!["\u{2764}".to_string()]);
        assert!(removed.is_empty());
    }

    #[test]
    fn custom_emoji_are_skipped() {
        // Identified by an opaque id with no readable name, so a vocabulary
        // has nothing to match.
        let custom = ReactionType::CustomEmoji {
            custom_emoji_id: teloxide::types::CustomEmojiId("12345".to_string()),
        };
        let (added, removed) = diff(&[], &[custom]);

        assert!(added.is_empty());
        assert!(removed.is_empty());
    }
}
