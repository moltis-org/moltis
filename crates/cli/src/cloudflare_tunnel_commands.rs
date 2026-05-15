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
                token_source_label(
                    config.cloudflare_tunnel.token.is_some(),
                    env_token_present(),
                )
            );
            if let Some(hostname) = config.cloudflare_tunnel.hostname {
                println!("URL:     https://{hostname}");
            }
        },
        CloudflareTunnelAction::Enable { token, hostname } => {
            if !has_enable_token(&token, env_token_present()) {
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

fn token_source_label(config_has_token: bool, env_has_token: bool) -> &'static str {
    if config_has_token {
        "stored in config"
    } else if env_has_token {
        "from CLOUDFLARE_TUNNEL_TOKEN"
    } else {
        "not configured"
    }
}

fn has_enable_token(token: &Option<String>, env_has_token: bool) -> bool {
    token
        .as_deref()
        .map(str::trim)
        .is_some_and(|token| !token.is_empty())
        || env_has_token
}

fn env_token_present() -> bool {
    std::env::var("CLOUDFLARE_TUNNEL_TOKEN")
        .ok()
        .map(|token| !token.trim().is_empty())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_source_label_prefers_config_token() {
        assert_eq!(token_source_label(true, true), "stored in config");
        assert_eq!(token_source_label(true, false), "stored in config");
    }

    #[test]
    fn token_source_label_reports_env_or_missing_token() {
        assert_eq!(
            token_source_label(false, true),
            "from CLOUDFLARE_TUNNEL_TOKEN"
        );
        assert_eq!(token_source_label(false, false), "not configured");
    }

    #[test]
    fn enable_requires_cli_or_env_token() {
        assert!(has_enable_token(&Some("token".to_string()), false));
        assert!(!has_enable_token(&Some("   ".to_string()), false));
        assert!(has_enable_token(&None, true));
        assert!(!has_enable_token(&None, false));
    }
}
