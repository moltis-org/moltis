//! HOOK.md metadata parsing.
//!
//! Hook metadata is stored as TOML frontmatter in `HOOK.md` files:
//! ```text
//! +++
//! name = "my-hook"
//! description = "What it does"
//! events = ["BeforeToolCall"]
//! command = "./handler.sh"
//! timeout = 5
//!
//! [requires]
//! os = ["darwin", "linux"]
//! bins = ["jq"]
//! env = ["SLACK_WEBHOOK_URL"]
//! +++
//!
//! # My Hook
//! Extended docs go here.
//! ```

use std::{collections::HashMap, path::Path};

use {
    anyhow::{Context, Result, bail},
    serde::{Deserialize, Serialize},
};

use moltis_common::hooks::HookEvent;

/// Requirements that must be met for a hook to be eligible.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HookRequirements {
    #[serde(default)]
    pub os: Vec<String>,
    #[serde(default)]
    pub bins: Vec<String>,
    #[serde(default)]
    pub env: Vec<String>,
    #[serde(default)]
    pub config: Vec<String>,
}

/// Metadata parsed from a HOOK.md file's TOML frontmatter.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookMetadata {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub emoji: Option<String>,
    pub events: Vec<HookEvent>,
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default = "default_timeout")]
    pub timeout: u64,
    #[serde(default)]
    pub priority: i32,
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default)]
    pub requires: HookRequirements,
}

fn default_timeout() -> u64 {
    10
}

/// Full parsed hook including metadata and the body text.
#[derive(Debug, Clone)]
pub struct ParsedHook {
    pub metadata: HookMetadata,
    pub body: String,
    pub source_path: std::path::PathBuf,
}

/// Parse a HOOK.md file content into metadata + body.
///
/// Expects TOML frontmatter delimited by `+++` lines.
pub fn parse_hook_md(content: &str, source_path: &Path) -> Result<ParsedHook> {
    let trimmed = content.trim_start();
    if !trimmed.starts_with("+++") {
        bail!("HOOK.md must start with +++ TOML frontmatter");
    }

    // Find the closing +++
    let after_first = &trimmed[3..];
    let end = after_first
        .find("\n+++")
        .context("missing closing +++ in HOOK.md frontmatter")?;

    let toml_str = after_first[..end].trim();
    let body_start = end + 4; // skip "\n+++"
    let body = after_first
        .get(body_start..)
        .map(|s| s.trim().to_string())
        .unwrap_or_default();

    let metadata: HookMetadata =
        toml::from_str(toml_str).context("failed to parse HOOK.md TOML frontmatter")?;

    Ok(ParsedHook {
        metadata,
        body,
        source_path: source_path.to_path_buf(),
    })
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_full_hook_md() {
        let content = r#"+++
name = "test-hook"
description = "A test hook"
emoji = "🔧"
events = ["BeforeToolCall", "AfterToolCall"]
command = "./handler.sh"
timeout = 5
priority = 10

[requires]
os = ["darwin"]
bins = ["jq"]
env = ["MY_VAR"]
+++

# Test Hook

This is the body.
"#;
        let parsed = parse_hook_md(content, Path::new("/tmp/test")).unwrap();
        assert_eq!(parsed.metadata.name, "test-hook");
        assert_eq!(parsed.metadata.description, "A test hook");
        assert_eq!(parsed.metadata.emoji.as_deref(), Some("🔧"));
        assert_eq!(parsed.metadata.events.len(), 2);
        assert_eq!(parsed.metadata.command.as_deref(), Some("./handler.sh"));
        assert_eq!(parsed.metadata.timeout, 5);
        assert_eq!(parsed.metadata.priority, 10);
        assert_eq!(parsed.metadata.requires.os, vec!["darwin"]);
        assert_eq!(parsed.metadata.requires.bins, vec!["jq"]);
        assert_eq!(parsed.metadata.requires.env, vec!["MY_VAR"]);
        assert!(parsed.body.contains("# Test Hook"));
    }

    #[test]
    fn parse_minimal_hook_md() {
        let content = r#"+++
name = "minimal"
events = ["SessionStart"]
+++
"#;
        let parsed = parse_hook_md(content, Path::new("/tmp/minimal")).unwrap();
        assert_eq!(parsed.metadata.name, "minimal");
        assert_eq!(parsed.metadata.timeout, 10); // default
        assert_eq!(parsed.metadata.priority, 0); // default
        assert!(parsed.metadata.command.is_none());
        assert!(parsed.body.is_empty());
    }

    #[test]
    fn parse_missing_frontmatter_fails() {
        let content = "# No frontmatter here";
        assert!(parse_hook_md(content, Path::new("/tmp/bad")).is_err());
    }

    #[test]
    fn parse_unclosed_frontmatter_fails() {
        let content = "+++\nname = \"bad\"\nevents = []\n";
        assert!(parse_hook_md(content, Path::new("/tmp/bad")).is_err());
    }
}
