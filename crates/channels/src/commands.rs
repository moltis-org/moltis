//! Centralized channel command registry.
//!
//! Every channel (Telegram, Discord, Slack, Matrix, Nostr, etc.) derives its
//! command interception, help text, and platform registration from this single
//! source of truth. Adding a command here automatically propagates to all
//! channels.

/// A channel command definition.
#[derive(Debug, Clone, Copy)]
pub struct CommandDef {
    /// The command name without the leading `/`.
    pub name: &'static str,
    /// Short description shown in help text and platform autocomplete.
    pub description: &'static str,
}

/// The single source of truth for all channel commands.
///
/// Order determines display order in help text and platform menus.
/// Every command listed here (except `"help"`) must have a matching arm in
/// `dispatch_command` in the gateway.
pub fn all_commands() -> &'static [CommandDef] {
    &[
        // Session management
        CommandDef {
            name: "new",
            description: "Start a new session",
        },
        CommandDef {
            name: "sessions",
            description: "List and switch sessions",
        },
        CommandDef {
            name: "attach",
            description: "Attach an existing session here",
        },
        CommandDef {
            name: "fork",
            description: "Fork this session into a new branch",
        },
        CommandDef {
            name: "clear",
            description: "Clear session history",
        },
        CommandDef {
            name: "compact",
            description: "Compact session (summarize)",
        },
        CommandDef {
            name: "title",
            description: "Auto-generate session title",
        },
        CommandDef {
            name: "context",
            description: "Show session context info",
        },
        // Control
        CommandDef {
            name: "approvals",
            description: "List pending exec approvals",
        },
        CommandDef {
            name: "approve",
            description: "Approve a pending exec request",
        },
        CommandDef {
            name: "deny",
            description: "Deny a pending exec request",
        },
        CommandDef {
            name: "agent",
            description: "Switch session agent",
        },
        CommandDef {
            name: "mode",
            description: "Switch session mode",
        },
        CommandDef {
            name: "model",
            description: "Switch provider/model",
        },
        CommandDef {
            name: "sandbox",
            description: "Toggle sandbox and choose image",
        },
        CommandDef {
            name: "sh",
            description: "Enable command mode (/sh off to exit)",
        },
        CommandDef {
            name: "stop",
            description: "Abort the current running agent",
        },
        CommandDef {
            name: "peek",
            description: "Show current thinking/tool status",
        },
        CommandDef {
            name: "update",
            description: "Update moltis to latest or specified version",
        },
        CommandDef {
            name: "rollback",
            description: "List or restore file checkpoints",
        },
        // Quick actions
        CommandDef {
            name: "btw",
            description: "Quick side question (no tools, not persisted)",
        },
        CommandDef {
            name: "fast",
            description: "Toggle fast/priority mode",
        },
        CommandDef {
            name: "insights",
            description: "Show session analytics and usage stats",
        },
        CommandDef {
            name: "steer",
            description: "Inject guidance into the current agent run",
        },
        CommandDef {
            name: "queue",
            description: "Queue a message for the next agent turn",
        },
        // Meta
        CommandDef {
            name: "help",
            description: "Show available commands",
        },
    ]
}

/// Whether a given command name is a known channel command.
///
/// Handles the `/sh` special case: only intercepts toggle sub-commands
/// (empty, `"on"`, `"off"`, `"exit"`, `"status"`), not arbitrary shell input
/// like `/sh ls -la`.
pub fn is_channel_command(cmd: &str, full_text: &str) -> bool {
    if cmd == "sh" {
        let args = full_text.strip_prefix("sh").unwrap_or("").trim();
        return args.is_empty() || matches!(args, "on" | "off" | "exit" | "status");
    }

    all_commands().iter().any(|c| c.name == cmd)
}

/// Generate help text (one line per command: `/name — description`).
pub fn help_text() -> String {
    let mut lines = Vec::with_capacity(all_commands().len() + 1);
    lines.push("Available commands:".to_string());
    for cmd in all_commands() {
        lines.push(format!("/{} — {}", cmd.name, cmd.description));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_commands_not_empty() {
        assert!(!all_commands().is_empty());
    }

    #[test]
    fn no_duplicate_names() {
        let names: Vec<&str> = all_commands().iter().map(|c| c.name).collect();
        let mut deduped = names.clone();
        deduped.sort();
        deduped.dedup();
        assert_eq!(names.len(), deduped.len(), "duplicate command names found");
    }

    #[test]
    fn help_is_in_list() {
        assert!(
            all_commands().iter().any(|c| c.name == "help"),
            "help command missing from registry"
        );
    }

    #[test]
    fn is_channel_command_basic() {
        assert!(is_channel_command("new", "new"));
        assert!(is_channel_command("stop", "stop"));
        assert!(is_channel_command("peek", "peek"));
        assert!(is_channel_command("help", "help"));
        assert!(is_channel_command("model", "model gpt-4"));
        assert!(!is_channel_command("unknown", "unknown"));
    }

    #[test]
    fn is_channel_command_sh_special_case() {
        // Toggle sub-commands should be intercepted.
        assert!(is_channel_command("sh", "sh"));
        assert!(is_channel_command("sh", "sh on"));
        assert!(is_channel_command("sh", "sh off"));
        assert!(is_channel_command("sh", "sh exit"));
        assert!(is_channel_command("sh", "sh status"));
        // Arbitrary shell input should NOT be intercepted.
        assert!(!is_channel_command("sh", "sh ls -la"));
        assert!(!is_channel_command("sh", "sh echo hello"));
    }

    #[test]
    fn help_text_contains_all_commands() {
        let text = help_text();
        for cmd in all_commands() {
            assert!(
                text.contains(&format!("/{}", cmd.name)),
                "help text missing command: /{}",
                cmd.name
            );
        }
    }

    #[test]
    fn expected_commands_present() {
        let names: Vec<&str> = all_commands().iter().map(|c| c.name).collect();
        for expected in [
            "new",
            "fork",
            "clear",
            "compact",
            "title",
            "context",
            "sessions",
            "attach",
            "approvals",
            "approve",
            "deny",
            "agent",
            "mode",
            "model",
            "sandbox",
            "sh",
            "stop",
            "peek",
            "update",
            "rollback",
            "btw",
            "fast",
            "insights",
            "steer",
            "queue",
            "help",
        ] {
            assert!(
                names.contains(&expected),
                "missing expected command: {expected}"
            );
        }
    }
}
