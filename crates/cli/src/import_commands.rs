//! CLI subcommand for importing data from external AI tools.

use clap::Subcommand;

#[derive(clap::ValueEnum, Clone, Debug)]
pub enum ImportSource {
    /// Import from OpenClaw.
    Openclaw,
    /// Import from Claude Code and Claude Desktop.
    Claude,
    /// Import from Hermes.
    Hermes,
}

#[derive(Subcommand)]
pub enum ImportAction {
    /// Detect available import sources and show what can be imported.
    Detect {
        /// Only detect a specific source.
        #[arg(short, long)]
        source: Option<ImportSource>,
        /// Emit structured JSON output.
        #[arg(long)]
        json: bool,
    },
    /// Import all categories from detected sources.
    All {
        /// Only import from a specific source.
        #[arg(short, long)]
        source: Option<ImportSource>,
        /// Dry-run: show what would be imported without writing anything.
        #[arg(long)]
        dry_run: bool,
        /// Emit structured JSON output.
        #[arg(long)]
        json: bool,
    },
    /// Import specific categories from a source.
    Select {
        /// Source to import from (required for selective import).
        #[arg(short, long)]
        source: ImportSource,
        /// Comma-separated list of categories to import.
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
        ImportAction::Detect { source, json } => handle_detect(source, json),
        ImportAction::All {
            source,
            dry_run,
            json,
        } => handle_import_all(source, dry_run, json),
        ImportAction::Select {
            source,
            categories,
            dry_run,
            json,
        } => handle_import_select(source, &categories, dry_run, json),
    }
}

// ── Detection ────────────────────────────────────────────────────────────────

fn handle_detect(source: Option<ImportSource>, json_output: bool) -> anyhow::Result<()> {
    let mut results = serde_json::Map::new();
    let mut any_found = false;

    if source.is_none() || matches!(source, Some(ImportSource::Openclaw)) {
        let found = detect_openclaw(json_output, &mut results);
        any_found |= found;
    }

    if source.is_none() || matches!(source, Some(ImportSource::Claude)) {
        let found = detect_claude(json_output, &mut results);
        any_found |= found;
    }

    if source.is_none() || matches!(source, Some(ImportSource::Hermes)) {
        let found = detect_hermes(json_output, &mut results);
        any_found |= found;
    }

    if json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::Value::Object(results))?
        );
    } else if !any_found {
        println!("No import sources detected.");
        println!("Checked: OpenClaw (~/.openclaw/), Claude Code (~/.claude/), Hermes (~/.hermes/)");
    }

    Ok(())
}

#[cfg_attr(not(feature = "openclaw-import"), allow(unused_variables))]
fn detect_openclaw(
    json_output: bool,
    results: &mut serde_json::Map<String, serde_json::Value>,
) -> bool {
    #[cfg(feature = "openclaw-import")]
    {
        let Some(detection) = moltis_openclaw_import::detect() else {
            if !json_output {
                println!("OpenClaw: not detected");
            }
            results.insert(
                "openclaw".to_string(),
                serde_json::json!({"detected": false}),
            );
            return false;
        };

        let scan = moltis_openclaw_import::scan(&detection);
        if json_output {
            results.insert(
                "openclaw".to_string(),
                serde_json::json!({
                    "detected": true,
                    "home_dir": detection.home_dir.display().to_string(),
                    "scan": scan,
                }),
            );
        } else {
            println!("OpenClaw: detected at {}", detection.home_dir.display());
            print_scan_item("  Identity", scan.identity_available, None);
            print_scan_item("  Providers", scan.providers_available, None);
            print_scan_item(
                "  Skills",
                scan.skills_count > 0,
                Some(format!("{} skill(s)", scan.skills_count)),
            );
            print_scan_item(
                "  Memory",
                scan.memory_available,
                Some(format!("{} file(s)", scan.memory_files_count)),
            );
            print_scan_item(
                "  Channels",
                scan.channels_available,
                format_channel_detail(&scan),
            );
            print_scan_item(
                "  Sessions",
                scan.sessions_count > 0,
                Some(format!("{} session(s)", scan.sessions_count)),
            );
            print_scan_item("  Workspace Files", scan.workspace_files_available, None);
            print_scan_item("  MCP Servers", scan.mcp_servers_available, None);
            println!();
        }
        true
    }
    #[cfg(not(feature = "openclaw-import"))]
    {
        false
    }
}

#[cfg_attr(not(feature = "claude-import"), allow(unused_variables))]
fn detect_claude(
    json_output: bool,
    results: &mut serde_json::Map<String, serde_json::Value>,
) -> bool {
    #[cfg(feature = "claude-import")]
    {
        let Some(detection) = moltis_claude_import::detect::detect() else {
            if !json_output {
                println!("Claude Code: not detected");
            }
            results.insert("claude".to_string(), serde_json::json!({"detected": false}));
            return false;
        };

        let skills = moltis_claude_import::skills::discover_skills(&detection);
        let commands = moltis_claude_import::skills::discover_commands(&detection);

        if json_output {
            results.insert(
                "claude".to_string(),
                serde_json::json!({
                    "detected": true,
                    "has_settings": detection.user_settings_path.is_some(),
                    "has_claude_json": detection.user_claude_json_path.is_some(),
                    "has_desktop_config": detection.desktop_config_path.is_some(),
                    "skills_count": skills.len(),
                    "commands_count": commands.len(),
                    "has_memory": detection.user_memory_path.is_some(),
                }),
            );
        } else {
            println!("Claude Code: detected");
            print_scan_item(
                "  MCP Servers",
                detection.user_claude_json_path.is_some()
                    || detection.desktop_config_path.is_some(),
                None,
            );
            print_scan_item(
                "  Skills",
                !skills.is_empty(),
                Some(format!("{} skill(s)", skills.len())),
            );
            print_scan_item(
                "  Commands",
                !commands.is_empty(),
                Some(format!("{} command(s) -> skills", commands.len())),
            );
            print_scan_item("  Memory", detection.user_memory_path.is_some(), None);
            println!();
        }
        true
    }
    #[cfg(not(feature = "claude-import"))]
    {
        false
    }
}

#[cfg_attr(not(feature = "hermes-import"), allow(unused_variables))]
fn detect_hermes(
    json_output: bool,
    results: &mut serde_json::Map<String, serde_json::Value>,
) -> bool {
    #[cfg(feature = "hermes-import")]
    {
        let Some(detection) = moltis_hermes_import::detect::detect() else {
            if !json_output {
                println!("Hermes: not detected");
            }
            results.insert("hermes".to_string(), serde_json::json!({"detected": false}));
            return false;
        };

        let creds = moltis_hermes_import::credentials::discover_credentials(&detection);
        let skills = moltis_hermes_import::skills::discover_skills(&detection);
        let has_memory = detection.soul_path.is_some()
            || detection.memory_path.is_some()
            || detection.agents_path.is_some()
            || detection.user_path.is_some();

        if json_output {
            results.insert(
                "hermes".to_string(),
                serde_json::json!({
                    "detected": true,
                    "home_dir": detection.home_dir.display().to_string(),
                    "has_config": detection.config_path.is_some(),
                    "credentials_count": creds.len(),
                    "skills_count": skills.len(),
                    "has_memory": has_memory,
                }),
            );
        } else {
            println!("Hermes: detected at {}", detection.home_dir.display());
            print_scan_item(
                "  Credentials",
                !creds.is_empty(),
                Some(format!("{} provider(s)", creds.len())),
            );
            print_scan_item(
                "  Skills",
                !skills.is_empty(),
                Some(format!("{} skill(s)", skills.len())),
            );
            print_scan_item("  Memory", has_memory, None);
            println!();
        }
        true
    }
    #[cfg(not(feature = "hermes-import"))]
    {
        false
    }
}

// ── Import All ───────────────────────────────────────────────────────────────

fn handle_import_all(
    source: Option<ImportSource>,
    dry_run: bool,
    json_output: bool,
) -> anyhow::Result<()> {
    if dry_run {
        return handle_detect(source, json_output);
    }

    let config_dir = moltis_config::config_dir()
        .ok_or_else(|| anyhow::anyhow!("could not determine config directory"))?;
    let data_dir = moltis_config::data_dir();

    let mut all_results = serde_json::Map::new();

    if source.is_none() || matches!(source, Some(ImportSource::Openclaw)) {
        import_openclaw_all(&config_dir, &data_dir, json_output, &mut all_results)?;
    }

    if source.is_none() || matches!(source, Some(ImportSource::Claude)) {
        import_claude_all(&data_dir, json_output, &mut all_results)?;
    }

    if source.is_none() || matches!(source, Some(ImportSource::Hermes)) {
        import_hermes_all(&config_dir, &data_dir, json_output, &mut all_results)?;
    }

    if json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::Value::Object(all_results))?
        );
    }

    Ok(())
}

#[cfg_attr(not(feature = "openclaw-import"), allow(unused_variables))]
fn import_openclaw_all(
    config_dir: &std::path::Path,
    data_dir: &std::path::Path,
    json_output: bool,
    results: &mut serde_json::Map<String, serde_json::Value>,
) -> anyhow::Result<()> {
    #[cfg(feature = "openclaw-import")]
    {
        let Some(detection) = moltis_openclaw_import::detect() else {
            if !json_output {
                println!("OpenClaw: not detected, skipping");
            }
            return Ok(());
        };

        if !json_output {
            println!(
                "Importing from OpenClaw at {} ...",
                detection.home_dir.display()
            );
        }

        let report = moltis_openclaw_import::import(
            &detection,
            &moltis_openclaw_import::ImportSelection::all(),
            config_dir,
            data_dir,
        );

        if json_output {
            results.insert(
                "openclaw".to_string(),
                serde_json::json!({
                    "report": report,
                    "total_imported": report.total_imported(),
                }),
            );
        } else {
            print_report("OpenClaw", &report.categories);
        }
    }
    Ok(())
}

#[cfg_attr(not(feature = "claude-import"), allow(unused_variables))]
fn import_claude_all(
    data_dir: &std::path::Path,
    json_output: bool,
    results: &mut serde_json::Map<String, serde_json::Value>,
) -> anyhow::Result<()> {
    #[cfg(feature = "claude-import")]
    {
        let Some(detection) = moltis_claude_import::detect::detect() else {
            if !json_output {
                println!("Claude Code: not detected, skipping");
            }
            return Ok(());
        };

        if !json_output {
            println!("Importing from Claude Code ...");
        }

        let mcp_path = data_dir.join("mcp-servers.json");
        let skills_dir = data_dir.join("skills");

        let categories = vec![
            moltis_claude_import::mcp_servers::import_mcp_servers(&detection, &mcp_path),
            moltis_claude_import::skills::import_skills(&detection, &skills_dir),
            moltis_claude_import::memory::import_memory(&detection, data_dir),
        ];

        let total: usize = categories.iter().map(|c| c.items_imported).sum();

        if json_output {
            results.insert(
                "claude".to_string(),
                serde_json::json!({
                    "categories": categories,
                    "total_imported": total,
                }),
            );
        } else {
            print_report("Claude Code", &categories);
        }
    }
    Ok(())
}

#[cfg_attr(not(feature = "hermes-import"), allow(unused_variables))]
fn import_hermes_all(
    config_dir: &std::path::Path,
    data_dir: &std::path::Path,
    json_output: bool,
    results: &mut serde_json::Map<String, serde_json::Value>,
) -> anyhow::Result<()> {
    #[cfg(feature = "hermes-import")]
    {
        let Some(detection) = moltis_hermes_import::detect::detect() else {
            if !json_output {
                println!("Hermes: not detected, skipping");
            }
            return Ok(());
        };

        if !json_output {
            println!(
                "Importing from Hermes at {} ...",
                detection.home_dir.display()
            );
        }

        let skills_dir = data_dir.join("skills");

        let categories = vec![
            moltis_hermes_import::credentials::import_credentials(&detection, config_dir),
            moltis_hermes_import::skills::import_skills(&detection, &skills_dir),
            moltis_hermes_import::memory::import_memory(&detection, data_dir),
        ];

        let total: usize = categories.iter().map(|c| c.items_imported).sum();

        if json_output {
            results.insert(
                "hermes".to_string(),
                serde_json::json!({
                    "categories": categories,
                    "total_imported": total,
                }),
            );
        } else {
            print_report("Hermes", &categories);
        }
    }
    Ok(())
}

// ── Selective Import ─────────────────────────────────────────────────────────

fn handle_import_select(
    source: ImportSource,
    categories: &[String],
    dry_run: bool,
    json_output: bool,
) -> anyhow::Result<()> {
    if dry_run {
        return handle_detect(Some(source), json_output);
    }

    let config_dir = moltis_config::config_dir()
        .ok_or_else(|| anyhow::anyhow!("could not determine config directory"))?;
    let data_dir = moltis_config::data_dir();

    match source {
        ImportSource::Openclaw => {
            import_openclaw_select(categories, &config_dir, &data_dir, json_output)
        },
        ImportSource::Claude => import_claude_select(categories, &data_dir, json_output),
        ImportSource::Hermes => {
            import_hermes_select(categories, &config_dir, &data_dir, json_output)
        },
    }
}

#[cfg_attr(not(feature = "openclaw-import"), allow(unused_variables))]
fn import_openclaw_select(
    categories: &[String],
    config_dir: &std::path::Path,
    data_dir: &std::path::Path,
    json_output: bool,
) -> anyhow::Result<()> {
    #[cfg(feature = "openclaw-import")]
    {
        let Some(detection) = moltis_openclaw_import::detect() else {
            anyhow::bail!("No OpenClaw installation found");
        };

        let selection = parse_openclaw_selection(categories);

        let report = moltis_openclaw_import::import(&detection, &selection, config_dir, data_dir);

        if json_output {
            print_json(serde_json::json!({
                "source": "openclaw",
                "report": report,
                "total_imported": report.total_imported(),
            }))?;
        } else {
            print_report("OpenClaw", &report.categories);
        }
    }
    #[cfg(not(feature = "openclaw-import"))]
    anyhow::bail!("openclaw-import feature is not enabled");
    #[allow(unreachable_code)]
    Ok(())
}

#[cfg(feature = "openclaw-import")]
fn parse_openclaw_selection(categories: &[String]) -> moltis_openclaw_import::ImportSelection {
    let mut sel = moltis_openclaw_import::ImportSelection::default();
    for cat in categories {
        match cat.trim().to_lowercase().as_str() {
            "identity" => sel.identity = true,
            "providers" | "credentials" => sel.providers = true,
            "skills" => sel.skills = true,
            "memory" => sel.memory = true,
            "channels" => sel.channels = true,
            "sessions" => sel.sessions = true,
            "workspace_files" | "workspace-files" => sel.workspace_files = true,
            "mcp_servers" | "mcp-servers" | "mcp" => sel.mcp_servers = true,
            other => eprintln!("Warning: unknown category '{other}' for openclaw, skipping"),
        }
    }
    sel
}

#[cfg_attr(not(feature = "claude-import"), allow(unused_variables))]
fn import_claude_select(
    categories: &[String],
    data_dir: &std::path::Path,
    json_output: bool,
) -> anyhow::Result<()> {
    #[cfg(feature = "claude-import")]
    {
        let Some(detection) = moltis_claude_import::detect::detect() else {
            anyhow::bail!("No Claude Code installation found");
        };

        let cats: Vec<String> = categories.iter().map(|c| c.trim().to_lowercase()).collect();

        let mcp_path = data_dir.join("mcp-servers.json");
        let skills_dir = data_dir.join("skills");

        let mut reports = Vec::new();
        for cat in &cats {
            match cat.as_str() {
                "mcp_servers" | "mcp-servers" | "mcp" => {
                    reports.push(moltis_claude_import::mcp_servers::import_mcp_servers(
                        &detection, &mcp_path,
                    ));
                },
                "skills" | "commands" => {
                    reports.push(moltis_claude_import::skills::import_skills(
                        &detection,
                        &skills_dir,
                    ));
                },
                "memory" => {
                    reports.push(moltis_claude_import::memory::import_memory(
                        &detection, data_dir,
                    ));
                },
                other => eprintln!("Warning: unknown category '{other}' for claude, skipping"),
            }
        }

        if json_output {
            let total: usize = reports.iter().map(|c| c.items_imported).sum();
            print_json(serde_json::json!({
                "source": "claude",
                "categories": reports,
                "total_imported": total,
            }))?;
        } else {
            print_report("Claude Code", &reports);
        }
    }
    #[cfg(not(feature = "claude-import"))]
    anyhow::bail!("claude-import feature is not enabled");
    #[allow(unreachable_code)]
    Ok(())
}

#[cfg_attr(not(feature = "hermes-import"), allow(unused_variables))]
fn import_hermes_select(
    categories: &[String],
    config_dir: &std::path::Path,
    data_dir: &std::path::Path,
    json_output: bool,
) -> anyhow::Result<()> {
    #[cfg(feature = "hermes-import")]
    {
        let Some(detection) = moltis_hermes_import::detect::detect() else {
            anyhow::bail!("No Hermes installation found");
        };

        let cats: Vec<String> = categories.iter().map(|c| c.trim().to_lowercase()).collect();

        let skills_dir = data_dir.join("skills");

        let mut reports = Vec::new();
        for cat in &cats {
            match cat.as_str() {
                "providers" | "credentials" => {
                    reports.push(moltis_hermes_import::credentials::import_credentials(
                        &detection, config_dir,
                    ));
                },
                "skills" => {
                    reports.push(moltis_hermes_import::skills::import_skills(
                        &detection,
                        &skills_dir,
                    ));
                },
                "memory" | "workspace-files" | "workspace_files" => {
                    reports.push(moltis_hermes_import::memory::import_memory(
                        &detection, data_dir,
                    ));
                },
                other => eprintln!("Warning: unknown category '{other}' for hermes, skipping"),
            }
        }

        if json_output {
            let total: usize = reports.iter().map(|c| c.items_imported).sum();
            print_json(serde_json::json!({
                "source": "hermes",
                "categories": reports,
                "total_imported": total,
            }))?;
        } else {
            print_report("Hermes", &reports);
        }
    }
    #[cfg(not(feature = "hermes-import"))]
    anyhow::bail!("hermes-import feature is not enabled");
    #[allow(unreachable_code)]
    Ok(())
}

// ── Output helpers ───────────────────────────────────────────────────────────

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

#[cfg(feature = "openclaw-import")]
fn format_channel_detail(scan: &moltis_openclaw_import::ImportScan) -> Option<String> {
    let mut parts = Vec::new();
    if scan.telegram_accounts > 0 {
        parts.push(format!("{} Telegram", scan.telegram_accounts));
    }
    if scan.discord_accounts > 0 {
        parts.push(format!("{} Discord", scan.discord_accounts));
    }
    if scan.signal_accounts > 0 {
        parts.push(format!("{} Signal", scan.signal_accounts));
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(", "))
    }
}

fn print_report(source: &str, categories: &[impl AsReport]) {
    println!();
    println!("{source} import complete:");
    for cat in categories {
        let (name, status, imported, updated, skipped, warnings, errors) = cat.as_report();
        let icon = match status {
            "success" => "+",
            "partial" => "~",
            "skipped" => "-",
            _ => "!",
        };
        if updated > 0 {
            println!(
                "  [{icon}] {name}: {imported} imported, {updated} updated, {skipped} skipped"
            );
        } else {
            println!("  [{icon}] {name}: {imported} imported, {skipped} skipped");
        }
        for w in warnings {
            println!("      warning: {w}");
        }
        for e in errors {
            println!("      error: {e}");
        }
    }
    println!();
}

/// Trait to unify report printing across different report types.
trait AsReport {
    fn as_report(&self) -> (&str, &str, usize, usize, usize, &[String], &[String]);
}

impl AsReport for moltis_import_core::report::CategoryReport {
    fn as_report(&self) -> (&str, &str, usize, usize, usize, &[String], &[String]) {
        let status = match self.status {
            moltis_import_core::report::ImportStatus::Success => "success",
            moltis_import_core::report::ImportStatus::Partial => "partial",
            moltis_import_core::report::ImportStatus::Skipped => "skipped",
            moltis_import_core::report::ImportStatus::Failed => "failed",
        };
        let name = match self.category {
            moltis_import_core::report::ImportCategory::Identity => "Identity",
            moltis_import_core::report::ImportCategory::Providers => "Providers",
            moltis_import_core::report::ImportCategory::Skills => "Skills",
            moltis_import_core::report::ImportCategory::Memory => "Memory",
            moltis_import_core::report::ImportCategory::Channels => "Channels",
            moltis_import_core::report::ImportCategory::Sessions => "Sessions",
            moltis_import_core::report::ImportCategory::McpServers => "MCP Servers",
            moltis_import_core::report::ImportCategory::WorkspaceFiles => "Workspace Files",
        };
        (
            name,
            status,
            self.items_imported,
            self.items_updated,
            self.items_skipped,
            &self.warnings,
            &self.errors,
        )
    }
}

#[cfg(feature = "openclaw-import")]
impl AsReport for moltis_openclaw_import::report::CategoryReport {
    fn as_report(&self) -> (&str, &str, usize, usize, usize, &[String], &[String]) {
        let status = match self.status {
            moltis_openclaw_import::report::ImportStatus::Success => "success",
            moltis_openclaw_import::report::ImportStatus::Partial => "partial",
            moltis_openclaw_import::report::ImportStatus::Skipped => "skipped",
            moltis_openclaw_import::report::ImportStatus::Failed => "failed",
        };
        (
            // Reuse Display impl
            match self.category {
                moltis_openclaw_import::report::ImportCategory::Identity => "Identity",
                moltis_openclaw_import::report::ImportCategory::Providers => "Providers",
                moltis_openclaw_import::report::ImportCategory::Skills => "Skills",
                moltis_openclaw_import::report::ImportCategory::Memory => "Memory",
                moltis_openclaw_import::report::ImportCategory::Channels => "Channels",
                moltis_openclaw_import::report::ImportCategory::Sessions => "Sessions",
                moltis_openclaw_import::report::ImportCategory::McpServers => "MCP Servers",
                moltis_openclaw_import::report::ImportCategory::WorkspaceFiles => "Workspace Files",
            },
            status,
            self.items_imported,
            self.items_updated,
            self.items_skipped,
            &self.warnings,
            &self.errors,
        )
    }
}
