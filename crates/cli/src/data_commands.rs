//! CLI subcommand for exporting and importing Moltis data archives.

use std::path::PathBuf;

use clap::Subcommand;

#[derive(Subcommand)]
pub enum DataAction {
    /// Export config, databases, and sessions to a .tar.gz archive.
    Export {
        /// Output file path (default: moltis-backup-<timestamp>.tar.gz in current directory).
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Include session media files (audio, images). Can make archives large.
        #[arg(long, default_value_t = false)]
        include_media: bool,
        /// Exclude provider API keys from the export.
        #[arg(long, default_value_t = false)]
        no_provider_keys: bool,
    },
    /// Import data from a .tar.gz archive.
    Import {
        /// Path to the archive file.
        archive: PathBuf,
        /// Preview what would be imported without writing anything.
        #[arg(long, default_value_t = false)]
        dry_run: bool,
        /// Conflict strategy: skip (default) or overwrite.
        #[arg(long, default_value = "skip")]
        conflict: String,
        /// Emit structured JSON output.
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    /// Inspect an archive and show its manifest without importing.
    Inspect {
        /// Path to the archive file.
        archive: PathBuf,
        /// Emit structured JSON output.
        #[arg(long, default_value_t = false)]
        json: bool,
    },
}

pub async fn handle_data(action: DataAction) -> anyhow::Result<()> {
    match action {
        DataAction::Export {
            output,
            include_media,
            no_provider_keys,
        } => handle_export(output, include_media, no_provider_keys).await,
        DataAction::Import {
            archive,
            dry_run,
            conflict,
            json,
        } => handle_import(archive, dry_run, &conflict, json).await,
        DataAction::Inspect { archive, json } => handle_inspect(archive, json),
    }
}

async fn handle_export(
    output: Option<PathBuf>,
    include_media: bool,
    no_provider_keys: bool,
) -> anyhow::Result<()> {
    let config_dir =
        moltis_config::config_dir().ok_or_else(|| anyhow::anyhow!("config directory not set"))?;
    let data_dir = moltis_config::data_dir();

    let opts = moltis_portable::ExportOptions {
        include_provider_keys: !no_provider_keys,
        include_media,
    };

    let output_path = output.unwrap_or_else(|| {
        let now = time::OffsetDateTime::now_utc();
        PathBuf::from(format!(
            "moltis-backup-{:04}{:02}{:02}-{:02}{:02}{:02}.tar.gz",
            now.year(),
            now.month() as u8,
            now.day(),
            now.hour(),
            now.minute(),
            now.second(),
        ))
    });

    let file = std::fs::File::create(&output_path)?;
    let manifest = moltis_portable::export_archive(&config_dir, &data_dir, &opts, file).await?;

    println!("Export complete: {}", output_path.display());
    println!("  Config files: {}", manifest.inventory.config_files.len());
    println!(
        "  Workspace files: {}",
        manifest.inventory.workspace_files.len()
    );
    println!("  Sessions: {}", manifest.inventory.session_count());
    if include_media {
        println!("  Media files: {}", manifest.inventory.media_count());
    }
    println!("  moltis.db: {}", manifest.inventory.has_moltis_db);
    println!("  memory.db: {}", manifest.inventory.has_memory_db);

    Ok(())
}

async fn handle_import(
    archive: PathBuf,
    dry_run: bool,
    conflict: &str,
    json: bool,
) -> anyhow::Result<()> {
    let config_dir =
        moltis_config::config_dir().ok_or_else(|| anyhow::anyhow!("config directory not set"))?;
    let data_dir = moltis_config::data_dir();

    let conflict_strategy = match conflict {
        "skip" => moltis_portable::ConflictStrategy::Skip,
        "overwrite" => moltis_portable::ConflictStrategy::Overwrite,
        other => anyhow::bail!("unknown conflict strategy: {other} (use 'skip' or 'overwrite')"),
    };

    let opts = moltis_portable::ImportOptions {
        conflict: conflict_strategy,
        dry_run,
    };

    let file = std::fs::File::open(&archive)?;
    let result = moltis_portable::import_archive(&config_dir, &data_dir, &opts, file).await?;

    if json {
        println!("{}", serde_json::to_string_pretty(&result)?);
        return Ok(());
    }

    if dry_run {
        println!("Dry run — no changes written.");
    }

    println!("Imported: {}", result.imported.len());
    for item in &result.imported {
        println!("  [{}] {} — {}", item.category, item.path, item.action);
    }

    if !result.skipped.is_empty() {
        println!("Skipped: {}", result.skipped.len());
        for item in &result.skipped {
            println!("  [{}] {} — {}", item.category, item.path, item.action);
        }
    }

    if !result.warnings.is_empty() {
        println!("Warnings:");
        for w in &result.warnings {
            println!("  {w}");
        }
    }

    Ok(())
}

fn handle_inspect(archive: PathBuf, json: bool) -> anyhow::Result<()> {
    let file = std::fs::File::open(&archive)?;
    let manifest = moltis_portable::inspect_archive(file)?;

    if json {
        println!("{}", serde_json::to_string_pretty(&manifest)?);
        return Ok(());
    }

    println!("Archive format version: {}", manifest.format_version);
    println!("Moltis version: {}", manifest.moltis_version);
    println!("Created: {}", manifest.created_at);
    println!("Contents:");
    println!("  Config files: {}", manifest.inventory.config_files.len());
    for f in &manifest.inventory.config_files {
        println!("    {f}");
    }
    println!(
        "  Workspace files: {}",
        manifest.inventory.workspace_files.len()
    );
    for f in &manifest.inventory.workspace_files {
        println!("    {f}");
    }
    println!("  moltis.db: {}", manifest.inventory.has_moltis_db);
    println!("  memory.db: {}", manifest.inventory.has_memory_db);
    println!("  Sessions: {}", manifest.inventory.session_count());
    println!("  Media files: {}", manifest.inventory.media_count());

    Ok(())
}
