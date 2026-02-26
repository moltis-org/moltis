//! CLI subcommands for channel configuration.

use std::io::Write;

use {
    anyhow::{Result, anyhow},
    clap::{Args, Subcommand},
    rand::RngCore,
    serde_json::{Map, Value},
};

const DEFAULT_OAUTH_TENANT: &str = "botframework.com";
const DEFAULT_OAUTH_SCOPE: &str = "https://api.botframework.com/.default";

#[derive(Subcommand)]
pub enum ChannelAction {
    /// Show channel status (not yet implemented in CLI).
    Status,
    /// Log in a channel provider (not yet implemented in CLI).
    Login,
    /// Log out a channel provider (not yet implemented in CLI).
    Logout,
    /// Microsoft Teams channel helpers.
    Teams {
        #[command(subcommand)]
        action: TeamsAction,
    },
}

#[derive(Subcommand)]
pub enum TeamsAction {
    /// Guide setup for a self-hosted Microsoft Teams channel account.
    Bootstrap(TeamsBootstrapArgs),
}

#[derive(Args, Clone)]
pub struct TeamsBootstrapArgs {
    /// Local account key used by Moltis and in the webhook path.
    #[arg(long)]
    account_id: Option<String>,
    /// Azure Bot App ID (client ID).
    #[arg(long)]
    app_id: Option<String>,
    /// Azure Bot App Password (client secret).
    #[arg(long)]
    app_password: Option<String>,
    /// Public base URL of your Moltis instance (for webhook endpoint generation).
    #[arg(long)]
    base_url: Option<String>,
    /// Optional webhook shared secret. If omitted, a random value is generated.
    #[arg(long)]
    webhook_secret: Option<String>,
    /// OAuth tenant segment used for token issuance.
    #[arg(long, default_value = DEFAULT_OAUTH_TENANT)]
    oauth_tenant: String,
    /// OAuth scope used for Bot Framework connector calls.
    #[arg(long, default_value = DEFAULT_OAUTH_SCOPE)]
    oauth_scope: String,
    /// Overwrite an existing Teams account config without confirmation.
    #[arg(long, default_value_t = false)]
    force: bool,
    /// Print generated values without writing `moltis.toml`.
    #[arg(long, default_value_t = false)]
    dry_run: bool,
    /// Open Microsoft setup docs in browser tabs.
    #[arg(long, default_value_t = false)]
    open: bool,
}

pub async fn handle_channels(action: ChannelAction) -> Result<()> {
    match action {
        ChannelAction::Status | ChannelAction::Login | ChannelAction::Logout => {
            eprintln!("not yet implemented");
            Ok(())
        },
        ChannelAction::Teams { action } => handle_teams(action),
    }
}

fn handle_teams(action: TeamsAction) -> Result<()> {
    match action {
        TeamsAction::Bootstrap(args) => run_teams_bootstrap(args),
    }
}

fn run_teams_bootstrap(args: TeamsBootstrapArgs) -> Result<()> {
    let config = moltis_config::discover_and_load();
    let default_base_url = default_gateway_base_url(&config);

    let app_id = required_value(
        args.app_id,
        "Azure Bot App ID (client ID)",
        Some("GUID from Azure App Registration"),
    )?;
    let app_password = required_value(
        args.app_password,
        "Azure Bot App Password (client secret)",
        Some("generated secret value"),
    )?;

    let account_id_default = app_id.clone();
    let account_id = {
        let chosen = match args.account_id {
            Some(value) if !value.trim().is_empty() => value.trim().to_string(),
            _ => prompt_required(
                "Account ID (local key used in webhook path)",
                Some(account_id_default.as_str()),
            )?,
        };
        validate_account_id(&chosen)?;
        chosen
    };

    let raw_base_url = match args.base_url {
        Some(value) if !value.trim().is_empty() => value.trim().to_string(),
        _ => prompt_required(
            "Public base URL for this Moltis instance",
            Some(default_base_url.as_str()),
        )?,
    };
    let base_url = normalize_base_url(&raw_base_url)?;

    let webhook_secret_provided = args
        .webhook_secret
        .as_ref()
        .is_some_and(|value| !value.trim().is_empty());
    let mut webhook_secret = if webhook_secret_provided {
        args.webhook_secret
            .as_deref()
            .unwrap_or_default()
            .trim()
            .to_string()
    } else {
        generate_webhook_secret()
    };
    if !webhook_secret_provided {
        println!("Generated webhook secret: {webhook_secret}");
        if !args.force && !prompt_yes_no("Use generated webhook secret", true)? {
            webhook_secret = prompt_required("Webhook secret", None)?;
        }
    }

    let existing = config.channels.msteams.get(&account_id).cloned();
    if existing.is_some()
        && !args.force
        && !prompt_yes_no(
            &format!("Teams account '{account_id}' already exists. Overwrite credentials"),
            false,
        )?
    {
        println!("Aborted. No changes written.");
        return Ok(());
    }

    let endpoint = build_webhook_endpoint(&base_url, &account_id, &webhook_secret)?;
    let config_value = build_channel_config(
        &app_id,
        &app_password,
        &args.oauth_tenant,
        &args.oauth_scope,
        &webhook_secret,
        existing.as_ref(),
    );

    println!();
    println!("Teams bootstrap summary");
    println!("  account_id:      {account_id}");
    println!("  app_id:          {app_id}");
    println!("  oauth_tenant:    {}", args.oauth_tenant);
    println!("  oauth_scope:     {}", args.oauth_scope);
    println!("  webhook_endpoint:");
    println!("    {endpoint}");
    println!();

    if args.dry_run {
        println!("Dry run enabled. Generated config payload:");
        println!("{}", serde_json::to_string_pretty(&config_value)?);
        println!();
        print_setup_links();
        return Ok(());
    }

    let account_id_for_save = account_id.clone();
    let value_for_save = config_value;
    let path = moltis_config::update_config(move |cfg| {
        cfg.channels
            .msteams
            .insert(account_id_for_save, value_for_save);
    })?;

    println!("Saved Teams channel config to {}", path.display());
    println!("Restart Moltis gateway to apply channel changes.");
    println!();
    print_setup_links();
    println!("Set your Azure Bot messaging endpoint to:");
    println!("  {endpoint}");
    println!();
    println!("Then verify in Settings -> Channels (Microsoft Teams).");

    if args.open {
        open_setup_links();
    }

    Ok(())
}

fn required_value(
    provided: Option<String>,
    prompt: &str,
    placeholder: Option<&str>,
) -> Result<String> {
    match provided {
        Some(value) if !value.trim().is_empty() => Ok(value.trim().to_string()),
        _ => prompt_required(prompt, placeholder),
    }
}

fn prompt_required(prompt: &str, default: Option<&str>) -> Result<String> {
    let mut stdout = std::io::stdout();
    match default {
        Some(value) if !value.trim().is_empty() => write!(stdout, "{prompt} [{value}]: ")?,
        _ => write!(stdout, "{prompt}: ")?,
    }
    stdout.flush()?;

    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return default
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow!("{prompt} is required"));
    }
    Ok(trimmed.to_string())
}

fn prompt_yes_no(prompt: &str, default_yes: bool) -> Result<bool> {
    loop {
        let default = if default_yes {
            "y"
        } else {
            "n"
        };
        let answer = prompt_required(
            &format!(
                "{prompt} [{}]",
                if default_yes {
                    "Y/n"
                } else {
                    "y/N"
                }
            ),
            Some(default),
        )?;
        match answer.trim().to_ascii_lowercase().as_str() {
            "y" | "yes" => return Ok(true),
            "n" | "no" => return Ok(false),
            _ => println!("Please answer with 'y' or 'n'."),
        }
    }
}

fn build_channel_config(
    app_id: &str,
    app_password: &str,
    oauth_tenant: &str,
    oauth_scope: &str,
    webhook_secret: &str,
    existing: Option<&Value>,
) -> Value {
    let mut map: Map<String, Value> = existing
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();

    map.insert("app_id".into(), Value::String(app_id.to_string()));
    map.insert(
        "app_password".into(),
        Value::String(app_password.to_string()),
    );
    map.insert(
        "oauth_tenant".into(),
        Value::String(oauth_tenant.to_string()),
    );
    map.insert("oauth_scope".into(), Value::String(oauth_scope.to_string()));
    let dm_policy = map
        .remove("dm_policy")
        .unwrap_or_else(|| Value::String("allowlist".into()));
    let group_policy = map
        .remove("group_policy")
        .unwrap_or_else(|| Value::String("open".into()));
    let mention_mode = map
        .remove("mention_mode")
        .unwrap_or_else(|| Value::String("mention".into()));
    let allowlist = map
        .remove("allowlist")
        .unwrap_or_else(|| Value::Array(Vec::new()));
    let group_allowlist = map
        .remove("group_allowlist")
        .unwrap_or_else(|| Value::Array(Vec::new()));

    map.insert("dm_policy".into(), dm_policy);
    map.insert("group_policy".into(), group_policy);
    map.insert("mention_mode".into(), mention_mode);
    map.insert("allowlist".into(), allowlist);
    map.insert("group_allowlist".into(), group_allowlist);

    if webhook_secret.trim().is_empty() {
        map.remove("webhook_secret");
    } else {
        map.insert(
            "webhook_secret".into(),
            Value::String(webhook_secret.to_string()),
        );
    }

    Value::Object(map)
}

fn default_gateway_base_url(config: &moltis_config::MoltisConfig) -> String {
    let scheme = if config.tls.enabled {
        "https"
    } else {
        "http"
    };
    let host = match config.server.bind.as_str() {
        "0.0.0.0" | "::" | "[::]" => "localhost",
        other => other,
    };
    format!("{scheme}://{host}:{}", config.server.port)
}

fn validate_account_id(account_id: &str) -> Result<()> {
    if account_id.trim().is_empty() {
        anyhow::bail!("account_id is required");
    }
    if !account_id
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
    {
        anyhow::bail!(
            "account_id contains unsupported characters. Use only letters, numbers, '-', '_', '.'"
        );
    }
    Ok(())
}

fn normalize_base_url(raw: &str) -> Result<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        anyhow::bail!("base URL is required");
    }

    let candidate = if trimmed.contains("://") {
        trimmed.to_string()
    } else {
        format!("https://{trimmed}")
    };

    let mut parsed = reqwest::Url::parse(&candidate)
        .map_err(|e| anyhow!("invalid base URL '{trimmed}': {e}"))?;
    if parsed.host_str().is_none() {
        anyhow::bail!("base URL must include a host");
    }
    if parsed.query().is_some() || parsed.fragment().is_some() {
        anyhow::bail!("base URL must not include query parameters or fragments");
    }

    let path = parsed.path().trim_end_matches('/').to_string();
    let normalized_path = if path.is_empty() {
        "/".to_string()
    } else {
        path
    };
    parsed.set_path(&normalized_path);
    Ok(parsed.as_str().trim_end_matches('/').to_string())
}

fn build_webhook_endpoint(
    base_url: &str,
    account_id: &str,
    webhook_secret: &str,
) -> Result<String> {
    validate_account_id(account_id)?;
    let normalized = normalize_base_url(base_url)?;
    if webhook_secret.trim().is_empty() {
        return Ok(format!(
            "{normalized}/api/channels/msteams/{account_id}/webhook"
        ));
    }

    Ok(format!(
        "{normalized}/api/channels/msteams/{account_id}/webhook?secret={}",
        encode_query_component(webhook_secret)
    ))
}

fn generate_webhook_secret() -> String {
    let mut bytes = [0_u8; 24];
    rand::rng().fill_bytes(&mut bytes);
    let mut out = String::with_capacity(bytes.len() * 2);
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

fn encode_query_component(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for b in value.bytes() {
        if matches!(b, b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~') {
            encoded.push(char::from(b));
        } else {
            encoded.push('%');
            encoded.push(hex_upper((b >> 4) & 0x0f));
            encoded.push(hex_upper(b & 0x0f));
        }
    }
    encoded
}

fn hex_upper(value: u8) -> char {
    match value {
        0..=9 => (b'0' + value) as char,
        10..=15 => (b'A' + (value - 10)) as char,
        _ => '0',
    }
}

fn print_setup_links() {
    println!("Microsoft setup links:");
    println!(
        "  - Bot registration docs: https://learn.microsoft.com/en-us/azure/bot-service/bot-service-quickstart-registration?view=azure-bot-service-4.0"
    );
    println!(
        "  - Teams bot docs: https://learn.microsoft.com/en-us/microsoftteams/platform/bots/build-conversational-capability"
    );
    println!(
        "  - Azure app registrations: https://portal.azure.com/#view/Microsoft_AAD_RegisteredApps/ApplicationsListBlade"
    );
}

fn open_setup_links() {
    for url in [
        "https://learn.microsoft.com/en-us/azure/bot-service/bot-service-quickstart-registration?view=azure-bot-service-4.0",
        "https://learn.microsoft.com/en-us/microsoftteams/platform/bots/build-conversational-capability",
        "https://portal.azure.com/#view/Microsoft_AAD_RegisteredApps/ApplicationsListBlade",
    ] {
        if let Err(error) = open::that(url) {
            eprintln!("Failed to open {url}: {error}");
        }
    }
}

#[allow(clippy::expect_used, clippy::unwrap_used)]
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_base_url_accepts_and_normalizes() {
        assert_eq!(
            normalize_base_url("example.com/").unwrap(),
            "https://example.com"
        );
        assert_eq!(
            normalize_base_url("https://example.com/base/").unwrap(),
            "https://example.com/base"
        );
    }

    #[test]
    fn normalize_base_url_rejects_query() {
        let err = normalize_base_url("https://example.com?x=1").unwrap_err();
        assert!(err.to_string().contains("must not include query"));
    }

    #[test]
    fn account_id_validation_rejects_invalid_chars() {
        assert!(validate_account_id("bot-01_ok").is_ok());
        assert!(validate_account_id("bad/id").is_err());
        assert!(validate_account_id("bad id").is_err());
    }

    #[test]
    fn webhook_endpoint_encodes_secret() {
        let endpoint =
            build_webhook_endpoint("https://bot.example.com", "my-bot", "a b+c").expect("endpoint");
        assert_eq!(
            endpoint,
            "https://bot.example.com/api/channels/msteams/my-bot/webhook?secret=a%20b%2Bc"
        );
    }

    #[test]
    fn generated_webhook_secret_has_expected_shape() {
        let secret = generate_webhook_secret();
        assert_eq!(secret.len(), 48);
        assert!(secret.bytes().all(|b| b.is_ascii_hexdigit()));
    }
}
