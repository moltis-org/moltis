//! CLI subcommands for NetBird private mesh access.

use {anyhow::Result, clap::Subcommand};

use moltis_gateway::netbird::{
    CliNetbirdManager, NetbirdManager, NetbirdMode, validate_netbird_config,
};

#[derive(Subcommand)]
pub enum NetbirdAction {
    /// Show current NetBird status.
    Status,
    /// Enable NetBird private mesh serve mode in config.
    Serve,
    /// Disable NetBird serve mode in config.
    Disable,
}

pub async fn handle_netbird(action: NetbirdAction) -> Result<()> {
    match action {
        NetbirdAction::Status => {
            let config = moltis_config::discover_and_load();
            let mode = config
                .netbird
                .mode
                .parse::<NetbirdMode>()
                .unwrap_or_default();
            let manager = CliNetbirdManager::new(mode, config.server.port, config.tls.enabled);
            let status = manager.status().await?;
            println!("Mode:       {}", status.mode);
            println!("NetBird up: {}", status.netbird_up);
            if let Some(peer_ip) = status.peer_ip {
                println!("Peer IP:    {peer_ip}");
            }
            if let Some(dns_name) = status.dns_name {
                println!("DNS name:   {dns_name}");
            }
            if let Some(url) = status.url {
                println!("URL:        {url}");
            }
        },
        NetbirdAction::Serve => {
            let config = moltis_config::discover_and_load();
            validate_netbird_config(NetbirdMode::Serve, &config.server.bind)?;
            moltis_config::update_config(|config| {
                config.netbird.mode = NetbirdMode::Serve.to_string();
            })?;
            println!("NetBird serve mode enabled in config");
        },
        NetbirdAction::Disable => {
            moltis_config::update_config(|config| {
                config.netbird.mode = NetbirdMode::Off.to_string();
            })?;
            println!("NetBird serve mode disabled");
        },
    }

    Ok(())
}
