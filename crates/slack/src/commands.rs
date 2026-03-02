//! Slack slash command manifest generation.
//!
//! Slack does not support programmatic command registration (unlike Discord).
//! Commands must be configured in the Slack app manifest. This module
//! generates the manifest snippet that users paste into their Slack app config.

/// A slash command definition.
#[derive(Debug, Clone)]
pub struct SlackCommandDef {
    pub name: &'static str,
    pub description: &'static str,
}

/// Returns the list of channel control commands.
pub fn command_definitions() -> Vec<SlackCommandDef> {
    vec![
        SlackCommandDef {
            name: "new",
            description: "Start a new conversation session",
        },
        SlackCommandDef {
            name: "clear",
            description: "Clear the current session history",
        },
        SlackCommandDef {
            name: "compact",
            description: "Compact the current session to save context",
        },
        SlackCommandDef {
            name: "context",
            description: "Show the current session context usage",
        },
        SlackCommandDef {
            name: "model",
            description: "View or change the AI model",
        },
        SlackCommandDef {
            name: "sessions",
            description: "List and switch between sessions",
        },
        SlackCommandDef {
            name: "agent",
            description: "View or switch the active agent persona",
        },
        SlackCommandDef {
            name: "help",
            description: "Show available commands",
        },
    ]
}

/// Generate a Slack app manifest YAML snippet for slash commands.
///
/// The output should be pasted into the `features.slash_commands` section
/// of a Slack app manifest.
pub fn generate_manifest_snippet(request_url_base: &str) -> String {
    let mut yaml = String::from("slash_commands:\n");
    for cmd in command_definitions() {
        yaml.push_str(&format!(
            "  - command: /{}\n    url: {}/api/channels/slack/{{{{account_id}}}}/commands\n    description: \"{}\"\n    usage_hint: \"\"\n    should_escape: false\n",
            cmd.name, request_url_base, cmd.description,
        ));
    }
    yaml
}

/// Generate a JSON array of command definitions for API responses.
pub fn command_definitions_json() -> serde_json::Value {
    let cmds: Vec<serde_json::Value> = command_definitions()
        .into_iter()
        .map(|cmd| {
            serde_json::json!({
                "name": cmd.name,
                "description": cmd.description,
            })
        })
        .collect();
    serde_json::Value::Array(cmds)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn command_definitions_not_empty() {
        let defs = command_definitions();
        assert!(!defs.is_empty());
        assert!(defs.iter().any(|c| c.name == "new"));
        assert!(defs.iter().any(|c| c.name == "model"));
        assert!(defs.iter().any(|c| c.name == "help"));
    }

    #[test]
    fn manifest_snippet_contains_commands() {
        let snippet = generate_manifest_snippet("https://example.com");
        assert!(snippet.contains("command: /new"));
        assert!(snippet.contains("command: /model"));
        assert!(snippet.contains("command: /help"));
        assert!(snippet.contains("https://example.com/api/channels/slack/"));
    }

    #[test]
    fn command_definitions_json_structure() {
        let json = command_definitions_json();
        let arr = json.as_array().unwrap();
        assert!(!arr.is_empty());
        for item in arr {
            assert!(item.get("name").unwrap().is_string());
            assert!(item.get("description").unwrap().is_string());
        }
    }
}
