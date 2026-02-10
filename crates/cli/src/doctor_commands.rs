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

    // Semantic warnings (security, etc.)
    for d in &result.diagnostics {
        if d.category == "security" || d.category == "unknown-provider" {
            let status = match d.severity {
                Severity::Error => Status::Fail,
                Severity::Warning => Status::Warn,
                Severity::Info => Status::Info,
            };
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

        let has_env_key = env_info.is_some_and(|(_, env, _)| std::env::var(env).is_ok());
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
        match moltis_gateway::tls::cert_dir() {
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

// ── Tests ───────────────────────────────────────────────────────────────────

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use {super::*, moltis_config::MoltisConfig};

    #[test]
    fn status_labels() {
        assert_eq!(Status::Ok.label(), "ok");
        assert_eq!(Status::Warn.label(), "warn");
        assert_eq!(Status::Fail.label(), "fail");
        assert_eq!(Status::Skip.label(), "skip");
        assert_eq!(Status::Info.label(), "info");
    }

    #[test]
    fn section_push_counts() {
        let mut section = Section::new("test");
        section.push(Status::Ok, "good");
        section.push(Status::Warn, "attention");
        section.push(Status::Fail, "bad");
        assert_eq!(section.items.len(), 3);
        assert_eq!(section.items[0].status, Status::Ok);
        assert_eq!(section.items[1].status, Status::Warn);
        assert_eq!(section.items[2].status, Status::Fail);
    }

    #[test]
    fn print_report_counts_errors_and_warnings() {
        let mut section = Section::new("test");
        section.push(Status::Ok, "fine");
        section.push(Status::Warn, "caution");
        section.push(Status::Warn, "caution2");
        section.push(Status::Fail, "broken");
        section.push(Status::Info, "note");

        let (errors, warnings) = print_report(&[section]);
        assert_eq!(errors, 1);
        assert_eq!(warnings, 2);
    }

    #[test]
    fn check_providers_empty_config() {
        let config = MoltisConfig::default();
        let section = check_providers(&config);
        assert_eq!(section.items.len(), 1);
        assert_eq!(section.items[0].status, Status::Info);
        assert!(section.items[0].message.contains("No providers configured"));
    }

    #[test]
    fn check_providers_with_config_key() {
        let mut config = MoltisConfig::default();
        let entry = moltis_config::schema::ProviderEntry {
            api_key: Some(secrecy::Secret::new("sk-test-fake".to_string())),
            ..Default::default()
        };
        config
            .providers
            .providers
            .insert("anthropic".to_string(), entry);

        let section = check_providers(&config);
        let anthropic_item = section
            .items
            .iter()
            .find(|i| i.message.contains("anthropic"));
        assert!(anthropic_item.is_some());
        assert_eq!(anthropic_item.unwrap().status, Status::Ok);
    }

    #[test]
    fn check_providers_missing_key_warns() {
        let mut config = MoltisConfig::default();
        // Use a provider unlikely to have its env var set in CI
        config.providers.providers.insert(
            "minimax".to_string(),
            moltis_config::schema::ProviderEntry::default(),
        );

        // Only assert warning if the env var is genuinely absent
        if std::env::var("MINIMAX_API_KEY").is_err() {
            let section = check_providers(&config);
            let item = section.items.iter().find(|i| i.message.contains("minimax"));
            assert!(item.is_some());
            assert_eq!(item.unwrap().status, Status::Warn);
        }
    }

    #[test]
    fn check_providers_ollama_optional() {
        let mut config = MoltisConfig::default();
        config.providers.providers.insert(
            "ollama".to_string(),
            moltis_config::schema::ProviderEntry::default(),
        );

        // Ollama key is optional — if the env var happens to be set,
        // it will report Ok; if not, it should be Info (not Warn).
        let section = check_providers(&config);
        let ollama_item = section.items.iter().find(|i| i.message.contains("ollama"));
        assert!(ollama_item.is_some());
        let status = ollama_item.unwrap().status;
        assert!(
            status == Status::Info || status == Status::Ok,
            "ollama should be Info or Ok, got {status:?}",
        );
    }

    #[test]
    fn check_providers_disabled_skipped() {
        let mut config = MoltisConfig::default();
        let entry = moltis_config::schema::ProviderEntry {
            enabled: false,
            ..Default::default()
        };
        config
            .providers
            .providers
            .insert("openai".to_string(), entry);

        let section = check_providers(&config);
        let openai_item = section.items.iter().find(|i| i.message.contains("openai"));
        assert!(openai_item.is_some());
        assert_eq!(openai_item.unwrap().status, Status::Skip);
    }

    #[test]
    fn check_providers_oauth_skipped() {
        let mut config = MoltisConfig::default();
        config.providers.providers.insert(
            "github-copilot".to_string(),
            moltis_config::schema::ProviderEntry::default(),
        );

        let section = check_providers(&config);
        let gh_item = section
            .items
            .iter()
            .find(|i| i.message.contains("github-copilot"));
        assert!(gh_item.is_some());
        assert_eq!(gh_item.unwrap().status, Status::Skip);
    }

    #[test]
    fn check_mcp_servers_empty() {
        let config = MoltisConfig::default();
        let section = check_mcp_servers(&config);
        assert_eq!(section.items.len(), 1);
        assert_eq!(section.items[0].status, Status::Info);
    }

    #[test]
    fn check_mcp_servers_disabled_skipped() {
        let mut config = MoltisConfig::default();
        let entry = moltis_config::schema::McpServerEntry {
            command: "node".to_string(),
            args: vec![],
            env: Default::default(),
            enabled: false,
            transport: String::new(),
            url: None,
        };
        config.mcp.servers.insert("test".to_string(), entry);

        let section = check_mcp_servers(&config);
        let test_item = section.items.iter().find(|i| i.message.contains("test"));
        assert!(test_item.is_some());
        assert_eq!(test_item.unwrap().status, Status::Skip);
    }

    #[test]
    fn check_mcp_servers_missing_command_fails() {
        let mut config = MoltisConfig::default();
        let entry = moltis_config::schema::McpServerEntry {
            command: String::new(),
            args: vec![],
            env: Default::default(),
            enabled: true,
            transport: String::new(),
            url: None,
        };
        config.mcp.servers.insert("broken".to_string(), entry);

        let section = check_mcp_servers(&config);
        let broken_item = section.items.iter().find(|i| i.message.contains("broken"));
        assert!(broken_item.is_some());
        assert_eq!(broken_item.unwrap().status, Status::Fail);
    }

    #[test]
    fn check_mcp_servers_sse_with_url_ok() {
        let mut config = MoltisConfig::default();
        let entry = moltis_config::schema::McpServerEntry {
            command: String::new(),
            args: vec![],
            env: Default::default(),
            enabled: true,
            transport: "sse".to_string(),
            url: Some("http://localhost:3000/sse".to_string()),
        };
        config.mcp.servers.insert("remote".to_string(), entry);

        let section = check_mcp_servers(&config);
        let remote_item = section.items.iter().find(|i| i.message.contains("remote"));
        assert!(remote_item.is_some());
        assert_eq!(remote_item.unwrap().status, Status::Ok);
    }

    #[test]
    fn check_mcp_servers_sse_without_url_fails() {
        let mut config = MoltisConfig::default();
        let entry = moltis_config::schema::McpServerEntry {
            command: String::new(),
            args: vec![],
            env: Default::default(),
            enabled: true,
            transport: "sse".to_string(),
            url: None,
        };
        config.mcp.servers.insert("broken-sse".to_string(), entry);

        let section = check_mcp_servers(&config);
        let item = section
            .items
            .iter()
            .find(|i| i.message.contains("broken-sse"));
        assert!(item.is_some());
        assert_eq!(item.unwrap().status, Status::Fail);
    }

    #[test]
    fn check_mcp_servers_nonexistent_command_fails() {
        let mut config = MoltisConfig::default();
        let entry = moltis_config::schema::McpServerEntry {
            command: "definitely-not-a-real-command-xyz123".to_string(),
            args: vec![],
            env: Default::default(),
            enabled: true,
            transport: String::new(),
            url: None,
        };
        config.mcp.servers.insert("bad".to_string(), entry);

        let section = check_mcp_servers(&config);
        let item = section.items.iter().find(|i| i.message.contains("bad"));
        assert!(item.is_some());
        assert_eq!(item.unwrap().status, Status::Fail);
    }

    #[test]
    fn check_directories_with_temp_dirs() {
        let temp = tempfile::TempDir::new().unwrap();
        let config_dir = temp.path().join("config");
        let data_dir = temp.path().join("data");
        std::fs::create_dir_all(&config_dir).unwrap();
        std::fs::create_dir_all(&data_dir).unwrap();

        let section = check_directories(Some(&config_dir), &data_dir);

        let ok_count = section
            .items
            .iter()
            .filter(|i| i.status == Status::Ok)
            .count();
        // Config dir + data dir should be ok at minimum
        assert!(
            ok_count >= 2,
            "expected at least 2 OK items, got {ok_count}"
        );
    }

    #[test]
    fn check_directories_missing_config_dir() {
        let temp = tempfile::TempDir::new().unwrap();
        let missing = temp.path().join("nonexistent");
        let data_dir = temp.path().join("data");
        std::fs::create_dir_all(&data_dir).unwrap();

        let section = check_directories(Some(&missing), &data_dir);

        let fail_item = section
            .items
            .iter()
            .find(|i| i.status == Status::Fail && i.message.contains("Config directory missing"));
        assert!(fail_item.is_some());
    }

    #[tokio::test]
    async fn check_database_missing_file() {
        let temp = tempfile::TempDir::new().unwrap();
        let section = check_database(temp.path()).await;
        assert_eq!(section.items.len(), 1);
        assert_eq!(section.items[0].status, Status::Skip);
    }

    #[tokio::test]
    async fn check_database_valid_db() {
        let temp = tempfile::TempDir::new().unwrap();
        let db_path = temp.path().join("moltis.db");

        // Create a minimal SQLite database
        let db_url = format!("sqlite:{}?mode=rwc", db_path.display());
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect(&db_url)
            .await
            .unwrap();
        pool.close().await;

        let section = check_database(temp.path()).await;
        let ok_item = section.items.iter().find(|i| i.status == Status::Ok);
        assert!(
            ok_item.is_some(),
            "expected OK for valid db, got: {:?}",
            section
                .items
                .iter()
                .map(|i| (&i.status, &i.message))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn check_security_no_api_keys_in_config() {
        let config = MoltisConfig::default();
        let temp = tempfile::TempDir::new().unwrap();
        let section = check_security(&config, Some(temp.path()), temp.path());

        let ok_item = section
            .items
            .iter()
            .find(|i| i.message.contains("No API keys in config file"));
        assert!(ok_item.is_some());
        assert_eq!(ok_item.unwrap().status, Status::Ok);
    }

    #[test]
    fn check_security_api_keys_in_config_warns() {
        let mut config = MoltisConfig::default();
        let entry = moltis_config::schema::ProviderEntry {
            api_key: Some(secrecy::Secret::new("sk-test".to_string())),
            ..Default::default()
        };
        config
            .providers
            .providers
            .insert("anthropic".to_string(), entry);

        let temp = tempfile::TempDir::new().unwrap();
        let section = check_security(&config, Some(temp.path()), temp.path());

        let warn_item = section
            .items
            .iter()
            .find(|i| i.message.contains("API keys found in config"));
        assert!(warn_item.is_some());
        assert_eq!(warn_item.unwrap().status, Status::Warn);
    }
}
