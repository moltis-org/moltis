//! CLI subcommand for importing data from an OpenClaw installation.

use clap::Subcommand;

#[derive(Subcommand)]
pub enum ImportAction {
    /// Detect an OpenClaw installation and show what can be imported.
    Detect {
        /// Emit structured JSON output.
        #[arg(long)]
        json: bool,
    },
    /// Import all categories from OpenClaw.
    All {
        /// Dry-run: show what would be imported without writing anything.
        #[arg(long)]
        dry_run: bool,
        /// Emit structured JSON output.
        #[arg(long)]
        json: bool,
    },
    /// Import specific categories from OpenClaw.
    Select {
        /// Comma-separated list of categories to import.
        /// Valid: identity, providers, skills, memory, channels, sessions, workspace-files
        #[arg(short, long, value_delimiter = ',')]
        categories: Vec<String>,
        /// Dry-run: show what would be imported without writing anything.
        #[arg(long)]
        dry_run: bool,
        /// Emit structured JSON output.
        #[arg(long)]
        json: bool,
    },
}

pub async fn handle_import(action: ImportAction) -> anyhow::Result<()> {
    match action {
        ImportAction::Detect { json } => handle_detect(json),
        ImportAction::All { dry_run, json } => handle_import_all(dry_run, json),
        ImportAction::Select {
            categories,
            dry_run,
            json,
        } => handle_import_select(&categories, dry_run, json),
    }
}

fn handle_detect(json_output: bool) -> anyhow::Result<()> {
    let Some(detection) = moltis_openclaw_import::detect() else {
        if json_output {
            return print_json(serde_json::json!({
                "detected": false,
                "checked_paths": ["OPENCLAW_HOME", "~/.openclaw"],
            }));
        }
        println!("No OpenClaw installation found.");
        println!("Checked: ~/.openclaw/ and OPENCLAW_HOME environment variable.");
        return Ok(());
    };

    let scan = moltis_openclaw_import::scan(&detection);
    if json_output {
        return print_json(serde_json::json!({
            "detected": true,
            "home_dir": detection.home_dir.display().to_string(),
            "scan": scan,
            "multiple_agents_detected": scan.agent_ids.len() > 1,
        }));
    }

    println!(
        "OpenClaw installation detected at: {}",
        detection.home_dir.display()
    );
    println!();

    println!("Available data:");
    print_scan_item("Identity", scan.identity_available, None);
    print_scan_item("Providers", scan.providers_available, None);
    print_scan_item(
        "Skills",
        scan.skills_count > 0,
        Some(format!("{} skill(s)", scan.skills_count)),
    );
    print_scan_item(
        "Memory",
        scan.memory_available,
        Some(format!("{} memory file(s)", scan.memory_files_count)),
    );
    print_scan_item(
        "Channels",
        scan.channels_available,
        format_channel_scan_detail(&scan),
    );
    print_scan_item(
        "Sessions",
        scan.sessions_count > 0,
        Some(format!("{} session(s)", scan.sessions_count)),
    );
    print_scan_item(
        "Workspace Files",
        scan.workspace_files_available,
        if scan.workspace_files_count > 0 {
            Some(scan.workspace_files_found.join(", "))
        } else {
            None
        },
    );

    if !scan.unsupported_channels.is_empty() {
        println!();
        println!(
            "Unsupported channels (TODO): {}",
            scan.unsupported_channels.join(", ")
        );
    }

    if scan.agent_ids.len() > 1 {
        println!();
        println!(
            "Multiple agents detected: {}. Only the default agent will be imported.",
            scan.agent_ids.join(", ")
        );
    }

    Ok(())
}

fn handle_import_all(dry_run: bool, json_output: bool) -> anyhow::Result<()> {
    let Some(detection) = moltis_openclaw_import::detect() else {
        if json_output {
            return print_json(serde_json::json!({ "detected": false }));
        }
        println!("No OpenClaw installation found.");
        return Ok(());
    };

    if dry_run {
        let scan = moltis_openclaw_import::scan(&detection);
        if json_output {
            return print_json(serde_json::json!({
                "detected": true,
                "dry_run": true,
                "selection": "all",
                "scan": scan,
            }));
        }
        println!("Dry run — showing what would be imported:");
        println!();
        print_scan_summary(&scan);
        return Ok(());
    }

    if !json_output {
        println!(
            "Importing from {} (read-only \u{2014} OpenClaw will not be modified)\u{2026}",
            detection.home_dir.display()
        );
        println!();
    }

    let config_dir = moltis_config::config_dir()
        .ok_or_else(|| anyhow::anyhow!("could not determine config directory"))?;
    let data_dir = moltis_config::data_dir();

    let report = moltis_openclaw_import::import(
        &detection,
        &moltis_openclaw_import::ImportSelection::all(),
        &config_dir,
        &data_dir,
    );

    if json_output {
        return print_json(serde_json::json!({
            "detected": true,
            "dry_run": false,
            "selection": "all",
            "report": report,
            "total_imported": report.total_imported(),
            "has_failures": report.has_failures(),
        }));
    }

    print_report(&report);
    Ok(())
}

fn handle_import_select(
    categories: &[String],
    dry_run: bool,
    json_output: bool,
) -> anyhow::Result<()> {
    let Some(detection) = moltis_openclaw_import::detect() else {
        if json_output {
            return print_json(serde_json::json!({ "detected": false }));
        }
        println!("No OpenClaw installation found.");
        return Ok(());
    };

    let parsed = parse_selection(categories, !json_output);

    if dry_run {
        if json_output {
            return print_json(serde_json::json!({
                "detected": true,
                "dry_run": true,
                "selection": parsed.selection,
                "unknown_categories": parsed.unknown_categories,
            }));
        }
        println!("Dry run — selected categories:");
        print_selection(&parsed.selection);
        return Ok(());
    }

    if !json_output {
        println!(
            "Importing from {} (read-only \u{2014} OpenClaw will not be modified)\u{2026}",
            detection.home_dir.display()
        );
        println!();
    }

    let config_dir = moltis_config::config_dir()
        .ok_or_else(|| anyhow::anyhow!("could not determine config directory"))?;
    let data_dir = moltis_config::data_dir();

    let report =
        moltis_openclaw_import::import(&detection, &parsed.selection, &config_dir, &data_dir);

    if json_output {
        return print_json(serde_json::json!({
            "detected": true,
            "dry_run": false,
            "selection": parsed.selection,
            "unknown_categories": parsed.unknown_categories,
            "report": report,
            "total_imported": report.total_imported(),
            "has_failures": report.has_failures(),
        }));
    }

    print_report(&report);
    Ok(())
}

struct ParsedSelection {
    selection: moltis_openclaw_import::ImportSelection,
    unknown_categories: Vec<String>,
}

fn parse_selection(categories: &[String], warn_unknown: bool) -> ParsedSelection {
    let mut sel = moltis_openclaw_import::ImportSelection::default();
    let mut unknown_categories = Vec::new();
    for cat in categories {
        match cat.trim().to_lowercase().as_str() {
            "identity" => sel.identity = true,
            "providers" => sel.providers = true,
            "skills" => sel.skills = true,
            "memory" => sel.memory = true,
            "channels" => sel.channels = true,
            "sessions" => sel.sessions = true,
            "workspace_files" | "workspace-files" => sel.workspace_files = true,
            other => {
                unknown_categories.push(other.to_string());
                if warn_unknown {
                    eprintln!("Warning: unknown category '{other}', skipping");
                }
            },
        }
    }
    ParsedSelection {
        selection: sel,
        unknown_categories,
    }
}

fn print_json(value: serde_json::Value) -> anyhow::Result<()> {
    println!("{}", serde_json::to_string_pretty(&value)?);
    Ok(())
}

fn print_scan_item(name: &str, available: bool, detail: Option<String>) {
    let status = if available {
        "+"
    } else {
        "-"
    };
    match detail {
        Some(d) if available => println!("  [{status}] {name}: {d}"),
        _ => println!("  [{status}] {name}"),
    }
}

fn print_scan_summary(scan: &moltis_openclaw_import::ImportScan) {
    print_scan_item("Identity", scan.identity_available, None);
    print_scan_item("Providers", scan.providers_available, None);
    print_scan_item(
        "Skills",
        scan.skills_count > 0,
        Some(format!("{} skill(s)", scan.skills_count)),
    );
    print_scan_item(
        "Memory",
        scan.memory_available,
        Some(format!("{} memory file(s)", scan.memory_files_count)),
    );
    print_scan_item(
        "Channels",
        scan.channels_available,
        format_channel_scan_detail(scan),
    );
    print_scan_item(
        "Sessions",
        scan.sessions_count > 0,
        Some(format!("{} session(s)", scan.sessions_count)),
    );
    print_scan_item(
        "Workspace Files",
        scan.workspace_files_available,
        if scan.workspace_files_count > 0 {
            Some(scan.workspace_files_found.join(", "))
        } else {
            None
        },
    );
}

fn print_selection(sel: &moltis_openclaw_import::ImportSelection) {
    let items = [
        ("Identity", sel.identity),
        ("Providers", sel.providers),
        ("Skills", sel.skills),
        ("Memory", sel.memory),
        ("Channels", sel.channels),
        ("Sessions", sel.sessions),
        ("Workspace Files", sel.workspace_files),
    ];
    for (name, enabled) in items {
        let mark = if enabled {
            "x"
        } else {
            " "
        };
        println!("  [{mark}] {name}");
    }
}

fn format_channel_scan_detail(scan: &moltis_openclaw_import::ImportScan) -> Option<String> {
    let mut parts = Vec::new();
    if scan.telegram_accounts > 0 {
        parts.push(format!("{} Telegram account(s)", scan.telegram_accounts));
    }
    if scan.discord_accounts > 0 {
        parts.push(format!("{} Discord account(s)", scan.discord_accounts));
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(", "))
    }
}

fn print_report(report: &moltis_openclaw_import::report::ImportReport) {
    use moltis_openclaw_import::report::ImportStatus;

    println!("Import complete! Your OpenClaw installation was not modified.");
    println!();

    for cat in &report.categories {
        let icon = match cat.status {
            ImportStatus::Success => "+",
            ImportStatus::Partial => "~",
            ImportStatus::Skipped => "-",
            ImportStatus::Failed => "!",
        };
        if cat.items_updated > 0 {
            println!(
                "  [{icon}] {}: {} imported, {} updated, {} skipped",
                cat.category, cat.items_imported, cat.items_updated, cat.items_skipped,
            );
        } else {
            println!(
                "  [{icon}] {}: {} imported, {} skipped",
                cat.category, cat.items_imported, cat.items_skipped,
            );
        }
        for w in &cat.warnings {
            println!("      warning: {w}");
        }
        for e in &cat.errors {
            println!("      error: {e}");
        }
    }

    if !report.todos.is_empty() {
        println!();
        println!("TODO (not yet supported in Moltis):");
        for todo in &report.todos {
            println!("  - {}: {}", todo.feature, todo.description);
        }
    }

    println!();
    println!("Total imported: {} items", report.total_imported());
}
