//! Terminal-based onboarding wizard using the shared state machine.

use std::io::{BufRead, Write};

use moltis_config::{MoltisConfig, find_or_default_config_path, save_config};

use crate::state::WizardState;

/// Run the interactive onboarding wizard in the terminal.
pub async fn run_onboarding() -> anyhow::Result<()> {
    let config_path = find_or_default_config_path();

    // Check if already onboarded.
    let mut identity_name: Option<String> = None;
    let mut user_name: Option<String> = None;
    if config_path.exists()
        && let Ok(cfg) = moltis_config::loader::load_config(&config_path)
    {
        identity_name = cfg.identity.name;
        user_name = cfg.user.name;
    }
    if let Some(id) = moltis_config::load_identity()
        && id.name.is_some()
    {
        identity_name = id.name;
    }
    if let Some(user) = moltis_config::load_user()
        && user.name.is_some()
    {
        user_name = user.name;
    }

    if identity_name.is_some() && user_name.is_some() {
        println!(
            "Already onboarded as {} with agent {}.",
            user_name.as_deref().unwrap_or("?"),
            identity_name.as_deref().unwrap_or("?"),
        );
        return Ok(());
    }

    let mut state = WizardState::new();
    let stdin = std::io::stdin();
    let mut reader = stdin.lock();

    while !state.is_done() {
        println!("{}", state.prompt());
        print!("> ");
        std::io::stdout().flush()?;
        let mut line = String::new();
        reader.read_line(&mut line)?;
        state.advance(&line);
    }

    // Merge into existing config or create new one.
    let mut config = if config_path.exists() {
        moltis_config::loader::load_config(&config_path).unwrap_or_default()
    } else {
        MoltisConfig::default()
    };
    config.identity = state.identity;
    config.user = state.user;

    let path = save_config(&config)?;
    moltis_config::save_identity(&config.identity)?;
    moltis_config::save_user(&config.user)?;
    println!("Config saved to {}", path.display());
    println!("Onboarding complete!");
    Ok(())
}
