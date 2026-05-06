//! Post-install recipes for featured skill repositories.
//!
//! When a user installs a featured repo, the recipe provides structured
//! instructions the agent can execute — registering MCP servers, prompting
//! for configuration, enabling skills, etc.
//!
//! Recipes are bundled data (not code). The agent reads them and uses the
//! existing `mcp.*` and `skills.*` RPC methods to carry out the steps.

use std::sync::LazyLock;

use serde::{Deserialize, Serialize};

// ── Recipe types ────────────────────────────────────────────────────────────

/// A post-install recipe for a featured skill repository.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PostInstallRecipe {
    /// The repo source identifier (e.g. `"garrytan/gbrain"`).
    pub source: String,
    /// Human-readable title for the recipe.
    pub title: String,
    /// Markdown instructions the agent should follow after installation.
    pub instructions: String,
    /// MCP servers to register as part of setup.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mcp_servers: Vec<McpRecipeServer>,
    /// Skills to enable after installation.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skills_to_enable: Vec<String>,
}

/// An MCP server that a recipe wants to register.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpRecipeServer {
    /// Server name for `mcp-servers.json`.
    pub name: String,
    /// Transport type: `"stdio"`, `"sse"`, or `"streamable-http"`.
    #[serde(default = "default_stdio")]
    pub transport: String,
    /// Command to run (stdio transport).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// Command arguments.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    /// URL for remote transports (SSE, streamable-http).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Environment variable names the user needs to provide.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub env_keys: Vec<String>,
    /// Human-readable display name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// Hint shown to the user about what this server provides.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

fn default_stdio() -> String {
    "stdio".to_string()
}

// ── Public API ──────────────────────────────────────────────────────────────

/// Look up a bundled post-install recipe by repo source identifier.
///
/// Source can be `"owner/repo"` or a full GitHub URL — we normalise to
/// `"owner/repo"` before matching.
pub fn get_recipe(source: &str) -> Option<&'static PostInstallRecipe> {
    let key = normalise_source(source);
    RECIPES.iter().find(|r| r.source == key)
}

/// List all bundled post-install recipes.
pub fn list_recipes() -> &'static [PostInstallRecipe] {
    &RECIPES
}

/// Check if a given repo source has a bundled post-install recipe.
pub fn has_recipe(source: &str) -> bool {
    let key = normalise_source(source);
    RECIPES.iter().any(|r| r.source == key)
}

fn normalise_source(source: &str) -> String {
    let trimmed = source.trim();
    let stripped = trimmed
        .strip_prefix("https://github.com/")
        .or_else(|| trimmed.strip_prefix("http://github.com/"))
        .unwrap_or(trimmed)
        .trim_end_matches('/');
    stripped
        .strip_suffix(".git")
        .unwrap_or(stripped)
        .to_lowercase()
}

// ── Static recipe definitions ───────────────────────────────────────────────

static RECIPES: LazyLock<Vec<PostInstallRecipe>> = LazyLock::new(build_recipes);

fn build_recipes() -> Vec<PostInstallRecipe> {
    vec![
        // ── GBrain ──────────────────────────────────────────────────────
        PostInstallRecipe {
            source: "garrytan/gbrain".into(),
            title: "GBrain Knowledge Base Setup".into(),
            instructions: "\
## Post-Install: GBrain Knowledge Base

GBrain gives the agent a persistent knowledge graph with hybrid search \
(vector + keyword) and self-wiring entity links.

### Steps

1. **Install the gbrain CLI** (requires Bun):
   ```
   bun install -g gbrain
   ```

2. **Reuse Moltis provider keys.** GBrain needs an OpenAI API key for \
embeddings and optionally an Anthropic key for enrichment. Check which keys \
Moltis already has:
   ```json
   // RPC: providers.available
   ```
   If `openai` and/or `anthropic` keys are configured, pass them in the \
`env` block when registering the MCP server (step 3). This avoids asking \
the user to enter the same keys twice.

3. **Register the MCP server** using `mcp.add`, injecting existing provider keys:
   ```json
   // RPC: mcp.add
   {
     \"name\": \"gbrain\",
     \"command\": \"gbrain\",
     \"args\": [\"serve\", \"--mcp\"],
     \"env\": {
       \"OPENAI_API_KEY\": \"<from providers.available openai key>\",
       \"ANTHROPIC_API_KEY\": \"<from providers.available anthropic key, if present>\"
     },
     \"display_name\": \"GBrain Knowledge Base\"
   }
   ```
   If the OpenAI key is not configured in Moltis, ask the user for it.

4. **Verify** the server is connected:
   ```json
   // RPC: mcp.status
   { \"name\": \"gbrain\" }
   ```

Once connected, the agent gains access to GBrain tools: `search`, `query`, \
`get_page`, `put_page`, `list_pages`, and more.

### Note on gbrain init

If the user has not run `gbrain init` before, the MCP server will use \
PGLite (embedded Postgres) by default with the API keys from the env block. \
For advanced setup (remote Postgres, custom config), the user can run \
`gbrain init` separately."
                .into(),
            mcp_servers: vec![McpRecipeServer {
                name: "gbrain".into(),
                transport: "stdio".into(),
                command: Some("gbrain".into()),
                args: vec!["serve".into(), "--mcp".into()],
                url: None,
                env_keys: vec!["OPENAI_API_KEY".into(), "ANTHROPIC_API_KEY".into()],
                display_name: Some("GBrain Knowledge Base".into()),
                hint: Some(
                    "Persistent knowledge graph with hybrid search for agent long-term memory"
                        .into(),
                ),
            }],
            skills_to_enable: vec![],
        },
    ]
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gbrain_recipe_found_by_short_source() {
        let recipe = get_recipe("garrytan/gbrain").expect("gbrain recipe should exist");
        assert_eq!(recipe.source, "garrytan/gbrain");
        assert!(!recipe.mcp_servers.is_empty());
        assert_eq!(recipe.mcp_servers[0].name, "gbrain");
    }

    #[test]
    fn gbrain_recipe_found_by_github_url() {
        let recipe =
            get_recipe("https://github.com/garrytan/gbrain").expect("should match GitHub URL");
        assert_eq!(recipe.source, "garrytan/gbrain");
    }

    #[test]
    fn gbrain_recipe_found_case_insensitive() {
        let recipe = get_recipe("GarryTan/GBrain").expect("should match case-insensitively");
        assert_eq!(recipe.source, "garrytan/gbrain");
    }

    #[test]
    fn unknown_repo_returns_none() {
        assert!(get_recipe("unknown/repo").is_none());
    }

    #[test]
    fn has_recipe_matches() {
        assert!(has_recipe("garrytan/gbrain"));
        assert!(has_recipe("https://github.com/garrytan/gbrain/"));
        assert!(!has_recipe("unknown/repo"));
    }

    #[test]
    fn list_recipes_non_empty() {
        let recipes = list_recipes();
        assert!(!recipes.is_empty());
    }

    #[test]
    fn all_recipes_have_required_fields() {
        for recipe in list_recipes() {
            assert!(!recipe.source.is_empty(), "recipe source must not be empty");
            assert!(!recipe.title.is_empty(), "recipe title must not be empty");
            assert!(
                !recipe.instructions.is_empty(),
                "recipe instructions must not be empty"
            );
        }
    }

    #[test]
    fn mcp_servers_have_valid_transport() {
        let valid = ["stdio", "sse", "streamable-http"];
        for recipe in list_recipes() {
            for server in &recipe.mcp_servers {
                assert!(
                    valid.contains(&server.transport.as_str()),
                    "invalid transport '{}' in recipe '{}'",
                    server.transport,
                    recipe.source,
                );
            }
        }
    }

    #[test]
    fn normalise_strips_github_prefix_and_trailing_slash() {
        assert_eq!(normalise_source("https://github.com/foo/bar/"), "foo/bar");
        assert_eq!(
            normalise_source("https://github.com/Foo/Bar.git"),
            "foo/bar"
        );
        assert_eq!(normalise_source("  foo/bar  "), "foo/bar");
    }

    #[test]
    fn recipe_serialises_cleanly() {
        let recipe = get_recipe("garrytan/gbrain").unwrap();
        let json = serde_json::to_string_pretty(recipe).unwrap();
        assert!(json.contains("gbrain"));
        assert!(json.contains("mcp_servers"));
    }
}
