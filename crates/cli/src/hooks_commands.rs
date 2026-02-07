//! CLI commands for hook management.

use clap::Subcommand;

use moltis_plugins::{
    hook_discovery::{FsHookDiscoverer, HookDiscoverer},
    hook_eligibility::check_hook_eligibility,
};

#[derive(Subcommand)]
pub enum HookAction {
    /// List all discovered hooks.
    List {
        /// Show only eligible hooks.
        #[arg(long)]
        eligible: bool,
        /// Output as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Show details about a hook.
    Info {
        /// Hook name.
        name: String,
    },
}

pub async fn handle_hooks(action: HookAction) -> anyhow::Result<()> {
    let discoverer = FsHookDiscoverer::new(FsHookDiscoverer::default_paths());
    let hooks: Vec<_> = discoverer.discover().await?;

    match action {
        HookAction::List { eligible, json } => {
            let mut entries: Vec<serde_json::Value> = Vec::new();

            for (parsed, source) in &hooks {
                let meta = &parsed.metadata;
                let elig = check_hook_eligibility(meta);

                if eligible && !elig.eligible {
                    continue;
                }

                if json {
                    entries.push(serde_json::json!({
                        "name": meta.name,
                        "description": meta.description,
                        "events": meta.events,
                        "command": meta.command,
                        "priority": meta.priority,
                        "source": source,
                        "eligible": elig.eligible,
                        "path": parsed.source_path,
                    }));
                } else {
                    let status = if elig.eligible {
                        "✓"
                    } else {
                        "✗"
                    };
                    let emoji = meta.emoji.as_deref().unwrap_or("🔧");
                    println!(
                        "  {status} {emoji} {name} — {desc} [{source:?}]",
                        name = meta.name,
                        desc = meta.description,
                    );
                    if !elig.eligible {
                        if elig.missing_os {
                            println!("    ↳ requires OS: {:?}", meta.requires.os);
                        }
                        if !elig.missing_bins.is_empty() {
                            println!("    ↳ missing binaries: {:?}", elig.missing_bins);
                        }
                        if !elig.missing_env.is_empty() {
                            println!("    ↳ missing env vars: {:?}", elig.missing_env);
                        }
                    }
                }
            }

            if json {
                println!("{}", serde_json::to_string_pretty(&entries)?);
            } else if hooks.is_empty() {
                println!("No hooks found.");
                let data_dir = moltis_config::data_dir();
                println!(
                    "Place hooks in {}/hooks/<name>/HOOK.md or {}/.moltis/hooks/<name>/HOOK.md",
                    data_dir.display(),
                    data_dir.display(),
                );
            }
        },
        HookAction::Info { name } => {
            let found = hooks.iter().find(|(p, _)| p.metadata.name == name);

            let Some((parsed, source)) = found else {
                eprintln!("Hook '{name}' not found.");
                std::process::exit(1);
            };

            let meta = &parsed.metadata;
            let elig = check_hook_eligibility(meta);

            println!("Name:        {}", meta.name);
            println!("Description: {}", meta.description);
            if let Some(ref emoji) = meta.emoji {
                println!("Emoji:       {emoji}");
            }
            println!(
                "Events:      {}",
                meta.events
                    .iter()
                    .map(|e: &moltis_common::hooks::HookEvent| e.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            if let Some(ref cmd) = meta.command {
                println!("Command:     {cmd}");
            }
            println!("Priority:    {}", meta.priority);
            println!("Timeout:     {}s", meta.timeout);
            println!("Source:      {source:?}");
            println!("Path:        {}", parsed.source_path.display());
            println!("Eligible:    {}", elig.eligible);

            if !elig.eligible {
                if elig.missing_os {
                    println!("  Missing OS: {:?}", meta.requires.os);
                }
                if !elig.missing_bins.is_empty() {
                    println!("  Missing bins: {:?}", elig.missing_bins);
                }
                if !elig.missing_env.is_empty() {
                    println!("  Missing env: {:?}", elig.missing_env);
                }
            }

            if !parsed.body.is_empty() {
                println!("\n{}", parsed.body);
            }
        },
    }

    Ok(())
}
