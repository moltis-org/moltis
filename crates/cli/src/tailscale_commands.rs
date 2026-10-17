//! CLI subcommands for Tailscale Serve/Funnel management.

use {anyhow::Result, clap::Subcommand};

use moltis_gateway::tailscale::{
    CliTailscaleManager, TailscaleManager, TailscaleMode, validate_tailscale_config,
};

#[derive(Subcommand)]
pub enum TailscaleAction {
    /// Show current tailscale serve/funnel status.
    Status,
    /// Enable tailscale serve (tailnet-only HTTPS).
    Serve {
        /// Port to proxy to (defaults to 18789).
        #[arg(long, default_value_t = 18789)]
        port: u16,
    },
    /// Enable tailscale funnel (public HTTPS).
    Funnel {
        /// Port to proxy to (defaults to 18789).
        #[arg(long, default_value_t = 18789)]
        port: u16,
    },
    /// Disable tailscale serve/funnel.
    Disable,
}

pub async fn handle_tailscale(action: TailscaleAction) -> Result<()> {
    let manager = CliTailscaleManager::new();

    match action {
        TailscaleAction::Status => {
            let status = manager.status().await?;
            println!("Mode:         {}", status.mode);
            println!("Tailscale up: {}", status.tailscale_up);
            if let Some(ref hostname) = status.hostname {
                println!("Hostname:     {hostname}");
            }
            if let Some(ref url) = status.url {
                println!("URL:          {url}");
            }
        },
        TailscaleAction::Serve { port } => {
            let config = moltis_config::discover_and_load();
            let tls = config.tls_active();
            validate_tailscale_config(TailscaleMode::Serve, "127.0.0.1", false)?;
            manager.enable_serve(port, tls).await?;
            println!("Tailscale serve enabled on port {port}");
            if let Ok(Some(hostname)) = manager.hostname().await {
                println!("URL: https://{hostname}");
            }
        },
        TailscaleAction::Funnel { port } => {
            let config = moltis_config::discover_and_load();
            let tls = config.tls_active();
            let has_password = !config.auth.disabled;
            validate_tailscale_config(TailscaleMode::Funnel, "127.0.0.1", has_password)?;
            manager.enable_funnel(port, tls).await?;
            println!("Tailscale funnel enabled on port {port}");
            if let Ok(Some(hostname)) = manager.hostname().await {
                println!("URL: https://{hostname}");
            }
        },
        TailscaleAction::Disable => {
            manager.disable().await?;
            println!("Tailscale serve/funnel disabled");
        },
    }

    Ok(())
}
