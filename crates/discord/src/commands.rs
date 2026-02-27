//! Native Discord slash command registration and handling.
//!
//! Registers global application commands (/new, /model, /help, etc.) when the
//! bot connects, and dispatches interactions through the same `dispatch_command`
//! path used by text-based `/` commands.

use {
    serenity::all::{
        Command, CommandInteraction, Context, CreateCommand, CreateInteractionResponseFollowup,
        EditInteractionResponse, Interaction,
    },
    tracing::{debug, info, warn},
};

use crate::state::AccountStateMap;

/// Build the set of global slash commands to register.
pub fn build_commands() -> Vec<CreateCommand> {
    vec![
        CreateCommand::new("new").description("Start a new chat session"),
        CreateCommand::new("clear").description("Clear the current session history"),
        CreateCommand::new("compact").description("Summarize the current session"),
        CreateCommand::new("context").description("Show session info (model, tokens, plugins)"),
        CreateCommand::new("model").description("List or switch the AI model"),
        CreateCommand::new("sessions").description("List or switch chat sessions"),
        CreateCommand::new("agent").description("List or switch agents"),
        CreateCommand::new("help").description("Show available commands"),
    ]
}

/// Register global slash commands for the bot.
pub async fn register_global_commands(ctx: &Context, account_id: &str) {
    match Command::set_global_commands(&ctx, build_commands()).await {
        Ok(commands) => {
            let names: Vec<&str> = commands.iter().map(|c| c.name.as_str()).collect();
            info!(
                account_id,
                commands = ?names,
                "Registered {} Discord slash commands",
                commands.len()
            );
        },
        Err(e) => {
            warn!(account_id, "Failed to register Discord slash commands: {e}");
        },
    }
}

/// Handle an incoming interaction (slash command, autocomplete, etc.).
pub async fn handle_interaction(
    ctx: &Context,
    interaction: &Interaction,
    account_id: &str,
    accounts: &AccountStateMap,
) {
    let Interaction::Command(command) = interaction else {
        return;
    };

    debug!(
        account_id,
        command = %command.data.name,
        user = %command.user.name,
        "Discord slash command received"
    );

    if let Err(e) = command.defer_ephemeral(ctx).await {
        warn!(
            command = %command.data.name,
            "Failed to acknowledge slash command: {e}"
        );
        return;
    }

    let event_sink = {
        let accts = accounts.read().unwrap_or_else(|e| e.into_inner());
        accts.get(account_id).and_then(|s| s.event_sink.clone())
    };

    let Some(sink) = event_sink else {
        respond_ephemeral(ctx, command, "Bot is not ready yet.").await;
        return;
    };

    let reply_to = moltis_channels::plugin::ChannelReplyTarget {
        channel_type: moltis_channels::ChannelType::Discord,
        account_id: account_id.to_string(),
        chat_id: command.channel_id.to_string(),
        message_id: None,
    };

    let response_text = match sink.dispatch_command(&command.data.name, reply_to).await {
        Ok(response) => response,
        Err(e) => format!("Command failed: {e}"),
    };

    respond_ephemeral(ctx, command, &response_text).await;
}

/// Send an ephemeral response to a slash command (only visible to the invoker).
async fn respond_ephemeral(ctx: &Context, command: &CommandInteraction, text: &str) {
    if let Err(e) = command
        .edit_response(&ctx, EditInteractionResponse::new().content(text))
        .await
    {
        warn!(
            command = %command.data.name,
            "Failed to edit deferred slash response: {e}"
        );
        if let Err(followup_err) = command
            .create_followup(
                &ctx,
                CreateInteractionResponseFollowup::new()
                    .content(text)
                    .ephemeral(true),
            )
            .await
        {
            warn!(
                command = %command.data.name,
                "Failed to send slash follow-up response: {followup_err}"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_commands_returns_expected_count() {
        let commands = build_commands();
        assert_eq!(commands.len(), 8, "expected 8 slash commands");
    }

    #[test]
    fn build_commands_serializes_to_valid_json() {
        let commands = build_commands();
        // Each CreateCommand should serialize successfully (validates structure).
        for cmd in &commands {
            let json = serde_json::to_value(cmd)
                .unwrap_or_else(|e| panic!("failed to serialize command: {e}"));
            // Verify name field is present and non-empty.
            let name = json["name"].as_str().unwrap_or_default();
            assert!(!name.is_empty(), "command name is empty");
            assert!(
                name.chars().all(|c| c.is_ascii_lowercase() || c == '_'),
                "invalid command name: {name}"
            );
            assert!(name.len() <= 32, "command name too long: {name}");
            // Verify description is present.
            let desc = json["description"].as_str().unwrap_or_default();
            assert!(!desc.is_empty(), "command {name} has empty description");
        }
    }

    #[test]
    fn no_duplicate_command_names() {
        let commands = build_commands();
        let mut names: Vec<String> = commands
            .iter()
            .filter_map(|c| {
                serde_json::to_value(c)
                    .ok()
                    .and_then(|v| v["name"].as_str().map(String::from))
            })
            .collect();
        let original_len = names.len();
        names.sort();
        names.dedup();
        assert_eq!(
            names.len(),
            original_len,
            "duplicate slash command names found"
        );
    }

    #[test]
    fn expected_command_names_present() {
        let commands = build_commands();
        let names: Vec<String> = commands
            .iter()
            .filter_map(|c| {
                serde_json::to_value(c)
                    .ok()
                    .and_then(|v| v["name"].as_str().map(String::from))
            })
            .collect();
        for expected in [
            "new", "clear", "compact", "context", "model", "sessions", "agent", "help",
        ] {
            assert!(
                names.contains(&expected.to_string()),
                "missing expected slash command: {expected}"
            );
        }
    }

    #[test]
    fn descriptions_within_discord_limit() {
        // Discord enforces a 100-character limit on command descriptions.
        let commands = build_commands();
        for cmd in &commands {
            let json = serde_json::to_value(cmd)
                .unwrap_or_else(|e| panic!("failed to serialize command: {e}"));
            let name = json["name"].as_str().unwrap_or("unknown");
            let desc = json["description"].as_str().unwrap_or_default();
            assert!(
                desc.len() <= 100,
                "command {name} description exceeds 100 chars ({} chars): {desc}",
                desc.len()
            );
        }
    }

    #[test]
    fn command_names_are_lowercase_alphanumeric() {
        // Discord requires command names to be lowercase with no spaces.
        let commands = build_commands();
        for cmd in &commands {
            let json = serde_json::to_value(cmd)
                .unwrap_or_else(|e| panic!("failed to serialize command: {e}"));
            let name = json["name"].as_str().unwrap_or_default();
            assert!(
                !name.is_empty() && name.len() <= 32,
                "command name length out of range: {name}"
            );
            assert!(
                name.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-'),
                "command name contains invalid characters: {name}"
            );
        }
    }
}
