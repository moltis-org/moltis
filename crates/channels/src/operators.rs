//! Operator (privileged sender) resolution for channel accounts.
//!
//! Channel access gating (`crate::gating`) answers "may this peer talk to the
//! bot at all". That is deliberately permissive: in a guild or group chat every
//! member who passes the group policy can talk to the bot.
//!
//! Operator resolution answers a narrower question: "may this sender run
//! privileged actions" — `/sh`, shell command mode, and tools that reach the
//! host (`exec`, file writes, …). It is **fail-closed**: when nothing is
//! configured, nobody is an operator.

use crate::gating::glob_match_lower;

/// Privilege level of a channel sender for a single inbound message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ChannelSenderRole {
    /// Explicitly trusted: may use `/sh`, command mode, and privileged tools.
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

/// Tools denied to guest (non-operator) channel senders by default.
///
/// Four families are blocked:
/// - **Host reach** — command execution, file writes, the browser, sandbox
///   image control, remote nodes, and the external agent bridges (`codex`,
///   `opencode`, `anthropic`), each of which runs code on the host.
/// - **Owner state** — `memory_*`, `sessions_*`, `codebase_*`, checkpoints,
///   calendar, Teams message history, and location. A guest must not read or
///   edit the owner's private notes, code, calendar, or other conversations
///   through the bot.
/// - **Escalation** — `update_channel_settings` edits allowlists and this very
///   operator list, so a guest who reached it could promote themselves.
/// - **Persistence, side effects and lateral movement** — `cron`, `webhook`,
///   `spawn_*`, `send_message` (proactive messages to any chat), `voice_call`,
///   `home_assistant`, skill authoring (a written skill is code the agent later
///   runs), and every MCP tool (`mcp_*`, covering both the management tools and
///   the `mcp__server__tool` call namespace), since MCP servers are the owner's
///   own integrations.
///
/// Guests keep the informational and reply tools (`web_search`, `web_fetch`,
/// `calc`, `generate_image`, `send_image`, `send_document`, …) so the bot stays
/// useful in public rooms.
///
/// Patterns use the same syntax as `moltis_tools::policy::ToolPolicy`: exact
/// names or a trailing `*` prefix wildcard. An account can tighten this further
/// through the per-channel tool policy at
/// `channels.<type>.<account>.tools.groups.<chat_type>`.
pub const DEFAULT_GUEST_DENIED_TOOLS: &[&str] = &[
    // Host reach.
    "exec",
    "process",
    "write_file",
    "browser",
    "screenshot_tool",
    "sandbox_packages",
    "nodes_*",
    "codex",
    "opencode",
    "anthropic",
    // Owner state.
    "memory_*",
    "sessions_*",
    "session_state",
    "branch_session",
    "checkpoint_restore",
    "checkpoints_list",
    "auto_checkpoint",
    "codebase_*",
    "caldav",
    "teams_*",
    "get_user_location",
    // Escalation: edits allowlists and the operator list itself.
    "update_channel_settings",
    // Persistence, side effects, and lateral movement.
    "cron",
    "webhook",
    "spawn_*",
    "cancel_spawn",
    "send_message",
    "voice_call",
    "speak",
    "notify",
    "home_assistant",
    "create_skill",
    "update_skill",
    "delete_skill",
    "patch_skill",
    "read_skill",
    "write_skill_files",
    "mcp_*",
];

/// Check whether a sender matches an entry in a privilege list.
///
/// Unlike [`crate::gating::is_allowed`], an **empty list matches nobody** —
/// "no operators configured" must never mean "everyone is an operator".
///
/// Matching is case-insensitive and supports `*` globs. Senders carrying a
/// host suffix (e.g. WhatsApp JIDs like `15551234567@s.whatsapp.net`, Matrix
/// IDs like `@alice:example.org`) also match on their user part, so operator
/// lists can use the same plain identifiers as allowlists.
#[must_use]
pub fn is_listed(sender_id: &str, list: &[String]) -> bool {
    if list.is_empty() || sender_id.is_empty() {
        return false;
    }
    if matches_any(sender_id, list) {
        return true;
    }
    // WhatsApp: "15551234567@s.whatsapp.net" → "15551234567".
    // Matrix:   "@alice:example.org" → "@alice".
    sender_id
        .split_once('@')
        .is_some_and(|(user, _)| !user.is_empty() && matches_any(user, list))
}

fn matches_any(sender_id: &str, list: &[String]) -> bool {
    let sender_lower = sender_id.to_lowercase();
    list.iter().any(|pattern| {
        let pat = pattern.to_lowercase();
        if pat.contains('*') {
            glob_match_lower(&pat, &sender_lower)
        } else {
            pat == sender_lower
        }
    })
}

/// Resolve a sender's privilege level for an account.
///
/// Precedence:
/// 1. `operators` — the explicit privileged-sender list, when non-empty.
/// 2. `allowlist` — fallback for accounts that predate `operators`. A non-empty
///    DM allowlist is the owner's own identity on a private instance, so it is
///    treated as the operator set. Group members who reach the bot through a
///    group/guild allowlist are *not* on it and stay guests.
/// 3. Otherwise every sender is a guest (fail-closed): an open account with no
///    allowlist grants nobody privileged access.
///
/// An absent `sender_id` (e.g. an unattributed button callback) is always a
/// guest — privilege is never granted to an unidentified sender.
#[must_use]
pub fn resolve_sender_role(
    sender_id: Option<&str>,
    operators: &[String],
    allowlist: &[String],
) -> ChannelSenderRole {
    let Some(sender_id) = sender_id.map(str::trim).filter(|s| !s.is_empty()) else {
        return ChannelSenderRole::Guest;
    };

    let list = if operators.is_empty() {
        allowlist
    } else {
        operators
    };

    if is_listed(sender_id, list) {
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
    fn exact_and_case_insensitive_match() {
        let ops = list(&["Alice", "400347514466992128"]);
        assert!(is_listed("alice", &ops));
        assert!(is_listed("ALICE", &ops));
        assert!(is_listed("400347514466992128", &ops));
        assert!(!is_listed("mallory", &ops));
    }

    #[test]
    fn glob_match() {
        let ops = list(&["admin_*"]);
        assert!(is_listed("admin_alice", &ops));
        assert!(!is_listed("user_bob", &ops));
    }

    #[test]
    fn matches_user_part_of_suffixed_id() {
        // WhatsApp JID against a plain phone number entry.
        assert!(is_listed(
            "15551234567@s.whatsapp.net",
            &list(&["15551234567"])
        ));
        // Matrix ID against a "@user" entry.
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
    fn operators_take_precedence_over_allowlist() {
        let operators = list(&["owner"]);
        let allowlist = list(&["owner", "helper"]);
        assert_eq!(
            resolve_sender_role(Some("owner"), &operators, &allowlist),
            ChannelSenderRole::Operator
        );
        // On the allowlist but not an operator → guest.
        assert_eq!(
            resolve_sender_role(Some("helper"), &operators, &allowlist),
            ChannelSenderRole::Guest
        );
    }

    #[test]
    fn falls_back_to_allowlist_when_no_operators() {
        let allowlist = list(&["owner"]);
        assert_eq!(
            resolve_sender_role(Some("owner"), &[], &allowlist),
            ChannelSenderRole::Operator
        );
        assert_eq!(
            resolve_sender_role(Some("guild_member"), &[], &allowlist),
            ChannelSenderRole::Guest
        );
    }

    #[test]
    fn fails_closed_with_no_lists() {
        // Open account (no allowlist, no operators): nobody is privileged.
        assert_eq!(
            resolve_sender_role(Some("anyone"), &[], &[]),
            ChannelSenderRole::Guest
        );
    }

    #[test]
    fn missing_or_blank_sender_is_guest() {
        let operators = list(&["owner"]);
        assert_eq!(
            resolve_sender_role(None, &operators, &[]),
            ChannelSenderRole::Guest
        );
        assert_eq!(
            resolve_sender_role(Some("   "), &operators, &[]),
            ChannelSenderRole::Guest
        );
    }

    #[test]
    fn wildcard_operator_entry_grants_everyone() {
        // Explicit opt-in to an open operator set is honoured — it must be
        // written deliberately, unlike an empty list.
        assert_eq!(
            resolve_sender_role(Some("anyone"), &list(&["*"]), &[]),
            ChannelSenderRole::Operator
        );
    }

    #[test]
    fn guest_denied_tools_cover_privileged_families() {
        for expected in [
            "exec",
            "process",
            "write_file",
            "memory_*",
            "sessions_*",
            "mcp_*",
            // Escalation path: this tool edits allowlists and the operator list.
            "update_channel_settings",
        ] {
            assert!(
                DEFAULT_GUEST_DENIED_TOOLS.contains(&expected),
                "{expected} must be denied to guests"
            );
        }
    }

    #[test]
    fn guest_denied_tools_leave_informational_tools_available() {
        for allowed in ["web_search", "web_fetch", "calc", "generate_image"] {
            assert!(
                !DEFAULT_GUEST_DENIED_TOOLS.contains(&allowed),
                "{allowed} should stay available to guests"
            );
        }
    }
}
