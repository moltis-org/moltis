//! CLI subcommands for Cloudflare Tunnel configuration.

use {anyhow::Result, clap::Subcommand, secrecy::Secret};

#[derive(Subcommand)]
pub enum CloudflareTunnelAction {
    /// Show configured Cloudflare Tunnel status.
    Status,
    /// Enable Cloudflare Tunnel in config.
    Enable {
        /// Connector token. If omitted, CLOUDFLARE_TUNNEL_TOKEN must be set.
        #[arg(long)]
        token: Option<String>,
        /// Optional public hostname for display and passkey origin updates.
        #[arg(long)]
        hostname: Option<String>,
    },
    /// Disable Cloudflare Tunnel in config.
    Disable,
}

pub async fn handle_cloudflare_tunnel(action: CloudflareTunnelAction) -> Result<()> {
    match action {
        CloudflareTunnelAction::Status => {
            let config = moltis_config::discover_and_load();
            println!("Enabled: {}", config.cloudflare_tunnel.enabled);
            println!(
                "Token:   {}",
                if config.cloudflare_tunnel.token.is_some() {
                    "stored in config"
                } else if std::env::var_os("CLOUDFLARE_TUNNEL_TOKEN").is_some() {
                    "from CLOUDFLARE_TUNNEL_TOKEN"
                } else {
                    "not configured"
                }
            );
            if let Some(hostname) = config.cloudflare_tunnel.hostname {
                println!("URL:     https://{hostname}");
            }
        },
        CloudflareTunnelAction::Enable { token, hostname } => {
            let has_token =
                token.is_some() || std::env::var_os("CLOUDFLARE_TUNNEL_TOKEN").is_some();
            if !has_token {
                anyhow::bail!("Cloudflare Tunnel requires --token or CLOUDFLARE_TUNNEL_TOKEN");
            }
            moltis_config::update_config(|config| {
                config.cloudflare_tunnel.enabled = true;
                if let Some(token) = token.clone() {
                    config.cloudflare_tunnel.token = Some(Secret::new(token));
                }
                config.cloudflare_tunnel.hostname = hostname.clone();
            })?;
            println!("Cloudflare Tunnel enabled in config");
        },
        CloudflareTunnelAction::Disable => {
            moltis_config::update_config(|config| {
                config.cloudflare_tunnel.enabled = false;
            })?;
            println!("Cloudflare Tunnel disabled");
        },
    }

    Ok(())
}
