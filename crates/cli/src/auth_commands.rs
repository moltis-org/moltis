use {
    anyhow::Result,
    clap::Subcommand,
    moltis_oauth::{
        CallbackServer, OAuthFlow, TokenStore, callback_port, device_flow, load_oauth_config,
    },
};

#[derive(Subcommand)]
pub enum AuthAction {
    /// Log in to a provider via OAuth.
    Login {
        /// Provider name (e.g. "openai-codex").
        #[arg(long)]
        provider: String,
    },
    /// Show authentication status for all providers.
    Status,
    /// Log out from a provider.
    Logout {
        /// Provider name (e.g. "openai-codex").
        #[arg(long)]
        provider: String,
    },
    /// Reset gateway authentication (remove password, sessions, passkeys, API keys).
    ResetPassword,
    /// Reset agent identity and user profile (triggers onboarding on next start).
    ResetIdentity,
    /// Create a new API key for authenticating with the gateway.
    CreateApiKey {
        /// Label for the API key (e.g. "CLI tool", "CI pipeline").
        #[arg(long)]
        label: String,
        /// Comma-separated list of scopes. If omitted, the key has full access.
        /// Valid scopes: operator.read, operator.write, operator.approvals, operator.pairing
        #[arg(long)]
        scopes: Option<String>,
    },
}

pub async fn handle_auth(action: AuthAction) -> Result<()> {
    match action {
        AuthAction::Login { provider } => login(&provider).await,
        AuthAction::Status => status(),
        AuthAction::Logout { provider } => logout(&provider),
        AuthAction::ResetPassword => reset_password().await,
        AuthAction::ResetIdentity => reset_identity(),
        AuthAction::CreateApiKey { label, scopes } => create_api_key(&label, scopes).await,
    }
}

async fn login(provider: &str) -> Result<()> {
    let config = load_oauth_config(provider)
        .ok_or_else(|| anyhow::anyhow!("unknown OAuth provider: {provider}"))?;

    if config.device_flow {
        return login_device_flow(provider, &config).await;
    }

    let port = callback_port(&config);
    let flow = OAuthFlow::new(config);
    let req = flow.start()?;

    println!("Opening browser for authentication...");
    if open::that(&req.url).is_err() {
        println!("Could not open browser. Please visit:\n{}", req.url);
    }

    println!("Waiting for callback on http://127.0.0.1:{port}/auth/callback ...");
    let code = CallbackServer::wait_for_code(port, req.state).await?;

    println!("Exchanging code for tokens...");
    let tokens = flow.exchange(&code, &req.pkce.verifier).await?;

    let store = TokenStore::new();
    store.save(provider, &tokens)?;

    println!("Successfully logged in to {provider}");
    Ok(())
}

async fn login_device_flow(provider: &str, config: &moltis_oauth::OAuthConfig) -> Result<()> {
    let client = reqwest::Client::new();

    // Build extra headers for providers that need them (e.g. Kimi Code).
    let extra_headers = build_provider_headers(provider);
    let extra = extra_headers.as_ref();

    let device_resp = device_flow::request_device_code_with_headers(&client, config, extra).await?;

    // Prefer verification_uri_complete (auto-includes user code).
    let open_url = device_resp
        .verification_uri_complete
        .as_deref()
        .unwrap_or(&device_resp.verification_uri);

    println!("Opening browser for device authorization...");
    println!("Your code: {}", device_resp.user_code);
    if open::that(open_url).is_err() {
        println!("Could not open browser. Please visit:\n{open_url}");
    }

    println!("Waiting for authorization...");
    let tokens = device_flow::poll_for_token_with_headers(
        &client,
        config,
        &device_resp.device_code,
        device_resp.interval,
        extra,
    )
    .await?;

    let store = TokenStore::new();
    store.save(provider, &tokens)?;

    println!("Successfully logged in to {provider}");
    Ok(())
}

/// Build provider-specific extra headers for the device flow.
fn build_provider_headers(provider: &str) -> Option<reqwest::header::HeaderMap> {
    match provider {
        "kimi-code" => Some(moltis_oauth::kimi_headers()),
        _ => None,
    }
}

fn status() -> Result<()> {
    let store = TokenStore::new();
    let providers = store.list();
    if providers.is_empty() {
        println!("No authenticated providers.");
        return Ok(());
    }
    for provider in providers {
        if let Some(tokens) = store.load(&provider) {
            let expiry = tokens.expires_at.map_or("unknown".to_string(), |ts| {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                if ts > now {
                    let remaining = ts - now;
                    let hours = remaining / 3600;
                    let mins = (remaining % 3600) / 60;
                    format!("valid ({hours}h {mins}m remaining)")
                } else {
                    "expired".to_string()
                }
            });
            println!("{provider} [{expiry}]");
        }
    }
    Ok(())
}

fn logout(provider: &str) -> Result<()> {
    let store = TokenStore::new();
    store.delete(provider)?;
    println!("Logged out from {provider}");
    Ok(())
}

fn reset_identity() -> Result<()> {
    moltis_config::loader::update_config(|cfg| {
        cfg.identity = Default::default();
        cfg.user = Default::default();
    })?;
    println!("Identity and user profile cleared. Onboarding will be required on next load.");
    Ok(())
}

async fn reset_password() -> Result<()> {
    let data_dir = moltis_config::data_dir();
    let db_path = data_dir.join("moltis.db");
    if !db_path.exists() {
        println!("No database found at {}", db_path.display());
        return Ok(());
    }

    moltis_gateway::auth::CredentialStore::reset_from_db_path(&db_path).await?;
    for line in reset_password_success_lines() {
        println!("{line}");
    }
    Ok(())
}

fn reset_password_success_lines() -> [&'static str; 2] {
    [
        "Authentication reset. Password, sessions, passkeys, and API keys removed.",
        "Authentication is now disabled. Open Settings > Security to set a password or passkey to re-enable it.",
    ]
}

async fn create_api_key(label: &str, scopes_str: Option<String>) -> Result<()> {
    let data_dir = moltis_config::data_dir();
    let db_path = data_dir.join("moltis.db");
    if !db_path.exists() {
        anyhow::bail!(
            "No database found at {}. Start the gateway first to initialize it.",
            db_path.display()
        );
    }

    // Parse and validate scopes
    let scopes: Option<Vec<String>> = if let Some(ref s) = scopes_str {
        let parsed: Vec<String> = s.split(',').map(|s| s.trim().to_string()).collect();
        for scope in &parsed {
            if !moltis_gateway::auth::VALID_SCOPES.contains(&scope.as_str()) {
                anyhow::bail!(
                    "Invalid scope: {scope}\nValid scopes: {}",
                    moltis_gateway::auth::VALID_SCOPES.join(", ")
                );
            }
        }
        Some(parsed)
    } else {
        None
    };

    // Connect to database and create the key
    let db_url = format!("sqlite:{}", db_path.display());
    let pool = sqlx::SqlitePool::connect(&db_url).await?;
    let config = moltis_config::discover_and_load();
    let store = moltis_gateway::auth::CredentialStore::with_config(pool, &config.auth).await?;

    let (id, raw_key) = store.create_api_key(label, scopes.as_deref()).await?;

    println!("API key created successfully!");
    println!();
    println!("  ID:     {id}");
    println!("  Label:  {label}");
    if let Some(ref s) = scopes {
        println!("  Scopes: {}", s.join(", "));
    } else {
        println!("  Scopes: Full access (all scopes)");
    }
    println!();
    println!("Key (save this now, it won't be shown again):");
    println!();
    println!("  {raw_key}");
    println!();

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::reset_password_success_lines;

    #[test]
    fn reset_password_message_describes_disabled_auth_state() {
        let lines = reset_password_success_lines();
        assert_eq!(
            lines[0],
            "Authentication reset. Password, sessions, passkeys, and API keys removed."
        );
        assert_eq!(
            lines[1],
            "Authentication is now disabled. Open Settings > Security to set a password or passkey to re-enable it."
        );
    }
}
