mod auth_commands;
mod browser_commands;
mod config_commands;
mod db_commands;
mod hooks_commands;
mod sandbox_commands;
#[cfg(feature = "tailscale")]
mod tailscale_commands;

use {
    clap::{Parser, Subcommand},
    moltis_gateway::logs::{LogBroadcastLayer, LogBuffer},
    tracing::info,
    tracing_subscriber::{EnvFilter, fmt, layer::SubscriberExt, util::SubscriberInitExt},
};

#[derive(Parser)]
#[command(name = "moltis", about = "Moltis — personal AI gateway")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    /// Log level (trace, debug, info, warn, error).
    #[arg(long, global = true, default_value = "info")]
    log_level: String,

    /// Output logs as JSON instead of human-readable.
    #[arg(long, global = true, default_value_t = false)]
    json_logs: bool,

    // Gateway arguments (used when no subcommand is provided, or with `gateway` subcommand)
    /// Address to bind to (overrides config value).
    #[arg(long, global = true)]
    bind: Option<String>,
    /// Port to listen on (overrides config value).
    #[arg(long, global = true)]
    port: Option<u16>,
    /// Custom config directory (overrides default ~/.config/moltis/).
    #[arg(long, global = true, env = "MOLTIS_CONFIG_DIR")]
    config_dir: Option<std::path::PathBuf>,
    /// Custom data directory (overrides default data dir).
    #[arg(long, global = true, env = "MOLTIS_DATA_DIR")]
    data_dir: Option<std::path::PathBuf>,
    /// Disable TLS (for cloud deployments where the provider handles TLS).
    #[cfg(feature = "tls")]
    #[arg(long, global = true, env = "MOLTIS_NO_TLS")]
    no_tls: bool,
    /// Tailscale mode: off, serve, or funnel.
    #[cfg(feature = "tailscale")]
    #[arg(long, global = true, env = "MOLTIS_TAILSCALE")]
    tailscale: Option<String>,
    /// Reset tailscale serve/funnel when the gateway exits.
    #[cfg(feature = "tailscale")]
    #[arg(long, global = true, default_value_t = true)]
    tailscale_reset_on_exit: bool,
}

#[derive(Subcommand)]
enum Commands {
    /// Start the gateway server (default when no subcommand is provided).
    Gateway,
    /// Invoke an agent directly.
    Agent {
        #[arg(short, long)]
        message: String,
        #[arg(long)]
        thinking: Option<String>,
    },
    /// Channel management.
    Channels {
        #[command(subcommand)]
        action: ChannelAction,
    },
    /// Send a message.
    Send {
        #[arg(long)]
        to: String,
        #[arg(short, long)]
        message: String,
    },
    /// Session management.
    Sessions {
        #[command(subcommand)]
        action: SessionAction,
    },
    /// Configuration management.
    Config {
        #[command(subcommand)]
        action: config_commands::ConfigAction,
    },
    /// List available models.
    Models,
    /// Interactive onboarding wizard.
    Onboard,
    /// Config validation and migration.
    Doctor,
    /// Authentication management for OAuth providers.
    Auth {
        #[command(subcommand)]
        action: auth_commands::AuthAction,
    },
    /// Skill management.
    Skills {
        #[command(subcommand)]
        action: SkillAction,
    },
    /// Hook management.
    Hooks {
        #[command(subcommand)]
        action: hooks_commands::HookAction,
    },
    /// Sandbox image management.
    Sandbox {
        #[command(subcommand)]
        action: sandbox_commands::SandboxAction,
    },
    /// Browser configuration management.
    Browser {
        #[command(subcommand)]
        action: browser_commands::BrowserAction,
    },
    /// Database management (reset, clear, migrate).
    Db {
        #[command(subcommand)]
        action: db_commands::DbAction,
    },
    /// Tailscale Serve/Funnel management.
    #[cfg(feature = "tailscale")]
    Tailscale {
        #[command(subcommand)]
        action: tailscale_commands::TailscaleAction,
    },
    /// Install the Moltis CA certificate into the system trust store.
    #[cfg(feature = "tls")]
    TrustCa,
}

#[derive(Subcommand)]
enum ChannelAction {
    Status,
    Login,
    Logout,
}

#[derive(Subcommand)]
enum SessionAction {
    List,
    Clear { key: String },
    History { key: String },
}

#[derive(Subcommand)]
enum SkillAction {
    /// List all discovered skills.
    List,
    /// Install a skill from a GitHub repository (owner/repo format).
    Add {
        /// Source in owner/repo format (e.g. vercel-labs/agent-skills).
        source: String,
    },
    /// Remove an installed repo and all its skills.
    Remove {
        /// Source in owner/repo format.
        source: String,
    },
    /// Show details about a skill.
    Info {
        /// Skill name.
        name: String,
    },
}

/// Initialise tracing and optionally attach a [`LogBroadcastLayer`] that
/// captures events into an in-memory ring buffer for the web UI.
fn init_telemetry(cli: &Cli, log_buffer: Option<LogBuffer>) {
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(&cli.log_level));

    let registry = tracing_subscriber::registry().with(filter);

    // Optionally attach the in-memory capture layer.
    let log_layer = log_buffer.map(LogBroadcastLayer::new);

    if cli.json_logs {
        registry
            .with(fmt::layer().json().with_target(true).with_thread_ids(false))
            .with(log_layer)
            .init();
    } else {
        registry
            .with(
                fmt::layer()
                    .with_target(false)
                    .with_thread_ids(false)
                    .with_ansi(true),
            )
            .with(log_layer)
            .init();
    }
}

#[cfg(feature = "tls")]
async fn trust_ca() -> anyhow::Result<()> {
    let cert_dir = moltis_gateway::tls::cert_dir()?;
    let ca_path = cert_dir.join("ca.pem");

    if !ca_path.exists() {
        eprintln!(
            "CA certificate not found at {}. Start the gateway first to generate certificates.",
            ca_path.display()
        );
        return Ok(());
    }

    eprintln!("Installing CA certificate: {}", ca_path.display());

    #[cfg(target_os = "macos")]
    {
        let status = std::process::Command::new("security")
            .args([
                "add-trusted-cert",
                "-r",
                "trustRoot",
                "-k",
                &format!(
                    "{}/Library/Keychains/login.keychain-db",
                    std::env::var("HOME").unwrap_or_default()
                ),
            ])
            .arg(&ca_path)
            .status()?;
        if status.success() {
            eprintln!(
                "CA certificate installed successfully. Restart your browser to pick up the change."
            );
        } else {
            eprintln!("Failed to install CA certificate (exit code: {})", status);
        }
    }

    #[cfg(target_os = "linux")]
    {
        let dest = std::path::PathBuf::from("/usr/local/share/ca-certificates/moltis-ca.crt");
        eprintln!("Copying CA to {} (may require sudo)", dest.display());
        let status = std::process::Command::new("sudo")
            .args(["cp"])
            .arg(&ca_path)
            .arg(&dest)
            .status()?;
        if status.success() {
            let update = std::process::Command::new("sudo")
                .arg("update-ca-certificates")
                .status()?;
            if update.success() {
                eprintln!("CA certificate installed successfully.");
            } else {
                eprintln!("update-ca-certificates failed (exit code: {})", update);
            }
        } else {
            eprintln!("Failed to copy CA certificate (exit code: {})", status);
        }
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        eprintln!(
            "Automatic trust installation is not supported on this OS.\n\
             Manually import the CA certificate from: {}",
            ca_path.display()
        );
    }

    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    let cli = Cli::parse();

    // Create the log buffer only for the gateway command so the web UI can
    // display captured log entries.
    let log_buffer = if matches!(cli.command, None | Some(Commands::Gateway)) {
        Some(LogBuffer::default())
    } else {
        None
    };

    init_telemetry(&cli, log_buffer.clone());

    info!(version = env!("CARGO_PKG_VERSION"), "moltis starting");

    // Apply directory overrides before any command so all subcommands
    // (config check, db, sandbox, etc.) respect --config-dir / --data-dir.
    if let Some(ref dir) = cli.config_dir {
        moltis_config::set_config_dir(dir.clone());
    }
    if let Some(ref dir) = cli.data_dir {
        moltis_config::set_data_dir(dir.clone());
    }

    match cli.command {
        // Default: start gateway when no subcommand is provided
        None | Some(Commands::Gateway) => {
            // Load config to get server settings
            let config = moltis_config::discover_and_load();

            // CLI args override config values
            let bind = cli.bind.unwrap_or(config.server.bind);
            let port = cli.port.unwrap_or(config.server.port);

            #[cfg(feature = "tls")]
            let no_tls = cli.no_tls;
            #[cfg(not(feature = "tls"))]
            let no_tls = false;

            #[cfg(feature = "tailscale")]
            let tailscale_opts = cli
                .tailscale
                .map(|mode| moltis_gateway::server::TailscaleOpts {
                    mode,
                    reset_on_exit: cli.tailscale_reset_on_exit,
                });
            #[cfg(not(feature = "tailscale"))]
            let tailscale_opts: Option<()> = None;
            let _ = &tailscale_opts; // suppress unused warning when feature disabled
            moltis_gateway::server::start_gateway(
                &bind,
                port,
                no_tls,
                log_buffer,
                cli.config_dir,
                cli.data_dir,
                #[cfg(feature = "tailscale")]
                tailscale_opts,
            )
            .await
        },
        Some(Commands::Agent { message, .. }) => {
            let result = moltis_agents::runner::run_agent("default", "main", &message).await?;
            println!("{result}");
            Ok(())
        },
        Some(Commands::Onboard) => moltis_onboarding::wizard::run_onboarding().await,
        Some(Commands::Auth { action }) => auth_commands::handle_auth(action).await,
        Some(Commands::Sandbox { action }) => sandbox_commands::handle_sandbox(action).await,
        Some(Commands::Browser { action }) => browser_commands::handle_browser(action),
        Some(Commands::Db { action }) => db_commands::handle_db(action).await,
        #[cfg(feature = "tailscale")]
        Some(Commands::Tailscale { action }) => tailscale_commands::handle_tailscale(action).await,
        Some(Commands::Skills { action }) => handle_skills(action).await,
        Some(Commands::Config { action }) => config_commands::handle_config(action).await,
        Some(Commands::Hooks { action }) => hooks_commands::handle_hooks(action).await,
        #[cfg(feature = "tls")]
        Some(Commands::TrustCa) => trust_ca().await,
        Some(_) => {
            eprintln!("command not yet implemented");
            Ok(())
        },
    }
}

async fn handle_skills(action: SkillAction) -> anyhow::Result<()> {
    use moltis_skills::{
        discover::FsSkillDiscoverer,
        install,
        registry::{InMemoryRegistry, SkillRegistry},
    };

    let cwd = std::env::current_dir()?;
    let search_paths = FsSkillDiscoverer::default_paths(&cwd);
    let discoverer = FsSkillDiscoverer::new(search_paths);

    match action {
        SkillAction::List => {
            let registry = InMemoryRegistry::from_discoverer(&discoverer).await?;
            let skills = registry.list_skills().await?;
            if skills.is_empty() {
                println!("No skills found.");
            } else {
                for skill in &skills {
                    let source = skill
                        .source
                        .as_ref()
                        .map(|s| format!("{s:?}"))
                        .unwrap_or_default();
                    println!("  {} — {} [{}]", skill.name, skill.description, source);
                }
            }
        },
        SkillAction::Add { source } => {
            let install_dir = install::default_install_dir()?;
            let skills = install::install_skill(&source, &install_dir).await?;
            for meta in &skills {
                println!("Installed skill '{}': {}", meta.name, meta.description);
            }
        },
        SkillAction::Remove { source } => {
            let install_dir = install::default_install_dir()?;
            install::remove_repo(&source, &install_dir).await?;
            println!("Removed repo '{source}' and all its skills.");
        },
        SkillAction::Info { name } => {
            let registry = InMemoryRegistry::from_discoverer(&discoverer).await?;
            let content = registry.load_skill(&name).await?;
            let meta = &content.metadata;
            println!("Name:        {}", meta.name);
            println!("Description: {}", meta.description);
            if let Some(ref license) = meta.license {
                println!("License:     {license}");
            }
            if !meta.allowed_tools.is_empty() {
                println!("Tools:       {}", meta.allowed_tools.join(", "));
            }
            println!("Path:        {}", meta.path.display());
            println!("Source:      {:?}", meta.source);
            println!("\n{}", content.body);
        },
    }

    Ok(())
}
