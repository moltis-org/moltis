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
    /// Hint shown in the Slack command input (e.g. `"[model name]"`).
    pub usage_hint: String,
}

/// Returns the list of channel control commands.
///
/// Derives from the centralized registry in `moltis_channels::commands`.
pub fn command_definitions() -> Vec<SlackCommandDef> {
    moltis_channels::commands::all_commands()
        .iter()
        .map(|c| {
            let usage_hint = match &c.arg {
                Some(arg) if !arg.choices.is_empty() => {
                    // Show choices inline: "[on | off | exit | status]"
                    let options: Vec<&str> = arg.choices.iter().map(|&(_, v)| v).collect();
                    format!("[{}]", options.join(" | "))
                },
                Some(arg) => format!("[{}]", arg.description),
                None => String::new(),
            };
            SlackCommandDef {
                name: c.name,
                description: c.description,
                usage_hint,
            }
        })
        .collect()
}

/// Generate a Slack app manifest YAML snippet for slash commands.
///
/// The output should be pasted into the `features.slash_commands` section
/// of a Slack app manifest.
pub fn generate_manifest_snippet(request_url_base: &str) -> String {
    let mut yaml = String::from("slash_commands:\n");
    for cmd in command_definitions() {
        yaml.push_str(&format!(
            "  - command: /{}\n    url: {}/api/channels/slack/{{{{account_id}}}}/commands\n    description: \"{}\"\n    usage_hint: \"{}\"\n    should_escape: false\n",
            cmd.name, request_url_base, cmd.description, cmd.usage_hint,
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
                "usage_hint": cmd.usage_hint,
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
    fn manifest_snippet_includes_usage_hints() {
        let snippet = generate_manifest_snippet("https://example.com");
        // /sh has choices, so usage_hint should list them.
        assert!(
            snippet.contains("usage_hint: \"[on | off | exit | status]\""),
            "manifest should include /sh choices in usage_hint"
        );
        // /fast also has choices.
        assert!(
            snippet.contains("usage_hint: \"[on | off]\""),
            "manifest should include /fast choices in usage_hint"
        );
        // /model has a free-form arg, so usage_hint should show description.
        assert!(
            snippet.contains("usage_hint: \"[Model name or provider:model]\""),
            "manifest should include /model arg description in usage_hint"
        );
        // /new has no arg, so usage_hint should be empty.
        assert!(
            snippet.contains("command: /new\n") && snippet.contains("usage_hint: \"\"\n"),
            "manifest should have empty usage_hint for /new"
        );
    }

    #[test]
    fn command_definitions_json_structure() {
        let json = command_definitions_json();
        let arr = json.as_array().unwrap();
        assert!(!arr.is_empty());
        for item in arr {
            assert!(item.get("name").unwrap().is_string());
            assert!(item.get("description").unwrap().is_string());
            assert!(item.get("usage_hint").unwrap().is_string());
        }
    }
}
