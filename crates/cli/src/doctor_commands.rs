//! `moltis doctor` — health check, config validation, and environment audit.
//!
//! Runs a series of checks against the local installation and prints a
//! structured report with `[ok]`, `[warn]`, `[fail]`, `[skip]`, or `[info]`
//! status indicators per item.

use std::path::{Path, PathBuf};

use {
    anyhow::Result,
    moltis_config::{
        MoltisConfig,
        validate::{self, Severity},
    },
    secrecy::ExposeSecret,
    tokio::process::Command,
};

// ── ANSI helpers ────────────────────────────────────────────────────────────

const GREEN: &str = "\x1b[32m";
const RED: &str = "\x1b[31m";
const YELLOW: &str = "\x1b[33m";
const CYAN: &str = "\x1b[36m";
const DIM: &str = "\x1b[2m";
const BOLD: &str = "\x1b[1m";
const RESET: &str = "\x1b[0m";

/// Per-check result used to build the report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Status {
    Ok,
    Warn,
    Fail,
    Skip,
    Info,
}

impl Status {
    fn label(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Warn => "warn",
            Self::Fail => "fail",
            Self::Skip => "skip",
            Self::Info => "info",
        }
    }

    fn color(self) -> &'static str {
        match self {
            Self::Ok => GREEN,
            Self::Warn => YELLOW,
            Self::Fail => RED,
            Self::Skip => DIM,
            Self::Info => CYAN,
        }
    }
}

struct CheckItem {
    status: Status,
    message: String,
}

struct Section {
    title: String,
    items: Vec<CheckItem>,
}

impl Section {
    fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            items: Vec::new(),
        }
    }

    fn push(&mut self, status: Status, message: impl Into<String>) {
        self.items.push(CheckItem {
            status,
            message: message.into(),
        });
    }
}

// ── Printing ────────────────────────────────────────────────────────────────

fn print_report(sections: &[Section]) -> (usize, usize) {
    let mut errors = 0usize;
    let mut warnings = 0usize;

    for section in sections {
        eprintln!("{BOLD}{}{RESET}", section.title);
        for item in &section.items {
            let color = item.status.color();
            let label = item.status.label();
            eprintln!("  [{color}{label}{RESET}]  {}", item.message);
            match item.status {
                Status::Fail => errors += 1,
                Status::Warn => warnings += 1,
                _ => {},
            }
        }
        eprintln!();
    }

    (errors, warnings)
}

// ── Provider → env var mapping ──────────────────────────────────────────────

/// (provider_name, env_var, is_key_optional)
const PROVIDER_ENV_MAP: &[(&str, &str, bool)] = &[
    ("anthropic", "ANTHROPIC_API_KEY", false),
    ("openai", "OPENAI_API_KEY", false),
    ("gemini", "GEMINI_API_KEY", false),
    ("groq", "GROQ_API_KEY", false),
    ("xai", "XAI_API_KEY", false),
    ("deepseek", "DEEPSEEK_API_KEY", false),
    ("mistral", "MISTRAL_API_KEY", false),
    ("openrouter", "OPENROUTER_API_KEY", false),
    ("cerebras", "CEREBRAS_API_KEY", false),
    ("minimax", "MINIMAX_API_KEY", false),
    ("moonshot", "MOONSHOT_API_KEY", false),
    ("venice", "VENICE_API_KEY", false),
    ("ollama", "OLLAMA_API_KEY", true),
    ("kimi-code", "KIMI_API_KEY", false),
];

/// OAuth providers that don't use env var API keys.
const OAUTH_PROVIDERS: &[&str] = &["openai-codex", "github-copilot"];

// ── Entry point ─────────────────────────────────────────────────────────────

pub async fn handle_doctor() -> Result<()> {
    let config_dir = moltis_config::config_dir();
    let data_dir = moltis_config::data_dir();

    eprintln!("{BOLD}moltis doctor{RESET}");
    eprintln!("{BOLD}============={RESET}\n");

    let mut sections = Vec::new();

    // 1. Config validation
    sections.push(check_config(config_dir.as_deref()));

    // Load config for subsequent checks (best-effort)
    let config = moltis_config::discover_and_load();

    // 2. Security audit
    sections.push(check_security(&config, config_dir.as_deref(), &data_dir));

    // 3. Directory health
    sections.push(check_directories(config_dir.as_deref(), &data_dir));

    // 4. Database health
    sections.push(check_database(&data_dir).await);

    // 5. Provider readiness
    sections.push(check_providers(&config));

    // 6. TLS health
    #[cfg(feature = "tls")]
    sections.push(check_tls(&config));

    // 7. MCP server health
    sections.push(check_mcp_servers(&config));

    // 8. Remote execution readiness
    sections.push(check_remote_exec(&config, &data_dir).await);

    let (errors, warnings) = print_report(&sections);

    eprintln!("{BOLD}Summary:{RESET} {errors} error(s), {warnings} warning(s)");

    if errors > 0 {
        std::process::exit(1);
    }

    Ok(())
}

// ── 1. Config validation ────────────────────────────────────────────────────

fn check_config(config_dir: Option<&Path>) -> Section {
    let label = config_dir
        .map(|d| d.join("moltis.toml").display().to_string())
        .unwrap_or_else(|| "default config".into());
    let mut section = Section::new(format!("Config ({label})"));

    let result = validate::validate(None);

    // Bucket diagnostics by category for clearer reporting.
    let has_syntax_error = result
        .diagnostics
        .iter()
        .any(|d| d.category == "syntax" && d.severity == Severity::Error);

    if has_syntax_error {
        for d in &result.diagnostics {
            if d.category == "syntax" {
                section.push(Status::Fail, format!("TOML syntax: {}", d.message));
            }
        }
        // Can't do further checks with broken syntax
        return section;
    }

    section.push(Status::Ok, "TOML syntax valid");

    let unknown_fields: Vec<_> = result
        .diagnostics
        .iter()
        .filter(|d| d.category == "unknown-field")
        .collect();
    if unknown_fields.is_empty() {
        section.push(Status::Ok, "All fields recognized");
    } else {
        for d in &unknown_fields {
            section.push(Status::Fail, format!("{}: {}", d.path, d.message));
        }
    }

    // Semantic warnings (security, deprecated fields, etc.)
    for d in &result.diagnostics {
        if let Some(status) = config_validation_status(d) {
            let msg = if d.path.is_empty() {
                d.message.clone()
            } else {
                format!("{}: {}", d.path, d.message)
            };
            section.push(status, msg);
        }
    }

    // Type errors
    let type_errors: Vec<_> = result
        .diagnostics
        .iter()
        .filter(|d| d.category == "type-error")
        .collect();
    if type_errors.is_empty() {
        section.push(Status::Ok, "No type errors");
    } else {
        for d in &type_errors {
            section.push(Status::Fail, d.message.clone());
        }
    }

    // File-ref warnings
    for d in &result.diagnostics {
        if d.category == "file-ref" && d.severity != Severity::Info {
            section.push(Status::Warn, format!("{}: {}", d.path, d.message));
        }
    }

    section
}

fn config_validation_status(diagnostic: &moltis_config::Diagnostic) -> Option<Status> {
    if diagnostic.category != "security"
        && diagnostic.category != "unknown-provider"
        && diagnostic.category != "deprecated-field"
    {
        return None;
    }

    Some(match diagnostic.severity {
        Severity::Error => Status::Fail,
        Severity::Warning => Status::Warn,
        Severity::Info => Status::Info,
    })
}

// ── 2. Security audit ───────────────────────────────────────────────────────

fn check_security(config: &MoltisConfig, config_dir: Option<&Path>, data_dir: &Path) -> Section {
    let mut section = Section::new("Security");

    // Check for API keys in config file (should use env vars or credential store)
    let mut api_keys_in_config = Vec::new();
    for (name, entry) in &config.providers.providers {
        if let Some(ref key) = entry.api_key
            && !key.expose_secret().is_empty()
        {
            api_keys_in_config.push(name.clone());
        }
    }
    if api_keys_in_config.is_empty() {
        section.push(Status::Ok, "No API keys in config file");
    } else {
        section.push(
            Status::Warn,
            format!(
                "API keys found in config for: {}. Use env vars or provider setup instead",
                api_keys_in_config.join(", ")
            ),
        );
    }

    // Unix file permission checks
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        // Config file permissions
        if let Some(dir) = config_dir {
            let config_file = dir.join("moltis.toml");
            if let Ok(meta) = std::fs::metadata(&config_file) {
                let mode = meta.permissions().mode();
                if mode & 0o044 != 0 {
                    section.push(
                        Status::Warn,
                        format!(
                            "Config file is world/group-readable (mode {:#05o}, expected 0600)",
                            mode & 0o777
                        ),
                    );
                } else {
                    section.push(Status::Ok, "Config file permissions");
                }
            }

            // Credentials file permissions
            let creds_file = dir.join("credentials.json");
            if creds_file.exists()
                && let Ok(meta) = std::fs::metadata(&creds_file)
            {
                let mode = meta.permissions().mode();
                if mode & 0o044 != 0 {
                    section.push(
                        Status::Warn,
                        format!(
                            "Credentials file is world/group-readable (mode {:#05o}, expected 0600)",
                            mode & 0o777
                        ),
                    );
                } else {
                    section.push(Status::Ok, "Credentials file permissions");
                }
            }
        }

        // Data directory permissions
        if let Ok(meta) = std::fs::metadata(data_dir) {
            let mode = meta.permissions().mode();
            if mode & 0o007 != 0 {
                section.push(
                    Status::Warn,
                    format!(
                        "Data directory is world-accessible (mode {:#05o}, expected 0700)",
                        mode & 0o777
                    ),
                );
            } else {
                section.push(Status::Ok, "Data directory permissions");
            }
        }
    }

    section
}

// ── 3. Directory health ─────────────────────────────────────────────────────

fn check_directories(config_dir: Option<&Path>, data_dir: &Path) -> Section {
    let mut section = Section::new("Directories");

    // Config directory
    match config_dir {
        Some(dir) if dir.is_dir() => {
            section.push(Status::Ok, format!("Config directory: {}", dir.display()));
        },
        Some(dir) => {
            section.push(
                Status::Fail,
                format!("Config directory missing: {}", dir.display()),
            );
        },
        None => {
            section.push(Status::Fail, "Unable to resolve config directory");
        },
    }

    // Data directory
    if data_dir.is_dir() {
        section.push(
            Status::Ok,
            format!("Data directory: {}", data_dir.display()),
        );
    } else {
        section.push(
            Status::Fail,
            format!("Data directory missing: {}", data_dir.display()),
        );
    }

    // Writable checks
    if let Some(dir) = config_dir {
        check_writable(&mut section, dir, "Config directory");
    }
    check_writable(&mut section, data_dir, "Data directory");

    // Check for expected files
    if let Some(dir) = config_dir {
        let config_file = dir.join("moltis.toml");
        if config_file.exists() {
            section.push(Status::Ok, "moltis.toml present");
        } else {
            section.push(Status::Info, "moltis.toml not found (using defaults)");
        }
    }

    let db_file = data_dir.join("moltis.db");
    if db_file.exists() {
        section.push(Status::Ok, "moltis.db present");
    } else {
        section.push(
            Status::Info,
            "moltis.db not found (will be created on first gateway start)",
        );
    }

    section
}

fn check_writable(section: &mut Section, dir: &Path, label: &str) {
    let probe = dir.join(".moltis-doctor-probe");
    match std::fs::write(&probe, b"probe") {
        Ok(()) => {
            let _ = std::fs::remove_file(&probe);
            // Only report if not already reported as existing
        },
        Err(e) => {
            section.push(Status::Fail, format!("{label} is not writable: {e}"));
        },
    }
}

// ── 4. Database health ──────────────────────────────────────────────────────

async fn check_database(data_dir: &Path) -> Section {
    let mut section = Section::new("Database");

    let db_path = data_dir.join("moltis.db");
    if !db_path.exists() {
        section.push(
            Status::Skip,
            "moltis.db not found (skipping connectivity check)",
        );
        return section;
    }

    let db_url = format!("sqlite:{}?mode=ro", db_path.display());
    match sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&db_url)
        .await
    {
        Ok(pool) => {
            match sqlx::query_scalar::<_, i32>("SELECT 1")
                .fetch_one(&pool)
                .await
            {
                Ok(_) => {
                    section.push(Status::Ok, "Database accessible (SELECT 1 OK)");
                },
                Err(e) => {
                    section.push(Status::Fail, format!("Database query failed: {e}"));
                },
            }
            pool.close().await;
        },
        Err(e) => {
            section.push(Status::Fail, format!("Cannot open database: {e}"));
        },
    }

    section
}

// ── 5. Provider readiness ───────────────────────────────────────────────────

fn check_providers(config: &MoltisConfig) -> Section {
    let mut section = Section::new("Providers");

    if config.providers.providers.is_empty() {
        section.push(Status::Info, "No providers configured");
        return section;
    }

    for (name, entry) in &config.providers.providers {
        if !entry.enabled {
            section.push(Status::Skip, format!("{name}: disabled"));
            continue;
        }

        // OAuth providers — skip env var check
        if OAUTH_PROVIDERS.contains(&name.as_str()) {
            section.push(
                Status::Skip,
                format!("{name}: OAuth (check via auth login)"),
            );
            continue;
        }

        // Check if API key available: config or env var
        let has_config_key = entry
            .api_key
            .as_ref()
            .is_some_and(|k| !k.expose_secret().is_empty());

        let env_info = PROVIDER_ENV_MAP
            .iter()
            .find(|(pname, ..)| *pname == name.as_str());

        let has_env_key = env_info.is_some_and(|(_, env, _)| std::env::var(env).is_ok())
            || (name == "gemini" && std::env::var("GOOGLE_API_KEY").is_ok());
        let is_optional = env_info.is_some_and(|(_, _, opt)| *opt);

        if has_config_key || has_env_key {
            section.push(Status::Ok, format!("{name}: API key available"));
        } else if is_optional {
            section.push(
                Status::Info,
                format!("{name}: no key required (local server)"),
            );
        } else {
            let hint = env_info
                .map(|(_, env, _)| {
                    format!("{name}: no API key found (set {env} or configure in provider setup)")
                })
                .unwrap_or_else(|| format!("{name}: no API key found (unknown provider)"));
            section.push(Status::Warn, hint);
        }
    }

    section
}

// ── 6. TLS health ───────────────────────────────────────────────────────────

#[cfg(feature = "tls")]
fn check_tls(config: &MoltisConfig) -> Section {
    let mut section = Section::new("TLS");

    if !config.tls.enabled {
        section.push(Status::Skip, "TLS disabled in config");
        return section;
    }

    // Custom cert/key paths
    if let (Some(cert_path), Some(key_path)) = (&config.tls.cert_path, &config.tls.key_path) {
        check_file_readable(&mut section, cert_path, "Custom certificate");
        check_file_readable(&mut section, key_path, "Custom private key");
        return section;
    }

    // Auto-generated certs
    if config.tls.auto_generate {
        match moltis_httpd::tls::cert_dir() {
            Ok(cert_dir) => {
                let ca_path = cert_dir.join("ca.pem");
                let server_cert = cert_dir.join("server.pem");
                let server_key = cert_dir.join("server-key.pem");

                if ca_path.exists() && server_cert.exists() && server_key.exists() {
                    section.push(Status::Ok, "Auto-generated certificates present");

                    // Check cert age as proxy for expiry
                    if let Some(days) = cert_age_days(&server_cert) {
                        // Certs are generated with ~365 day validity
                        let remaining = 365i64.saturating_sub(days);
                        if remaining < 30 {
                            section.push(
                                Status::Warn,
                                format!(
                                    "Certificate may expire soon (~{remaining} days remaining)"
                                ),
                            );
                        } else {
                            section.push(
                                Status::Ok,
                                format!("Certificate valid for ~{remaining} more days"),
                            );
                        }
                    }
                } else {
                    section.push(
                        Status::Info,
                        "Auto-generated certificates not yet created (generated on first gateway start)",
                    );
                }
            },
            Err(e) => {
                section.push(Status::Fail, format!("Cannot resolve cert directory: {e}"));
            },
        }
    }

    section
}

#[cfg(feature = "tls")]
fn check_file_readable(section: &mut Section, path: &str, label: &str) {
    let p = Path::new(path);
    if p.exists() {
        if std::fs::File::open(p).is_ok() {
            section.push(Status::Ok, format!("{label}: {path}"));
        } else {
            section.push(Status::Fail, format!("{label} not readable: {path}"));
        }
    } else {
        section.push(Status::Fail, format!("{label} not found: {path}"));
    }
}

/// Returns the age of a file in days (from mtime), or `None` on error.
#[cfg(feature = "tls")]
fn cert_age_days(path: &Path) -> Option<i64> {
    let meta = std::fs::metadata(path).ok()?;
    let modified = meta.modified().ok()?;
    let elapsed = std::time::SystemTime::now().duration_since(modified).ok()?;
    let secs_per_day = time::Duration::days(1).unsigned_abs().as_secs();
    Some((elapsed.as_secs() / secs_per_day) as i64)
}

// ── 7. MCP server health ───────────────────────────────────────────────────

fn check_mcp_servers(config: &MoltisConfig) -> Section {
    let mut section = Section::new("MCP Servers");

    if config.mcp.servers.is_empty() {
        section.push(Status::Info, "No MCP servers configured");
        return section;
    }

    for (name, entry) in &config.mcp.servers {
        if !entry.enabled {
            section.push(Status::Skip, format!("{name}: disabled"));
            continue;
        }

        // SSE/HTTP transports don't need a local command
        let transport = entry.transport.as_str();
        if transport == "sse" || transport == "http" {
            if let Some(ref url) = entry.url {
                section.push(Status::Ok, format!("{name}: {transport} transport ({url})"));
            } else {
                section.push(
                    Status::Fail,
                    format!("{name}: {transport} transport but no url configured"),
                );
            }
            continue;
        }

        // stdio transport — check command exists on PATH
        let cmd = &entry.command;
        if cmd.is_empty() {
            section.push(Status::Fail, format!("{name}: no command configured"));
            continue;
        }

        // If the command is an absolute path, check it directly
        let cmd_path = PathBuf::from(cmd);
        if cmd_path.is_absolute() {
            if cmd_path.exists() {
                section.push(Status::Ok, format!("{name}: command \"{cmd}\" found"));
            } else {
                section.push(Status::Fail, format!("{name}: command \"{cmd}\" not found"));
            }
        } else {
            match which::which(cmd) {
                Ok(_) => {
                    section.push(Status::Ok, format!("{name}: command \"{cmd}\" found"));
                },
                Err(_) => {
                    section.push(
                        Status::Fail,
                        format!("{name}: command \"{cmd}\" not found in PATH"),
                    );
                },
            }
        }
    }

    section
}

struct RemoteExecInventory {
    managed_key_count: i64,
    encrypted_key_count: i64,
    managed_target_count: i64,
    pinned_target_count: i64,
    default_target_label: Option<String>,
    default_target_auth_mode: Option<String>,
    default_target_is_pinned: bool,
}

async fn detect_ssh_version() -> Option<String> {
    let output = Command::new("ssh").arg("-V").output().await.ok()?;
    let text = if output.stdout.is_empty() {
        String::from_utf8_lossy(&output.stderr).trim().to_string()
    } else {
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    };
    (!text.is_empty()).then_some(text)
}

async fn read_remote_exec_inventory(data_dir: &Path) -> Result<Option<RemoteExecInventory>> {
    let db_path = data_dir.join("moltis.db");
    if !db_path.exists() {
        return Ok(None);
    }

    let db_url = format!("sqlite:{}?mode=ro", db_path.display());
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&db_url)
        .await?;

    let ssh_keys_exists = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(1) FROM sqlite_master WHERE type = 'table' AND name = 'ssh_keys'",
    )
    .fetch_one(&pool)
    .await?
        > 0;
    let ssh_targets_exists = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(1) FROM sqlite_master WHERE type = 'table' AND name = 'ssh_targets'",
    )
    .fetch_one(&pool)
    .await?
        > 0;

    if !ssh_keys_exists && !ssh_targets_exists {
        pool.close().await;
        return Ok(Some(RemoteExecInventory {
            managed_key_count: 0,
            encrypted_key_count: 0,
            managed_target_count: 0,
            pinned_target_count: 0,
            default_target_label: None,
            default_target_auth_mode: None,
            default_target_is_pinned: false,
        }));
    }

    let ssh_targets_has_known_host = if ssh_targets_exists {
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(1) FROM pragma_table_info('ssh_targets') WHERE name = 'known_host'",
        )
        .fetch_one(&pool)
        .await?
            > 0
    } else {
        false
    };

    let managed_key_count = if ssh_keys_exists {
        sqlx::query_scalar::<_, i64>("SELECT COUNT(1) FROM ssh_keys")
            .fetch_one(&pool)
            .await?
    } else {
        0
    };
    let encrypted_key_count = if ssh_keys_exists {
        sqlx::query_scalar::<_, i64>("SELECT COALESCE(SUM(encrypted), 0) FROM ssh_keys")
            .fetch_one(&pool)
            .await?
    } else {
        0
    };

    let (
        managed_target_count,
        pinned_target_count,
        default_target_label,
        default_target_auth_mode,
        default_target_is_pinned,
    ) = if ssh_targets_exists && ssh_targets_has_known_host {
        let row = sqlx::query_as::<_, (i64, i64, Option<String>, Option<String>, i64)>(
                "SELECT
                    (SELECT COUNT(1) FROM ssh_targets),
                    (SELECT COUNT(1) FROM ssh_targets WHERE known_host IS NOT NULL AND TRIM(known_host) <> ''),
                    (SELECT label FROM ssh_targets WHERE is_default = 1 ORDER BY updated_at DESC, id DESC LIMIT 1),
                    (SELECT auth_mode FROM ssh_targets WHERE is_default = 1 ORDER BY updated_at DESC, id DESC LIMIT 1),
                    COALESCE((SELECT CASE WHEN known_host IS NOT NULL AND TRIM(known_host) <> '' THEN 1 ELSE 0 END FROM ssh_targets WHERE is_default = 1 ORDER BY updated_at DESC, id DESC LIMIT 1), 0)",
            )
            .fetch_one(&pool)
            .await?;
        (row.0, row.1, row.2, row.3, row.4 != 0)
    } else if ssh_targets_exists {
        let row = sqlx::query_as::<_, (i64, Option<String>, Option<String>)>(
                "SELECT
                    (SELECT COUNT(1) FROM ssh_targets),
                    (SELECT label FROM ssh_targets WHERE is_default = 1 ORDER BY updated_at DESC, id DESC LIMIT 1),
                    (SELECT auth_mode FROM ssh_targets WHERE is_default = 1 ORDER BY updated_at DESC, id DESC LIMIT 1)",
            )
            .fetch_one(&pool)
            .await?;
        (row.0, 0, row.1, row.2, false)
    } else {
        (0, 0, None, None, false)
    };

    pool.close().await;

    Ok(Some(RemoteExecInventory {
        managed_key_count,
        encrypted_key_count,
        managed_target_count,
        pinned_target_count,
        default_target_label,
        default_target_auth_mode,
        default_target_is_pinned,
    }))
}

async fn check_remote_exec(config: &MoltisConfig, data_dir: &Path) -> Section {
    let mut section = Section::new("Remote Execution");
    let exec_host = config.tools.exec.host.trim();
    let ssh_binary_path = which::which("ssh").ok();
    let ssh_version = if ssh_binary_path.is_some() {
        detect_ssh_version().await
    } else {
        None
    };
    let configured_node = config
        .tools
        .exec
        .node
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let legacy_target = config
        .tools
        .exec
        .ssh_target
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());

    section.push(Status::Ok, match exec_host {
        "ssh" => "Backend mode: ssh",
        "node" => "Backend mode: node",
        _ => "Backend mode: local",
    });

    match ssh_binary_path {
        Some(path) => {
            if let Some(version) = ssh_version {
                section.push(
                    Status::Ok,
                    format!("SSH client found at {} ({version})", path.display()),
                );
            } else {
                section.push(
                    Status::Ok,
                    format!("SSH client found at {}", path.display()),
                );
            }
        },
        None => {
            let status = if exec_host == "ssh" {
                Status::Fail
            } else {
                Status::Warn
            };
            section.push(
                status,
                "SSH client not found in PATH, SSH targets will not work".to_string(),
            );
        },
    }

    let inventory = match read_remote_exec_inventory(data_dir).await {
        Ok(inventory) => inventory,
        Err(error) => {
            section.push(
                Status::Fail,
                format!("Failed to read managed SSH inventory from moltis.db: {error}"),
            );
            return section;
        },
    };

    if let Some(inventory) = inventory {
        section.push(
            Status::Info,
            format!(
                "Managed SSH inventory: {} key(s), {} target(s), {} pinned target(s), {} encrypted key(s)",
                inventory.managed_key_count,
                inventory.managed_target_count,
                inventory.pinned_target_count,
                inventory.encrypted_key_count
            ),
        );
        if let Some(default_label) = inventory.default_target_label.as_deref() {
            let auth_mode = inventory
                .default_target_auth_mode
                .as_deref()
                .unwrap_or("unknown");
            section.push(
                Status::Info,
                format!(
                    "Default managed target: {default_label} ({auth_mode}, {})",
                    if inventory.default_target_is_pinned {
                        "host pinned"
                    } else {
                        "inherits known_hosts policy"
                    }
                ),
            );
        }

        if exec_host == "ssh" && legacy_target.is_none() && inventory.default_target_label.is_none()
        {
            section.push(
                Status::Fail,
                "SSH backend is active, but there is no default managed target and no legacy ssh_target configured".to_string(),
            );
        } else if exec_host == "ssh"
            && inventory.default_target_label.is_some()
            && !inventory.default_target_is_pinned
        {
            section.push(
                Status::Warn,
                "Active managed SSH route is not host-pinned, paste a known_hosts line in Settings → SSH".to_string(),
            );
        }
    } else {
        section.push(
            Status::Skip,
            "moltis.db not found, managed SSH inventory unavailable",
        );
    }

    if let Some(target) = legacy_target {
        let status = if exec_host == "ssh" {
            Status::Warn
        } else {
            Status::Info
        };
        section.push(
            status,
            format!(
                "Legacy ssh_target is configured as '{target}', move it into Settings → SSH if you want named targets, host pinning, and managed keys"
            ),
        );
    }

    match exec_host {
        "node" => {
            if let Some(node) = configured_node {
                section.push(
                    Status::Info,
                    format!("Default paired-node preference: {node}"),
                );
            } else {
                section.push(
                    Status::Warn,
                    "Node backend is active, but tools.exec.node is not set. Session picks or runtime routing will decide.".to_string(),
                );
            }
            section.push(
                Status::Info,
                "Live paired-node presence and active-route tests are available from the Nodes page when the gateway is running".to_string(),
            );
        },
        "ssh" => {
            if configured_node.is_some() {
                section.push(
                    Status::Info,
                    "tools.exec.node is set but ignored while the SSH backend is active"
                        .to_string(),
                );
            }
        },
        _ => {
            if legacy_target.is_some() || configured_node.is_some() {
                section.push(
                    Status::Info,
                    "Remote targets are configured, but local execution remains the default until you switch tools.exec.host or pick a route in chat".to_string(),
                );
            }
        },
    }

    section
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
#[path = "doctor_commands_tests.rs"]
mod tests;
