//! Trait interfaces for domain services the gateway delegates to.
//! Each trait has a `Noop` implementation that returns empty/default responses,
//! allowing the gateway to run standalone before domain crates are wired in.
//!
//! Pure trait definitions and simple noop implementations live in `moltis-service-traits`.
//! This module re-exports everything from that crate and adds gateway-specific implementations.

mod skills;

// Re-export all trait definitions and simple noops from service-traits.
pub use {moltis_service_traits::*, skills::NoopSkillsService};

use {
    async_trait::async_trait,
    serde_json::Value,
    std::{path::Path, sync::Arc},
};

mod browser;
mod gateway;

pub use {browser::RealBrowserService, gateway::GatewayServices};

fn security_audit(event: &str, details: Value) {
    let dir = moltis_config::data_dir().join("logs");
    let path = dir.join("security-audit.jsonl");
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let line = serde_json::json!({
        "ts": now_ms,
        "event": event,
        "details": details,
    })
    .to_string();

    let _ = (|| -> std::io::Result<()> {
        std::fs::create_dir_all(&dir)?;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        use std::io::Write as _;
        writeln!(file, "{line}")?;
        Ok(())
    })();
}

async fn command_available(command: &str) -> bool {
    tokio::process::Command::new(command)
        .arg("--version")
        .output()
        .await
        .map(|o| o.status.success())
        .unwrap_or(false)
}

async fn run_mcp_scan(installed_dir: &Path) -> anyhow::Result<Value> {
    let mut cmd = if command_available("uvx").await {
        let mut c = tokio::process::Command::new("uvx");
        c.arg("mcp-scan@latest");
        c
    } else {
        tokio::process::Command::new("mcp-scan")
    };

    cmd.arg("--skills")
        .arg(installed_dir)
        .arg("--json")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    let output = tokio::time::timeout(std::time::Duration::from_secs(300), cmd.output())
        .await
        .map_err(|_| anyhow::anyhow!("mcp-scan timed out after 5 minutes"))??;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        anyhow::bail!(if stderr.is_empty() {
            "mcp-scan failed".to_string()
        } else {
            format!("mcp-scan failed: {stderr}")
        });
    }

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let parsed: Value = serde_json::from_str(&stdout)
        .map_err(|e| anyhow::anyhow!("invalid mcp-scan JSON output: {e}"))?;
    Ok(parsed)
}

/// Returns `true` for discovered skill names that are protected and cannot be
/// deleted from the UI (e.g. built-in template/tmux skills).
pub fn is_protected_discovered_skill(name: &str) -> bool {
    matches!(name, "template-skill" | "template" | "tmux")
}

fn commit_url_for_source(source: &str, sha: &str) -> Option<String> {
    if sha.trim().is_empty() {
        return None;
    }
    if source.starts_with("https://") || source.starts_with("http://") {
        return Some(format!("{}/commit/{}", source.trim_end_matches('/'), sha));
    }
    if source.contains('/') {
        return Some(format!("https://github.com/{}/commit/{}", source, sha));
    }
    None
}

fn license_url_for_source(source: &str, license: Option<&str>) -> Option<String> {
    let text = license?.to_ascii_lowercase();
    let file = if text.contains("license.txt") {
        "LICENSE.txt"
    } else if text.contains("license.md") {
        "LICENSE.md"
    } else if text.contains("license") {
        "LICENSE"
    } else {
        return None;
    };

    if source.starts_with("https://") || source.starts_with("http://") {
        Some(format!(
            "{}/blob/main/{}",
            source.trim_end_matches('/'),
            file
        ))
    } else if source.contains('/') {
        Some(format!("https://github.com/{}/blob/main/{}", source, file))
    } else {
        None
    }
}

fn local_repo_head_timestamp_ms(repo_dir: &Path) -> Option<u64> {
    let repo = gix::open(repo_dir).ok()?;
    let obj = repo.rev_parse_single("HEAD").ok()?;
    let commit = repo.find_commit(obj.detach()).ok()?;
    let secs = commit.time().ok()?.seconds;
    Some((secs as i128).max(0) as u64 * 1000)
}

fn commit_age_days(commit_ts_ms: Option<u64>) -> Option<u64> {
    let ts = commit_ts_ms?;
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_millis() as u64;
    Some(now_ms.saturating_sub(ts) / 86_400_000)
}

fn risky_install_pattern(command: &str) -> Option<&'static str> {
    let c = command.to_ascii_lowercase();
    if (c.contains("curl") || c.contains("wget")) && (c.contains("| sh") || c.contains("|bash")) {
        return Some("piped shell execution");
    }

    let patterns = [
        ("base64", "obfuscated payload decoding"),
        ("xattr -d com.apple.quarantine", "quarantine bypass"),
        ("bash -c", "inline shell execution"),
        ("sh -c", "inline shell execution"),
        ("python -c", "inline code execution"),
        ("node -e", "inline code execution"),
    ];
    patterns
        .into_iter()
        .find_map(|(needle, reason)| c.contains(needle).then_some(reason))
}

/// Convert markdown to sanitized HTML using pulldown-cmark.
pub(crate) fn markdown_to_html(md: &str) -> String {
    use pulldown_cmark::{Options, Parser, html};
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_TASKLISTS);
    let parser = Parser::new_ext(md, opts);
    let mut html_output = String::new();
    html::push_html(&mut html_output, parser);
    html_output
}
