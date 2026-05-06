//! `moltis-ctl` — lightweight CLI for controlling a Moltis gateway.
//!
//! Designed for use inside sandboxes where the full `moltis` binary is
//! not available. Communicates with the gateway over WebSocket RPC.
//!
//! # Environment variables
//!
//! - `MOLTIS_GATEWAY_URL`: Gateway HTTP URL (default: `http://localhost:8080`)
//! - `MOLTIS_API_KEY`: API key for authentication

mod client;
mod commands;
mod error;
mod output;

use {
    clap::{Parser, Subcommand},
    serde_json::Value,
};

use crate::{client::CtlClient, error::exit, output::print_json};

#[derive(Parser)]
#[command(
    name = "moltis-ctl",
    about = "Lightweight CLI for controlling a Moltis gateway",
    version
)]
struct Cli {
    /// Gateway URL.
    #[arg(
        long,
        env = "MOLTIS_GATEWAY_URL",
        default_value = "http://localhost:8080"
    )]
    gateway_url: String,

    /// API key for authentication.
    #[arg(long, env = "MOLTIS_API_KEY", default_value = "")]
    api_key: String,

    /// Pretty-print JSON output.
    #[arg(long, global = true)]
    pretty: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Check gateway health.
    Health,
    /// Show gateway status.
    Status,
    /// Manage MCP servers.
    #[command(subcommand)]
    Mcp(commands::mcp::McpCommand),
    /// Manage webhooks.
    #[command(subcommand)]
    Webhooks(commands::webhooks::WebhooksCommand),
    /// Manage skills.
    #[command(subcommand)]
    Skills(commands::skills::SkillsCommand),
    /// Manage configuration.
    #[command(subcommand)]
    Config(commands::config::ConfigCommand),
    /// Manage LLM providers.
    #[command(subcommand)]
    Providers(commands::providers::ProvidersCommand),
    /// Send a raw RPC call.
    Rpc {
        /// RPC method name (e.g. mcp.list, webhooks.create).
        method: String,
        /// Optional JSON params.
        params: Option<String>,
    },
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    let mut client = match CtlClient::connect(&cli.gateway_url, &cli.api_key).await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(exit::CONNECTION_ERROR);
        },
    };

    let result = match cli.command {
        Command::Health => commands::health::health(&mut client).await,
        Command::Status => commands::health::status(&mut client).await,
        Command::Mcp(cmd) => commands::mcp::run(&mut client, cmd).await,
        Command::Webhooks(cmd) => commands::webhooks::run(&mut client, cmd).await,
        Command::Skills(cmd) => commands::skills::run(&mut client, cmd).await,
        Command::Config(cmd) => commands::config::run(&mut client, cmd).await,
        Command::Providers(cmd) => commands::providers::run(&mut client, cmd).await,
        Command::Rpc { method, params } => {
            let p: Value = match params {
                Some(s) => match serde_json::from_str(&s) {
                    Ok(v) => v,
                    Err(e) => {
                        eprintln!("error: invalid JSON params: {e}");
                        std::process::exit(exit::BAD_ARGS);
                    },
                },
                None => Value::Null,
            };
            client.call(&method, p).await.map_err(Into::into)
        },
    };

    match result {
        Ok(payload) => {
            print_json(&payload, cli.pretty);
            std::process::exit(exit::SUCCESS);
        },
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(exit::RPC_ERROR);
        },
    }
}
