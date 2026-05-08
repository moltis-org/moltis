//! Skills management subcommands.

use {
    clap::Subcommand,
    serde_json::{Value, json},
};

use crate::client::CtlClient;

#[derive(Subcommand)]
pub enum SkillsCommand {
    /// List all discovered skills.
    List,
    /// Show skills service status.
    Status,
    /// Install a skill repository.
    Install {
        /// Repository source (owner/repo or GitHub URL).
        #[arg(long)]
        source: String,
    },
    /// List installed repositories.
    Repos,
    /// Get post-install recipe for a repository.
    Recipe {
        /// Repository source (owner/repo or GitHub URL).
        #[arg(long)]
        source: String,
    },
    /// List bundled skill categories.
    Categories,
}

pub async fn run(client: &mut CtlClient, cmd: SkillsCommand) -> anyhow::Result<Value> {
    match cmd {
        SkillsCommand::List => client
            .call("skills.list", Value::Null)
            .await
            .map_err(Into::into),
        SkillsCommand::Status => client
            .call("skills.status", Value::Null)
            .await
            .map_err(Into::into),
        SkillsCommand::Install { source } => client
            .call("skills.install", json!({ "source": source }))
            .await
            .map_err(Into::into),
        SkillsCommand::Repos => client
            .call("skills.repos.list", Value::Null)
            .await
            .map_err(Into::into),
        SkillsCommand::Recipe { source } => client
            .call("skills.recipe", json!({ "source": source }))
            .await
            .map_err(Into::into),
        SkillsCommand::Categories => client
            .call("skills.bundled.categories", Value::Null)
            .await
            .map_err(Into::into),
    }
}
