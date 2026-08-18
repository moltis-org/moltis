//! Centralized channel command registry.
//!
//! Every channel (Telegram, Discord, Slack, Matrix, Nostr, etc.) derives its
//! command interception, help text, and platform registration from this single
//! source of truth. Adding a command here automatically propagates to all
//! channels.

/// A single argument that a command accepts.
#[derive(Debug, Clone, Copy)]
pub struct CommandArg {
    /// Semantic name for the argument (e.g. `"model"`, `"mode"`, `"id"`).
    ///
    /// Used as the Discord option name and shown in platform UIs.
    pub name: &'static str,
    /// Short description shown next to the argument in platform UIs.
    pub description: &'static str,
    /// Fixed choices the user can pick from (label, value).
    ///
    /// Platforms like Discord render these as a dropdown. An empty slice means
    /// free-form input.
    pub choices: &'static [(&'static str, &'static str)],
    /// Whether the argument must be provided.
    ///
    /// When `true`, platforms like Discord enforce the argument at the UI level
    /// and won't allow bare invocation.
    pub required: bool,
}

/// A channel command definition.
#[derive(Debug, Clone, Copy)]
pub struct CommandDef {
    /// The command name without the leading `/`.
    pub name: &'static str,
    /// Short description shown in help text and platform autocomplete.
    pub description: &'static str,
    /// If set, the command accepts a single string argument.
    pub arg: Option<CommandArg>,
}

/// Authorization required before a channel command reaches its handler.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandPrivilege {
    Public,
    OperatorDirect,
}

impl CommandDef {
    /// Commands default to operator direct-chat only. Adding a new command
    /// therefore fails closed until it is deliberately reviewed and listed as
    /// public here.
    ///
    /// A command is public only when the worst a hostile sender can do with it
    /// is disrupt the conversation in the room they are already in — something
    /// any member can do by talking. Anything that reaches the host, acts on
    /// the owner's behalf, or reads state from outside this room is operator
    /// direct-chat only:
    ///
    /// - `sh` runs host commands; `update` replaces the running binary.
    /// - `approve`/`deny` act on the *owner's* pending exec requests, so a
    ///   guest approving one is arbitrary code execution by proxy. `approvals`
    ///   lists the command lines awaiting approval.
    /// - `sandbox` can turn the sandbox off, moving execution onto the host.
    /// - `attach` pulls an existing session into this chat, and
    ///   `sessions`/`context`/`insights`/`peek`/`btw` read session, prompt, or
    ///   cross-session state the room's participants are not entitled to.
    /// - `rollback` restores a checkpoint; `agent` swaps the system prompt and
    ///   tool posture; `steer`/`queue` inject into a run someone else started.
    #[must_use]
    pub fn privilege(self) -> CommandPrivilege {
        match self.name {
            // Help lists the commands and marks which need an operator.
            "help"
            // Scoped to this chat's own session: start it over, clear it,
            // summarize it, retitle it, branch it, or abort its current run.
            | "new" | "clear" | "compact" | "title" | "fork" | "stop"
            // Which model/mode answers in this chat. Reversible, and bounded by
            // the account's own provider configuration.
            | "model" | "mode" | "fast" => CommandPrivilege::Public,
            _ => CommandPrivilege::OperatorDirect,
        }
    }
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
            arg: None,
        },
        CommandDef {
            name: "sessions",
            description: "List and switch sessions",
            arg: Some(CommandArg {
                name: "id",
                description: "Session ID to switch to",
                choices: &[],
                required: false,
            }),
        },
        CommandDef {
            name: "attach",
            description: "Attach an existing session here",
            arg: Some(CommandArg {
                name: "id",
                description: "Session ID to attach",
                choices: &[],
                required: true,
            }),
        },
        CommandDef {
            name: "fork",
            description: "Fork this session into a new branch",
            arg: Some(CommandArg {
                name: "title",
                description: "Optional title for the fork",
                choices: &[],
                required: false,
            }),
        },
        CommandDef {
            name: "clear",
            description: "Clear session history",
            arg: None,
        },
        CommandDef {
            name: "compact",
            description: "Compact session (summarize)",
            arg: None,
        },
        CommandDef {
            name: "title",
            description: "Auto-generate session title",
            arg: None,
        },
        CommandDef {
            name: "context",
            description: "Show session context info",
            arg: None,
        },
        // Control
        CommandDef {
            name: "approvals",
            description: "List pending exec approvals",
            arg: None,
        },
        CommandDef {
            name: "approve",
            description: "Approve a pending exec request",
            arg: Some(CommandArg {
                name: "id",
                description: "Approval ID",
                choices: &[],
                required: true,
            }),
        },
        CommandDef {
            name: "deny",
            description: "Deny a pending exec request",
            arg: Some(CommandArg {
                name: "id",
                description: "Approval ID",
                choices: &[],
                required: true,
            }),
        },
        CommandDef {
            name: "agent",
            description: "Switch session agent",
            arg: Some(CommandArg {
                name: "name",
                description: "Agent name",
                choices: &[],
                required: false,
            }),
        },
        CommandDef {
            name: "mode",
            description: "Switch session mode",
            arg: Some(CommandArg {
                name: "mode",
                description: "Mode number or name",
                choices: &[],
                required: false,
            }),
        },
        CommandDef {
            name: "model",
            description: "Switch provider/model",
            arg: Some(CommandArg {
                name: "model",
                description: "Model name or provider:model",
                choices: &[],
                required: false,
            }),
        },
        CommandDef {
            name: "sandbox",
            description: "Toggle sandbox and choose image",
            arg: Some(CommandArg {
                name: "action",
                description: "Sandbox action",
                choices: &[("Toggle", "toggle"), ("On", "on"), ("Off", "off")],
                required: false,
            }),
        },
        CommandDef {
            name: "sh",
            description: "Enable command mode (/sh off to exit)",
            arg: Some(CommandArg {
                name: "action",
                description: "Command mode action",
                choices: &[
                    ("On", "on"),
                    ("Off", "off"),
                    ("Exit", "exit"),
                    ("Status", "status"),
                ],
                required: false,
            }),
        },
        CommandDef {
            name: "stop",
            description: "Abort the current running agent",
            arg: None,
        },
        CommandDef {
            name: "peek",
            description: "Show current thinking/tool status",
            arg: None,
        },
        CommandDef {
            name: "update",
            description: "Update moltis to latest or specified version",
            arg: Some(CommandArg {
                name: "version",
                description: "Version to update to",
                choices: &[],
                required: false,
            }),
        },
        CommandDef {
            name: "rollback",
            description: "List or restore file checkpoints",
            arg: Some(CommandArg {
                name: "id",
                description: "Checkpoint ID to restore",
                choices: &[],
                required: false,
            }),
        },
        // Quick actions
        CommandDef {
            name: "btw",
            description: "Quick side question (no tools, not persisted)",
            arg: Some(CommandArg {
                name: "question",
                description: "Your question",
                choices: &[],
                required: true,
            }),
        },
        CommandDef {
            name: "fast",
            description: "Toggle fast/priority mode",
            arg: Some(CommandArg {
                name: "toggle",
                description: "Enable or disable",
                choices: &[("On", "on"), ("Off", "off")],
                required: false,
            }),
        },
        CommandDef {
            name: "insights",
            description: "Show session analytics and usage stats",
            arg: Some(CommandArg {
                name: "scope",
                description: "Scope or filter",
                choices: &[],
                required: false,
            }),
        },
        CommandDef {
            name: "steer",
            description: "Inject guidance into the current agent run",
            arg: Some(CommandArg {
                name: "guidance",
                description: "Guidance text",
                choices: &[],
                required: true,
            }),
        },
        CommandDef {
            name: "queue",
            description: "Queue a message for the next agent turn",
            arg: Some(CommandArg {
                name: "message",
                description: "Message to queue",
                choices: &[],
                required: true,
            }),
        },
        // Meta
        CommandDef {
            name: "help",
            description: "Show available commands",
            arg: None,
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
        let operator = matches!(cmd.privilege(), CommandPrivilege::OperatorDirect)
            .then_some(" (operator DM only)")
            .unwrap_or_default();
        lines.push(format!("/{} — {}{operator}", cmd.name, cmd.description));
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

    /// The public set is small and enumerated here on purpose. Adding a command
    /// to it is a security decision, so it must be made in both places or this
    /// fails — a new command is never public by accident.
    #[test]
    fn only_reviewed_commands_are_public() {
        const PUBLIC: &[&str] = &[
            "help", "new", "clear", "compact", "title", "fork", "stop", "model", "mode", "fast",
        ];

        for command in all_commands() {
            let expected = if PUBLIC.contains(&command.name) {
                CommandPrivilege::Public
            } else {
                CommandPrivilege::OperatorDirect
            };
            assert_eq!(
                command.privilege(),
                expected,
                "unexpected privilege for /{}",
                command.name
            );
        }
    }

    /// The commands that reach the host, act for the owner, or read state from
    /// outside the current chat must never become public.
    #[test]
    fn host_and_owner_scoped_commands_stay_operator_only() {
        for name in [
            "sh",
            "update",
            "approve",
            "deny",
            "approvals",
            "sandbox",
            "attach",
            "sessions",
            "context",
            "insights",
            "peek",
            "btw",
            "rollback",
            "agent",
            "steer",
            "queue",
        ] {
            let command = all_commands()
                .iter()
                .find(|candidate| candidate.name == name)
                .copied()
                .unwrap_or_else(|| panic!("/{name} is missing from the command registry"));
            assert_eq!(
                command.privilege(),
                CommandPrivilege::OperatorDirect,
                "/{name} must stay restricted to operator direct chats"
            );
        }
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
