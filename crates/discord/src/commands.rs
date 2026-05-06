//! Native Discord slash command registration and handling.
//!
//! Registers global application commands (/new, /model, /help, etc.) when the
//! bot connects, and dispatches interactions through the same `dispatch_command`
//! path used by text-based `/` commands.

use {
    serenity::all::{
        Command, CommandDataOption, CommandDataOptionValue, CommandInteraction, CommandOptionType,
        ComponentInteraction, Context, CreateCommand, CreateCommandOption,
        CreateInteractionResponse, CreateInteractionResponseFollowup, EditInteractionResponse,
        Interaction,
    },
    tracing::{debug, info, warn},
};

use crate::state::AccountStateMap;

/// Build the set of global slash commands to register.
///
/// Derives from the centralized command registry in `moltis_channels::commands`.
pub fn build_commands() -> Vec<CreateCommand> {
    moltis_channels::commands::all_commands()
        .iter()
        .map(|c| {
            let mut cmd = CreateCommand::new(c.name).description(c.description);
            if let Some(arg) = &c.arg {
                let mut opt =
                    CreateCommandOption::new(CommandOptionType::String, arg.name, arg.description)
                        .required(arg.required);
                for &(label, value) in arg.choices {
                    opt = opt.add_string_choice(label, value);
                }
                cmd = cmd.add_option(opt);
            }
            cmd
        })
        .collect()
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

/// Handle an incoming interaction (slash command, button click, etc.).
pub async fn handle_interaction(
    ctx: &Context,
    interaction: &Interaction,
    account_id: &str,
    accounts: &AccountStateMap,
) {
    match interaction {
        Interaction::Command(command) => {
            handle_slash_command(ctx, command, account_id, accounts).await;
        },
        Interaction::Component(component) => {
            handle_component_interaction(ctx, component, account_id, accounts).await;
        },
        _ => {},
    }
}

/// Handle a slash command interaction.
async fn handle_slash_command(
    ctx: &Context,
    command: &CommandInteraction,
    account_id: &str,
    accounts: &AccountStateMap,
) {
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
        thread_id: None,
    };
    let sender_id = command.user.id.to_string();

    let command_text = build_command_text(&command.data.name, &command.data.options);

    let response_text = match sink
        .dispatch_command(&command_text, reply_to, Some(&sender_id))
        .await
    {
        Ok(response) => response,
        Err(e) => format!("Command failed: {e}"),
    };

    respond_ephemeral(ctx, command, &response_text).await;
}

/// Handle a component (button click) interaction.
async fn handle_component_interaction(
    ctx: &Context,
    component: &ComponentInteraction,
    account_id: &str,
    accounts: &AccountStateMap,
) {
    let callback_data = &component.data.custom_id;

    debug!(
        account_id,
        callback_data,
        user = %component.user.name,
        "Discord component interaction received"
    );

    // Acknowledge the interaction immediately so Discord doesn't show a failure.
    if let Err(e) = component
        .create_response(ctx, CreateInteractionResponse::Acknowledge)
        .await
    {
        warn!(
            account_id,
            callback_data, "Failed to acknowledge component interaction: {e}"
        );
        return;
    }

    let event_sink = {
        let accts = accounts.read().unwrap_or_else(|e| e.into_inner());
        accts.get(account_id).and_then(|s| s.event_sink.clone())
    };

    let Some(sink) = event_sink else {
        return;
    };

    let reply_to = moltis_channels::plugin::ChannelReplyTarget {
        channel_type: moltis_channels::ChannelType::Discord,
        account_id: account_id.to_string(),
        chat_id: component.channel_id.to_string(),
        message_id: None,
        thread_id: None,
    };

    match sink.dispatch_interaction(callback_data, reply_to).await {
        Ok(_response) => {
            // Response already sent by the gateway.
        },
        Err(e) => {
            debug!(
                account_id,
                callback_data, "interaction dispatch failed: {e}"
            );
        },
    }
}

/// Build the full command string from the slash command name and its options.
///
/// Discord slash commands pass arguments as structured options rather than
/// inline text. This reconstructs the `"name value"` format expected by
/// `dispatch_command`.
fn build_command_text(name: &str, options: &[CommandDataOption]) -> String {
    let arg = options.iter().find_map(|opt| match &opt.value {
        CommandDataOptionValue::String(s) => Some(s.as_str()),
        _ => None,
    });

    match arg {
        Some(value) if !value.is_empty() => format!("{name} {value}"),
        _ => name.to_string(),
    }
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
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn build_commands_matches_registry_count() {
        let commands = build_commands();
        let registry_count = moltis_channels::commands::all_commands().len();
        assert_eq!(
            commands.len(),
            registry_count,
            "build_commands should produce one command per registry entry"
        );
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
                name.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-'),
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
    fn all_registry_commands_present() {
        let commands = build_commands();
        let names: Vec<String> = commands
            .iter()
            .filter_map(|c| {
                serde_json::to_value(c)
                    .ok()
                    .and_then(|v| v["name"].as_str().map(String::from))
            })
            .collect();
        for cmd in moltis_channels::commands::all_commands() {
            assert!(
                names.contains(&cmd.name.to_string()),
                "missing slash command from registry: {}",
                cmd.name
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

    #[test]
    fn commands_with_arg_have_options() {
        let commands = build_commands();
        let registry = moltis_channels::commands::all_commands();

        for reg_cmd in registry {
            let json = commands
                .iter()
                .find_map(|c| {
                    let v = serde_json::to_value(c).ok()?;
                    if v["name"].as_str()? == reg_cmd.name {
                        Some(v)
                    } else {
                        None
                    }
                })
                .unwrap_or_else(|| panic!("missing built command: {}", reg_cmd.name));

            let options = json["options"].as_array();

            if let Some(arg) = &reg_cmd.arg {
                let opts = options.unwrap_or_else(|| {
                    panic!("command /{} has arg but no Discord options", reg_cmd.name)
                });
                assert!(
                    !opts.is_empty(),
                    "command /{} has arg but empty options array",
                    reg_cmd.name
                );
                let first = &opts[0];
                assert_eq!(
                    first["name"].as_str(),
                    Some(arg.name),
                    "command /{} option name should be \"{}\"",
                    reg_cmd.name,
                    arg.name,
                );
                // CommandOptionType::String == 3
                assert_eq!(
                    first["type"].as_u64(),
                    Some(3),
                    "command /{} option type should be String (3)",
                    reg_cmd.name
                );

                // Verify choices match
                if !arg.choices.is_empty() {
                    let json_choices = first["choices"].as_array().unwrap_or_else(|| {
                        panic!("command /{} has choices but none in JSON", reg_cmd.name)
                    });
                    assert_eq!(
                        json_choices.len(),
                        arg.choices.len(),
                        "command /{} choice count mismatch",
                        reg_cmd.name
                    );
                    for (json_choice, &(label, value)) in
                        json_choices.iter().zip(arg.choices.iter())
                    {
                        assert_eq!(json_choice["name"].as_str(), Some(label));
                        assert_eq!(json_choice["value"].as_str(), Some(value));
                    }
                }
            } else {
                let is_empty = options.is_none_or(|o| o.is_empty());
                assert!(
                    is_empty,
                    "command /{} has no arg but has Discord options",
                    reg_cmd.name
                );
            }
        }
    }

    fn string_option(value: &str) -> CommandDataOption {
        serde_json::from_value(serde_json::json!({
            "name": "value",
            "type": 3,
            "value": value,
        }))
        .expect("valid string option")
    }

    fn bool_option(value: bool) -> CommandDataOption {
        serde_json::from_value(serde_json::json!({
            "name": "value",
            "type": 5,
            "value": value,
        }))
        .expect("valid bool option")
    }

    #[test]
    fn build_command_text_no_options() {
        let text = build_command_text("new", &[]);
        assert_eq!(text, "new");
    }

    #[test]
    fn build_command_text_with_string_option() {
        let options = vec![string_option("2")];
        let text = build_command_text("mode", &options);
        assert_eq!(text, "mode 2");
    }

    #[test]
    fn build_command_text_with_empty_string_option() {
        let options = vec![string_option("")];
        let text = build_command_text("mode", &options);
        assert_eq!(text, "mode");
    }

    #[test]
    fn build_command_text_ignores_non_string_options() {
        let options = vec![bool_option(true)];
        let text = build_command_text("fast", &options);
        assert_eq!(text, "fast");
    }

    #[test]
    fn build_command_text_with_multi_word_arg() {
        let options = vec![string_option("provider:openai gpt-4o")];
        let text = build_command_text("model", &options);
        assert_eq!(text, "model provider:openai gpt-4o");
    }

    #[test]
    fn option_descriptions_within_discord_limit() {
        // Discord enforces a 100-character limit on option descriptions too.
        for cmd in moltis_channels::commands::all_commands() {
            if let Some(arg) = &cmd.arg {
                assert!(
                    arg.description.len() <= 100,
                    "command /{} arg description exceeds 100 chars ({} chars): {}",
                    cmd.name,
                    arg.description.len(),
                    arg.description,
                );
            }
        }
    }

    #[test]
    fn arg_names_are_valid_discord_option_names() {
        // Discord option names: lowercase, 1-32 chars, alphanumeric + hyphens.
        for cmd in moltis_channels::commands::all_commands() {
            if let Some(arg) = &cmd.arg {
                assert!(
                    !arg.name.is_empty() && arg.name.len() <= 32,
                    "command /{} arg name length out of range: {}",
                    cmd.name,
                    arg.name,
                );
                assert!(
                    arg.name.chars().all(|c| c.is_ascii_lowercase()
                        || c.is_ascii_digit()
                        || c == '-'
                        || c == '_'),
                    "command /{} arg name has invalid characters: {}",
                    cmd.name,
                    arg.name,
                );
            }
        }
    }

    #[test]
    fn choices_within_discord_limits() {
        // Discord allows max 25 choices, each name ≤ 100 chars, each value ≤ 100 chars.
        for cmd in moltis_channels::commands::all_commands() {
            if let Some(arg) = &cmd.arg {
                assert!(
                    arg.choices.len() <= 25,
                    "command /{} has {} choices (max 25)",
                    cmd.name,
                    arg.choices.len(),
                );
                for &(label, value) in arg.choices {
                    assert!(
                        label.len() <= 100,
                        "command /{} choice label too long: {label}",
                        cmd.name,
                    );
                    assert!(
                        value.len() <= 100,
                        "command /{} choice value too long: {value}",
                        cmd.name,
                    );
                }
            }
        }
    }
}
