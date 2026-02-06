use {anyhow::Result, clap::Subcommand};

use moltis_config::validate::{self, Severity};

#[derive(Subcommand)]
pub enum ConfigAction {
    /// Validate the configuration file and report errors/warnings.
    Check {
        /// Show informational diagnostics in addition to errors and warnings.
        #[arg(long)]
        verbose: bool,
    },
    /// Get a config value (not yet implemented).
    Get { key: Option<String> },
    /// Set a config value (not yet implemented).
    Set { key: String, value: String },
    /// Open the config file in your editor (not yet implemented).
    Edit,
}

pub async fn handle_config(action: ConfigAction) -> Result<()> {
    match action {
        ConfigAction::Check { verbose } => check(verbose),
        ConfigAction::Get { .. } | ConfigAction::Set { .. } | ConfigAction::Edit => {
            eprintln!("not yet implemented");
            Ok(())
        },
    }
}

/// ANSI color codes.
const RED: &str = "\x1b[31m";
const YELLOW: &str = "\x1b[33m";
const CYAN: &str = "\x1b[36m";
const BOLD: &str = "\x1b[1m";
const RESET: &str = "\x1b[0m";

fn check(verbose: bool) -> Result<()> {
    let result = validate::validate(None);

    // Print which file we're checking
    if let Some(ref path) = result.config_path {
        eprintln!("Checking {}\n", path.display());
    } else {
        eprintln!("No config file found; checking defaults.\n");
    }

    let mut shown = 0;
    for d in &result.diagnostics {
        if d.severity == Severity::Info && !verbose {
            continue;
        }

        let (color, label) = match d.severity {
            Severity::Error => (RED, "error"),
            Severity::Warning => (YELLOW, "warning"),
            Severity::Info => (CYAN, "info"),
        };

        if d.path.is_empty() {
            eprintln!("  {BOLD}{color}{label}{RESET} {}", d.message);
        } else {
            eprintln!("  {BOLD}{color}{label}{RESET} {}: {}", d.path, d.message);
        }
        shown += 1;
    }

    let errors = result.count(Severity::Error);
    let warnings = result.count(Severity::Warning);

    if shown > 0 {
        eprintln!();
    }

    if errors == 0 && warnings == 0 {
        eprintln!("No issues found.");
    } else {
        eprintln!("{errors} error(s), {warnings} warning(s)");
    }

    if errors > 0 {
        std::process::exit(1);
    }

    Ok(())
}
