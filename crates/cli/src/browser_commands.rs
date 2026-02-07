//! CLI subcommands for browser configuration management.

use {anyhow::Result, clap::Subcommand};

#[derive(Subcommand)]
pub enum BrowserAction {
    /// Show current browser configuration status.
    Status,
    /// Enable browser sandbox mode (run Chrome in a container).
    #[command(name = "sandbox-enable")]
    SandboxEnable,
    /// Disable browser sandbox mode (run Chrome on host).
    #[command(name = "sandbox-disable")]
    SandboxDisable,
    /// Enable browser support.
    Enable,
    /// Disable browser support.
    Disable,
}

pub fn handle_browser(action: BrowserAction) -> Result<()> {
    match action {
        BrowserAction::Status => status(),
        BrowserAction::SandboxEnable => sandbox_enable(),
        BrowserAction::SandboxDisable => sandbox_disable(),
        BrowserAction::Enable => enable(),
        BrowserAction::Disable => disable(),
    }
}

fn status() -> Result<()> {
    let config = moltis_config::discover_and_load();
    let browser = &config.tools.browser;

    println!("Browser configuration:");
    println!("  enabled:        {}", browser.enabled);
    println!("  sandbox:        {}", browser.sandbox);
    println!("  headless:       {}", browser.headless);
    println!(
        "  viewport:       {}x{}",
        browser.viewport_width, browser.viewport_height
    );
    println!("  max_instances:  {}", browser.max_instances);

    if let Some(ref path) = browser.chrome_path {
        println!("  chrome_path:    {}", path);
    } else {
        println!("  chrome_path:    (auto-detect)");
    }

    println!("  sandbox_image:  {}", browser.sandbox_image);

    if !browser.allowed_domains.is_empty() {
        println!("  allowed_domains: {:?}", browser.allowed_domains);
    }

    Ok(())
}

fn sandbox_enable() -> Result<()> {
    let path = moltis_config::update_config(|config| {
        config.tools.browser.sandbox = true;
    })?;
    println!("Browser sandbox enabled.");
    println!("Config saved to: {}", path.display());
    println!("\nRestart the gateway for changes to take effect.");
    Ok(())
}

fn sandbox_disable() -> Result<()> {
    let path = moltis_config::update_config(|config| {
        config.tools.browser.sandbox = false;
    })?;
    println!("Browser sandbox disabled.");
    println!("Config saved to: {}", path.display());
    println!("\nRestart the gateway for changes to take effect.");
    Ok(())
}

fn enable() -> Result<()> {
    let path = moltis_config::update_config(|config| {
        config.tools.browser.enabled = true;
    })?;
    println!("Browser support enabled.");
    println!("Config saved to: {}", path.display());
    println!("\nRestart the gateway for changes to take effect.");
    Ok(())
}

fn disable() -> Result<()> {
    let path = moltis_config::update_config(|config| {
        config.tools.browser.enabled = false;
    })?;
    println!("Browser support disabled.");
    println!("Config saved to: {}", path.display());
    println!("\nRestart the gateway for changes to take effect.");
    Ok(())
}
