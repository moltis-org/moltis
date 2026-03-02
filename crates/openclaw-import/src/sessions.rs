//! Import session JSONL files from OpenClaw to Moltis format.
//!
//! OpenClaw sessions live at either:
//! - `~/.openclaw/agents/<id>/sessions/<key>.jsonl` (legacy layout), or
//! - `~/.openclaw/agents/<id>/agent/sessions/<key>.jsonl` (newer layout).
//!
//! Moltis sessions live at `<data_dir>/sessions/<safe-key>.jsonl` with metadata
//! in `metadata.json`.

use std::{
    collections::HashMap,
    io::{BufRead, BufReader, Write},
    path::Path,
};

use {
    serde::{Deserialize, Serialize},
    tracing::{debug, warn},
};

use crate::{
    detect::{OpenClawDetection, resolve_agent_sessions_dir},
    identity::normalize_display_name,
    report::{CategoryReport, ImportCategory, ImportStatus},
    types::{
        OpenClawContent, OpenClawRole, OpenClawSessionIndexEntry, OpenClawSessionOrigin,
        OpenClawSessionRecord, OpenClawTimestamp,
    },
};

/// Minimal session metadata for the Moltis `metadata.json` index.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportedSessionEntry {
    pub id: String,
    pub key: String,
    pub label: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview: Option<String>,
    pub created_at: u64,
    pub updated_at: u64,
    pub message_count: u32,
    #[serde(default)]
    pub last_seen_message_count: u32,
    #[serde(default)]
    pub source_line_count: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(default)]
    pub archived: bool,
    #[serde(default)]
    pub version: u64,
}

/// A converted Moltis message (matches `PersistedMessage` serde format).
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "role", rename_all = "lowercase")]
enum MoltisMessage {
    System {
        content: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        created_at: Option<u64>,
    },
    User {
        content: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        created_at: Option<u64>,
    },
    Assistant {
        content: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        created_at: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        model: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        provider: Option<String>,
    },
    Tool {
        tool_call_id: String,
        content: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        created_at: Option<u64>,
    },
}

/// Import sessions from all agents in an OpenClaw installation.
///
/// In addition to converting JSONL files, this also generates markdown
/// transcripts in `memory_sessions_dir` (typically `<data>/memory/sessions/`)
/// so that imported conversations are searchable by the Moltis memory system.
///
/// `agent_id_mapping` maps OpenClaw agent IDs to Moltis agent IDs. If empty,
/// agent IDs are used as-is (backward compatible for single-agent installs).
pub fn import_sessions(
    detection: &OpenClawDetection,
    dest_sessions_dir: &Path,
    memory_sessions_dir: &Path,
    agent_id_mapping: &HashMap<String, String>,
) -> CategoryReport {
    if detection.agent_ids.is_empty() {
        return CategoryReport::skipped(ImportCategory::Sessions);
    }

    let mut imported = 0;
    let mut updated = 0;
    let mut skipped = 0;
    let mut errors = Vec::new();
    let warnings = Vec::new();
    let mut entries = Vec::new();

    if let Err(e) = std::fs::create_dir_all(dest_sessions_dir) {
        return CategoryReport::failed(
            ImportCategory::Sessions,
            format!("failed to create destination directory: {e}"),
        );
    }

    // Load existing metadata once to detect incremental changes
    let metadata_path = dest_sessions_dir.join("metadata.json");
    let existing_metadata = load_session_metadata(&metadata_path);

    for openclaw_agent_id in &detection.agent_ids {
        let moltis_agent_id = agent_id_mapping
            .get(openclaw_agent_id.as_str())
            .cloned()
            .unwrap_or_else(|| openclaw_agent_id.clone());

        let agent_dir = detection.home_dir.join("agents").join(openclaw_agent_id);
        let Some(sessions_dir) = resolve_agent_sessions_dir(&agent_dir) else {
            debug!(agent = %openclaw_agent_id, "no sessions directory found, skipping");
            continue;
        };

        let Ok(dir_entries) = std::fs::read_dir(&sessions_dir) else {
            debug!(agent = %openclaw_agent_id, "failed to read sessions directory, skipping");
            continue;
        };

        let session_label_map = load_session_label_map(&sessions_dir);

        for entry in dir_entries.flatten() {
            let path = entry.path();
            if !path.is_file() || path.extension().is_some_and(|e| e != "jsonl") {
                continue;
            }

            let stem = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown");
            let dest_key = format!("oc:{moltis_agent_id}:{stem}");
            let dest_file =
                dest_sessions_dir.join(format!("{}.jsonl", sanitize_session_key(&dest_key)));

            let source_lines = count_lines(&path);

            // Check if we already have this session and whether it has grown
            let existing_entry = existing_metadata.get(&dest_key);
            let is_update = if let Some(prev) = existing_entry {
                if prev.source_line_count > 0 && source_lines <= prev.source_line_count {
                    debug!(key = %dest_key, "session unchanged, skipping");
                    skipped += 1;
                    continue;
                }
                true
            } else {
                false
            };

            match convert_session(&path, &dest_file) {
                Ok(stats) => {
                    let label = session_label_map
                        .get(stem)
                        .map(|(logical_key, origin)| {
                            build_session_label(logical_key, origin.as_ref())
                        })
                        .unwrap_or_else(|| {
                            let prefix_len = 8.min(stem.len());
                            format!("OpenClaw: {}", &stem[..prefix_len])
                        });

                    if is_update {
                        debug!(key = %dest_key, messages = stats.message_count, "updated session (incremental)");
                    } else {
                        debug!(key = %dest_key, messages = stats.message_count, "imported session");
                    }

                    if !stats.transcript.is_empty()
                        && let Err(e) = write_transcript(
                            memory_sessions_dir,
                            &dest_key,
                            &label,
                            stats.last_model.as_deref(),
                            &stats,
                        )
                    {
                        warn!(key = %dest_key, error = %e, "failed to write session transcript");
                    }

                    let (id, created_at, version) = if let Some(prev) = existing_entry {
                        (
                            prev.id.clone(),
                            prev.created_at,
                            prev.version.saturating_add(1),
                        )
                    } else {
                        (uuid_v4(), stats.first_timestamp.unwrap_or_else(now_ms), 0)
                    };

                    // Set agent_id for non-default agents
                    let agent_id = if moltis_agent_id == "main" {
                        None
                    } else {
                        Some(moltis_agent_id.clone())
                    };

                    entries.push(ImportedSessionEntry {
                        id,
                        key: dest_key,
                        label: Some(label),
                        model: stats.last_model,
                        preview: stats.preview,
                        created_at,
                        updated_at: stats.last_timestamp.unwrap_or_else(now_ms),
                        message_count: stats.message_count,
                        last_seen_message_count: stats.message_count,
                        source_line_count: source_lines,
                        agent_id,
                        archived: false,
                        version,
                    });
                    if is_update {
                        updated += 1;
                    } else {
                        imported += 1;
                    }
                },
                Err(e) => {
                    warn!(source = %path.display(), error = %e, "failed to convert session");
                    errors.push(format!("failed to convert {}: {e}", path.display()));
                },
            }
        }
    }

    // Write/merge session metadata
    if !entries.is_empty()
        && let Err(e) = merge_session_metadata(&metadata_path, &entries)
    {
        errors.push(format!("failed to update metadata.json: {e}"));
    }

    let total_changed = imported + updated;
    let status = if !errors.is_empty() && total_changed > 0 {
        ImportStatus::Partial
    } else if !errors.is_empty() {
        ImportStatus::Failed
    } else if total_changed == 0 {
        ImportStatus::Skipped
    } else {
        ImportStatus::Success
    };

    CategoryReport {
        category: ImportCategory::Sessions,
        status,
        items_imported: imported,
        items_updated: updated,
        items_skipped: skipped,
        warnings,
        errors,
    }
}

/// Load the `sessions.json` index and build a reverse map from session-ID
/// (UUID filename stem) to the logical key and origin metadata.
fn load_session_label_map(
    sessions_dir: &Path,
) -> HashMap<String, (String, Option<OpenClawSessionOrigin>)> {
    let index_path = sessions_dir.join("sessions.json");
    let Ok(content) = std::fs::read_to_string(&index_path) else {
        return HashMap::new();
    };
    let Ok(entries): Result<HashMap<String, OpenClawSessionIndexEntry>, _> =
        serde_json::from_str(&content)
    else {
        return HashMap::new();
    };

    let mut map = HashMap::new();
    for (logical_key, entry) in entries {
        if let Some(ref sid) = entry.session_id {
            map.insert(sid.clone(), (logical_key, entry.origin));
        }
    }
    map
}

/// Build a human-readable session label from the logical key and origin.
///
/// Key patterns:
/// - `agent:*:main`             → "Main"
/// - `agent:*:telegram:dm:*`    → "Telegram: {cleaned label}"
/// - `agent:*:signal:dm:*`      → "Signal: {cleaned label}"
/// - `agent:*:cron:*`           → "Cron"
/// - Other with origin label    → "OpenClaw: {cleaned label}"
/// - Fallback (no index entry)  → "OpenClaw: {uuid prefix}"
fn build_session_label(logical_key: &str, origin: Option<&OpenClawSessionOrigin>) -> String {
    let parts: Vec<&str> = logical_key.split(':').collect();

    // agent:<name>:main → "Main"
    if parts.len() == 3 && parts[0] == "agent" && parts[2] == "main" {
        return "Main".to_string();
    }

    // agent:<name>:telegram:dm:<id> → "Telegram: {label}"
    if parts.len() >= 4 && parts[0] == "agent" && parts[2] == "telegram" {
        if let Some(cleaned) = origin
            .and_then(|o| o.label.as_deref())
            .and_then(normalize_display_name)
        {
            return format!("Telegram: {cleaned}");
        }
        return "Telegram".to_string();
    }

    // agent:<name>:signal:dm:<id> → "Signal: {label}"
    if parts.len() >= 4 && parts[0] == "agent" && parts[2] == "signal" {
        if let Some(cleaned) = origin
            .and_then(|o| o.label.as_deref())
            .and_then(normalize_display_name)
        {
            return format!("Signal: {cleaned}");
        }
        return "Signal".to_string();
    }

    // agent:<name>:cron:* → "Cron"
    if parts.len() >= 3 && parts[0] == "agent" && parts[2] == "cron" {
        return "Cron".to_string();
    }

    // Fallback: use cleaned origin label if available
    if let Some(cleaned) = origin
        .and_then(|o| o.label.as_deref())
        .and_then(normalize_display_name)
    {
        return format!("OpenClaw: {cleaned}");
    }

    format!("OpenClaw: {logical_key}")
}

struct ConvertStats {
    message_count: u32,
    first_timestamp: Option<u64>,
    last_timestamp: Option<u64>,
    last_model: Option<String>,
    preview: Option<String>,
    /// Collected transcript entries for markdown export.
    transcript: Vec<TranscriptEntry>,
}

/// A single entry in a session transcript (for markdown export).
struct TranscriptEntry {
    role: &'static str,
    content: String,
}

fn convert_session(src: &Path, dest: &Path) -> crate::error::Result<ConvertStats> {
    let file = std::fs::File::open(src)?;
    let reader = BufReader::new(file);

    let mut dest_file = std::fs::File::create(dest)?;
    let mut stats = ConvertStats {
        message_count: 0,
        first_timestamp: None,
        last_timestamp: None,
        last_model: None,
        preview: None,
        transcript: Vec::new(),
    };

    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }

        let record: OpenClawSessionRecord = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(_) => continue, // Skip malformed lines
        };

        match record {
            OpenClawSessionRecord::Message { timestamp, message } => {
                let message_timestamp = message
                    .timestamp
                    .as_ref()
                    .and_then(OpenClawTimestamp::to_millis)
                    .or_else(|| timestamp.as_ref().and_then(OpenClawTimestamp::to_millis));
                if let Some(msg) = convert_message(&message, message_timestamp, &mut stats) {
                    let json = serde_json::to_string(&msg)?;
                    writeln!(dest_file, "{json}")?;
                    stats.message_count += 1;
                }
            },
            OpenClawSessionRecord::Custom { custom_type, data } => {
                // Extract model name from model-snapshot records
                if custom_type.as_deref() == Some("model-snapshot")
                    && let Some(model) = data
                        .as_ref()
                        .and_then(|d| d.get("model"))
                        .and_then(|m| m.as_str())
                {
                    stats.last_model = Some(model.to_string());
                }
            },
            OpenClawSessionRecord::SessionMeta { .. } => {
                // Session metadata is used for detection/scan only
            },
        }
    }

    stats.preview = build_preview(&stats.transcript);

    Ok(stats)
}

fn convert_message(
    msg: &crate::types::OpenClawMessage,
    timestamp_ms: Option<u64>,
    stats: &mut ConvertStats,
) -> Option<MoltisMessage> {
    let content = msg.content.as_ref().map(OpenClawContent::as_text)?;
    if content.is_empty() {
        return None;
    }

    let created_at = timestamp_ms.unwrap_or_else(now_ms);
    if stats.first_timestamp.is_none() {
        stats.first_timestamp = Some(created_at);
    }
    stats.last_timestamp = Some(created_at);

    // Collect user/assistant messages for the markdown transcript
    let role_label = match msg.role {
        OpenClawRole::User => Some("User"),
        OpenClawRole::Assistant => Some("Assistant"),
        _ => None,
    };
    if let Some(label) = role_label {
        stats.transcript.push(TranscriptEntry {
            role: label,
            content: content.clone(),
        });
    }

    match msg.role {
        OpenClawRole::System => Some(MoltisMessage::System {
            content,
            created_at: Some(created_at),
        }),
        OpenClawRole::User => Some(MoltisMessage::User {
            content,
            created_at: Some(created_at),
        }),
        OpenClawRole::Assistant => Some(MoltisMessage::Assistant {
            content,
            created_at: Some(created_at),
            model: None,
            provider: None,
        }),
        OpenClawRole::Tool | OpenClawRole::ToolResult => {
            let tool_call_id = msg.tool_use_id.clone().unwrap_or_default();
            Some(MoltisMessage::Tool {
                tool_call_id,
                content,
                created_at: Some(created_at),
            })
        },
    }
}

/// Count lines in a file without parsing content.
fn count_lines(path: &Path) -> u32 {
    let Ok(file) = std::fs::File::open(path) else {
        return 0;
    };
    BufReader::new(file).lines().count() as u32
}

fn sanitize_session_key(key: &str) -> String {
    key.replace(':', "_")
}

/// Load existing session metadata from disk.
fn load_session_metadata(path: &Path) -> HashMap<String, ImportedSessionEntry> {
    if !path.is_file() {
        return HashMap::new();
    }
    let Ok(content) = std::fs::read_to_string(path) else {
        return HashMap::new();
    };
    serde_json::from_str(&content).unwrap_or_default()
}

fn merge_session_metadata(
    path: &Path,
    new_entries: &[ImportedSessionEntry],
) -> crate::error::Result<()> {
    let mut existing = load_session_metadata(path);

    for entry in new_entries {
        existing.insert(entry.key.clone(), entry.clone());
    }

    let json = serde_json::to_string_pretty(&existing)?;
    std::fs::write(path, json)?;
    Ok(())
}

fn build_preview(transcript: &[TranscriptEntry]) -> Option<String> {
    const TARGET_CHARS: usize = 140;
    const MAX_CHARS: usize = 200;

    let mut combined = String::new();
    for entry in transcript {
        let normalized = normalize_whitespace(&entry.content);
        if normalized.is_empty() {
            continue;
        }
        if !combined.is_empty() {
            combined.push(' ');
        }
        combined.push_str(&normalized);
        if combined.chars().count() >= TARGET_CHARS {
            break;
        }
    }

    if combined.is_empty() {
        return None;
    }

    Some(truncate_preview(&combined, MAX_CHARS))
}

fn normalize_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn truncate_preview(text: &str, max_chars: usize) -> String {
    let len = text.chars().count();
    if len <= max_chars {
        return text.to_string();
    }

    let cutoff = max_chars.saturating_sub(3);
    let mut out = String::new();
    for ch in text.chars().take(cutoff) {
        out.push(ch);
    }
    out.push_str("...");
    out
}

/// Write a markdown transcript of a session for memory search indexing.
///
/// The file is placed in `memory/sessions/` and includes all user/assistant
/// messages so they become searchable by the Moltis memory system.
fn write_transcript(
    dir: &Path,
    dest_key: &str,
    label: &str,
    model: Option<&str>,
    stats: &ConvertStats,
) -> crate::error::Result<()> {
    std::fs::create_dir_all(dir)?;

    // Use hyphens instead of colons for filesystem safety
    let safe_name = dest_key.replace(':', "-");
    let path = dir.join(format!("{safe_name}.md"));

    let mut content = format!("# Session: {label}\n\n");
    content.push_str("*Imported from OpenClaw*");
    if let Some(m) = model {
        content.push_str(&format!(" | Model: {m}"));
    }
    content.push_str(&format!(" | Messages: {}\n\n---\n\n", stats.message_count));

    for entry in &stats.transcript {
        content.push_str(&format!("**{}:** {}\n\n", entry.role, entry.content));
    }

    std::fs::write(path, content)?;
    Ok(())
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn uuid_v4() -> String {
    // Simple UUID v4 without pulling in the uuid crate.
    // Format: 8-4-4-4-12 hex characters.
    use std::time::{SystemTime, UNIX_EPOCH};
    let seed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!(
        "{:08x}-{:04x}-4{:03x}-{:04x}-{:012x}",
        (seed >> 96) as u32,
        (seed >> 80) as u16,
        (seed >> 64) as u16 & 0x0FFF,
        ((seed >> 48) as u16 & 0x3FFF) | 0x8000,
        seed as u64 & 0xFFFF_FFFF_FFFF,
    )
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn make_detection(home: &Path) -> OpenClawDetection {
        OpenClawDetection {
            home_dir: home.to_path_buf(),
            has_config: false,
            has_credentials: false,
            has_mcp_servers: false,
            workspace_dir: home.join("workspace"),
            has_memory: false,
            has_skills: false,
            agent_ids: vec!["main".to_string()],
            session_count: 1,
            unsupported_channels: Vec::new(),
            has_workspace_files: false,
            workspace_files_found: Vec::new(),
        }
    }

    fn default_mapping() -> HashMap<String, String> {
        let mut m = HashMap::new();
        m.insert("main".to_string(), "main".to_string());
        m
    }

    fn setup_session(home: &Path, agent: &str, key: &str, lines: &[&str]) {
        let dir = home
            .join("agents")
            .join(agent)
            .join("agent")
            .join("sessions");
        std::fs::create_dir_all(&dir).unwrap();
        let content = lines.join("\n");
        std::fs::write(dir.join(format!("{key}.jsonl")), content).unwrap();
    }

    fn setup_session_legacy_layout(home: &Path, agent: &str, key: &str, lines: &[&str]) {
        let dir = home.join("agents").join(agent).join("sessions");
        std::fs::create_dir_all(&dir).unwrap();
        let content = lines.join("\n");
        std::fs::write(dir.join(format!("{key}.jsonl")), content).unwrap();
    }

    #[test]
    fn convert_basic_session() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        let dest = tmp.path().join("sessions");
        let mem = tmp.path().join("memory").join("sessions");

        setup_session(home, "main", "test-session", &[
            r#"{"type":"session-meta","agentId":"main"}"#,
            r#"{"type":"message","message":{"role":"user","content":"Hello"}}"#,
            r#"{"type":"message","message":{"role":"assistant","content":"Hi there!"}}"#,
            r#"{"type":"custom","customType":"model-snapshot","data":{}}"#,
        ]);

        let detection = make_detection(home);
        let report = import_sessions(&detection, &dest, &mem, &default_mapping());

        assert_eq!(report.status, ImportStatus::Success);
        assert_eq!(report.items_imported, 1);

        // Verify converted JSONL
        let converted_path = dest.join("oc_main_test-session.jsonl");
        assert!(converted_path.is_file());

        let content = std::fs::read_to_string(&converted_path).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 2); // Only message records, not meta/custom

        let first: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(first["role"], "user");
        assert_eq!(first["content"], "Hello");

        let second: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(second["role"], "assistant");
        assert_eq!(second["content"], "Hi there!");
    }

    #[test]
    fn convert_basic_session_legacy_layout() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        let dest = tmp.path().join("sessions");
        let mem = tmp.path().join("memory").join("sessions");

        setup_session_legacy_layout(home, "main", "legacy-session", &[
            r#"{"type":"session-meta","agentId":"main"}"#,
            r#"{"type":"message","message":{"role":"user","content":"Hello"}}"#,
            r#"{"type":"message","message":{"role":"assistant","content":"Hi there!"}}"#,
        ]);

        let detection = make_detection(home);
        let report = import_sessions(&detection, &dest, &mem, &default_mapping());

        assert_eq!(report.status, ImportStatus::Success);
        assert_eq!(report.items_imported, 1);
        assert!(dest.join("oc_main_legacy-session.jsonl").is_file());
    }

    #[test]
    fn convert_tool_messages() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        let dest = tmp.path().join("sessions");
        let mem = tmp.path().join("memory").join("sessions");

        setup_session(home, "main", "tools", &[
            r#"{"type":"message","message":{"role":"user","content":"Run ls"}}"#,
            r#"{"type":"message","message":{"role":"tool","content":"file.txt","toolUseId":"call_1"}}"#,
        ]);

        let detection = make_detection(home);
        let report = import_sessions(&detection, &dest, &mem, &default_mapping());

        assert_eq!(report.items_imported, 1);

        let content = std::fs::read_to_string(dest.join("oc_main_tools.jsonl")).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 2);

        let tool: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(tool["role"], "tool");
        assert_eq!(tool["tool_call_id"], "call_1");
    }

    #[test]
    fn import_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        let dest = tmp.path().join("sessions");
        let mem = tmp.path().join("memory").join("sessions");

        setup_session(home, "main", "existing", &[
            r#"{"type":"message","message":{"role":"user","content":"test"}}"#,
        ]);

        // First import
        let detection = make_detection(home);
        let report1 = import_sessions(&detection, &dest, &mem, &default_mapping());
        assert_eq!(report1.items_imported, 1);

        // Second import — should skip
        let report2 = import_sessions(&detection, &dest, &mem, &default_mapping());
        assert_eq!(report2.items_imported, 0);
        assert_eq!(report2.items_skipped, 1);
    }

    #[test]
    fn no_agents_returns_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        let mem = tmp.path().join("memory").join("sessions");
        let detection = OpenClawDetection {
            home_dir: tmp.path().to_path_buf(),
            has_config: false,
            has_credentials: false,
            has_mcp_servers: false,
            workspace_dir: tmp.path().join("workspace"),
            has_memory: false,
            has_skills: false,
            agent_ids: Vec::new(),
            session_count: 0,
            unsupported_channels: Vec::new(),
            has_workspace_files: false,
            workspace_files_found: Vec::new(),
        };

        let report = import_sessions(&detection, &tmp.path().join("dest"), &mem, &HashMap::new());
        assert_eq!(report.status, ImportStatus::Skipped);
    }

    #[test]
    fn multi_agent_imports_all() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        let dest = tmp.path().join("sessions");
        let mem = tmp.path().join("memory").join("sessions");

        setup_session(home, "main", "s1", &[
            r#"{"type":"message","message":{"role":"user","content":"hi from main"}}"#,
        ]);
        setup_session(home, "secondary", "s2", &[
            r#"{"type":"message","message":{"role":"user","content":"hi from secondary"}}"#,
        ]);

        let mut detection = make_detection(home);
        detection.agent_ids = vec!["main".to_string(), "secondary".to_string()];

        let mut mapping = HashMap::new();
        mapping.insert("main".to_string(), "main".to_string());
        mapping.insert("secondary".to_string(), "research".to_string());

        let report = import_sessions(&detection, &dest, &mem, &mapping);
        assert_eq!(report.status, ImportStatus::Success);
        assert_eq!(report.items_imported, 2);

        // Verify both sessions were imported with correct keys
        assert!(dest.join("oc_main_s1.jsonl").is_file());
        assert!(dest.join("oc_research_s2.jsonl").is_file());

        // Verify metadata has correct agent_id
        let metadata: HashMap<String, ImportedSessionEntry> =
            serde_json::from_str(&std::fs::read_to_string(dest.join("metadata.json")).unwrap())
                .unwrap();
        let main_entry = metadata.get("oc:main:s1").unwrap();
        assert!(main_entry.agent_id.is_none()); // default agent has no agent_id
        let research_entry = metadata.get("oc:research:s2").unwrap();
        assert_eq!(research_entry.agent_id.as_deref(), Some("research"));
    }

    #[test]
    fn session_metadata_written() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        let dest = tmp.path().join("sessions");
        let mem = tmp.path().join("memory").join("sessions");

        setup_session(home, "main", "meta-test", &[
            r#"{"type":"message","message":{"role":"user","content":"hello"}}"#,
        ]);

        let detection = make_detection(home);
        import_sessions(&detection, &dest, &mem, &default_mapping());

        let metadata_path = dest.join("metadata.json");
        assert!(metadata_path.is_file());

        let content = std::fs::read_to_string(&metadata_path).unwrap();
        let metadata: HashMap<String, serde_json::Value> = serde_json::from_str(&content).unwrap();
        assert!(metadata.contains_key("oc:main:meta-test"));
    }

    #[test]
    fn malformed_lines_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        let dest = tmp.path().join("sessions");
        let mem = tmp.path().join("memory").join("sessions");

        setup_session(home, "main", "messy", &[
            r#"not valid json"#,
            r#"{"type":"message","message":{"role":"user","content":"valid"}}"#,
            r#"{"broken":true}"#,
        ]);

        let detection = make_detection(home);
        let report = import_sessions(&detection, &dest, &mem, &default_mapping());

        assert_eq!(report.items_imported, 1);
        let content = std::fs::read_to_string(dest.join("oc_main_messy.jsonl")).unwrap();
        assert_eq!(content.lines().count(), 1);
    }

    #[test]
    fn session_transcript_written() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        let dest = tmp.path().join("sessions");
        let mem = tmp.path().join("memory").join("sessions");

        setup_session(home, "main", "chat", &[
            r#"{"type":"session-meta","agentId":"main"}"#,
            r#"{"type":"message","message":{"role":"user","content":"What is Rust?"}}"#,
            r#"{"type":"message","message":{"role":"assistant","content":"Rust is a systems programming language."}}"#,
            r#"{"type":"custom","customType":"model-snapshot","data":{"model":"claude-opus-4-6"}}"#,
        ]);

        let detection = make_detection(home);
        let report = import_sessions(&detection, &dest, &mem, &default_mapping());

        assert_eq!(report.items_imported, 1);

        // Verify transcript markdown was written
        let transcript_path = mem.join("oc-main-chat.md");
        assert!(transcript_path.is_file());

        let content = std::fs::read_to_string(&transcript_path).unwrap();
        assert!(content.contains("# Session: OpenClaw: chat"));
        assert!(content.contains("Imported from OpenClaw"));
        assert!(content.contains("Model: claude-opus-4-6"));
        assert!(content.contains("**User:** What is Rust?"));
        assert!(content.contains("**Assistant:** Rust is a systems programming language."));
    }

    #[test]
    fn model_extracted_from_custom_record() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        let dest = tmp.path().join("sessions");
        let mem = tmp.path().join("memory").join("sessions");

        setup_session(home, "main", "model-test", &[
            r#"{"type":"custom","customType":"model-snapshot","data":{"model":"gpt-4o"}}"#,
            r#"{"type":"message","message":{"role":"user","content":"test"}}"#,
        ]);

        let detection = make_detection(home);
        import_sessions(&detection, &dest, &mem, &default_mapping());

        let metadata_path = dest.join("metadata.json");
        let content = std::fs::read_to_string(&metadata_path).unwrap();
        let metadata: HashMap<String, serde_json::Value> = serde_json::from_str(&content).unwrap();
        let entry = metadata.get("oc:main:model-test").unwrap();
        assert_eq!(entry["model"], "gpt-4o");
    }

    #[test]
    fn incremental_import_detects_growth() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        let dest = tmp.path().join("sessions");
        let mem = tmp.path().join("memory").join("sessions");

        // Initial import with 1 message
        setup_session(home, "main", "growing", &[
            r#"{"type":"message","message":{"role":"user","content":"Hello"}}"#,
        ]);

        let detection = make_detection(home);
        let report1 = import_sessions(&detection, &dest, &mem, &default_mapping());
        assert_eq!(report1.items_imported, 1);
        assert_eq!(report1.items_updated, 0);

        let content1 = std::fs::read_to_string(dest.join("oc_main_growing.jsonl")).unwrap();
        assert_eq!(content1.lines().count(), 1);

        // Append a new message to the source
        setup_session(home, "main", "growing", &[
            r#"{"type":"message","message":{"role":"user","content":"Hello"}}"#,
            r#"{"type":"message","message":{"role":"assistant","content":"Hi there!"}}"#,
        ]);

        // Re-import should detect growth and update
        let report2 = import_sessions(&detection, &dest, &mem, &default_mapping());
        assert_eq!(report2.items_imported, 0);
        assert_eq!(report2.items_updated, 1);
        assert_eq!(report2.items_skipped, 0);

        // Destination should now have 2 messages
        let content2 = std::fs::read_to_string(dest.join("oc_main_growing.jsonl")).unwrap();
        assert_eq!(content2.lines().count(), 2);
    }

    #[test]
    fn incremental_import_noop_when_unchanged() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        let dest = tmp.path().join("sessions");
        let mem = tmp.path().join("memory").join("sessions");

        setup_session(home, "main", "stable", &[
            r#"{"type":"message","message":{"role":"user","content":"test"}}"#,
        ]);

        let detection = make_detection(home);
        let report1 = import_sessions(&detection, &dest, &mem, &default_mapping());
        assert_eq!(report1.items_imported, 1);

        // Re-import without changes — should skip
        let report2 = import_sessions(&detection, &dest, &mem, &default_mapping());
        assert_eq!(report2.items_imported, 0);
        assert_eq!(report2.items_updated, 0);
        assert_eq!(report2.items_skipped, 1);
    }

    #[test]
    fn incremental_import_preserves_id_and_created_at() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        let dest = tmp.path().join("sessions");
        let mem = tmp.path().join("memory").join("sessions");

        setup_session(home, "main", "preserve", &[
            r#"{"type":"message","message":{"role":"user","content":"first"}}"#,
        ]);

        let detection = make_detection(home);
        import_sessions(&detection, &dest, &mem, &default_mapping());

        // Read original metadata
        let metadata_path = dest.join("metadata.json");
        let content = std::fs::read_to_string(&metadata_path).unwrap();
        let metadata: HashMap<String, ImportedSessionEntry> =
            serde_json::from_str(&content).unwrap();
        let original = metadata.get("oc:main:preserve").unwrap();
        let original_id = original.id.clone();
        let original_created_at = original.created_at;
        assert_eq!(original.version, 0);

        // Append and re-import
        setup_session(home, "main", "preserve", &[
            r#"{"type":"message","message":{"role":"user","content":"first"}}"#,
            r#"{"type":"message","message":{"role":"assistant","content":"second"}}"#,
        ]);

        import_sessions(&detection, &dest, &mem, &default_mapping());

        let content2 = std::fs::read_to_string(&metadata_path).unwrap();
        let metadata2: HashMap<String, ImportedSessionEntry> =
            serde_json::from_str(&content2).unwrap();
        let updated = metadata2.get("oc:main:preserve").unwrap();

        assert_eq!(updated.id, original_id);
        assert_eq!(updated.created_at, original_created_at);
        assert_eq!(updated.version, 1);
        assert_eq!(updated.message_count, 2);
    }

    #[test]
    fn incremental_import_regenerates_transcript() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        let dest = tmp.path().join("sessions");
        let mem = tmp.path().join("memory").join("sessions");

        setup_session(home, "main", "transcript", &[
            r#"{"type":"message","message":{"role":"user","content":"What is Rust?"}}"#,
        ]);

        let detection = make_detection(home);
        import_sessions(&detection, &dest, &mem, &default_mapping());

        let transcript_path = mem.join("oc-main-transcript.md");
        let content1 = std::fs::read_to_string(&transcript_path).unwrap();
        assert!(content1.contains("**User:** What is Rust?"));
        assert!(!content1.contains("systems programming language"));

        // Append response and re-import
        setup_session(home, "main", "transcript", &[
            r#"{"type":"message","message":{"role":"user","content":"What is Rust?"}}"#,
            r#"{"type":"message","message":{"role":"assistant","content":"A systems programming language."}}"#,
        ]);

        import_sessions(&detection, &dest, &mem, &default_mapping());

        let content2 = std::fs::read_to_string(&transcript_path).unwrap();
        assert!(content2.contains("**User:** What is Rust?"));
        assert!(content2.contains("**Assistant:** A systems programming language."));
    }

    #[test]
    fn incremental_import_upgrades_legacy_metadata() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        let dest = tmp.path().join("sessions");
        let mem = tmp.path().join("memory").join("sessions");
        std::fs::create_dir_all(&dest).unwrap();

        setup_session(home, "main", "legacy", &[
            r#"{"type":"message","message":{"role":"user","content":"old message"}}"#,
        ]);

        // Write legacy metadata without source_line_count (will deserialize as 0)
        let legacy_metadata = serde_json::json!({
            "oc:main:legacy": {
                "id": "legacy-id-123",
                "key": "oc:main:legacy",
                "label": "OpenClaw: legacy",
                "model": null,
                "created_at": 1000,
                "updated_at": 1000,
                "message_count": 1,
                "last_seen_message_count": 0,
                "archived": false,
                "version": 0
            }
        });
        std::fs::write(
            dest.join("metadata.json"),
            serde_json::to_string_pretty(&legacy_metadata).unwrap(),
        )
        .unwrap();

        // Also write a destination JSONL so it looks like a previous import happened
        std::fs::write(
            dest.join("oc_main_legacy.jsonl"),
            r#"{"role":"user","content":"old message"}"#,
        )
        .unwrap();

        // Re-import should detect legacy (source_line_count == 0) and re-import
        let detection = make_detection(home);
        let report = import_sessions(&detection, &dest, &mem, &default_mapping());
        assert_eq!(report.items_updated, 1);
        assert_eq!(report.items_imported, 0);

        // Verify metadata now has source_line_count set
        let metadata_path = dest.join("metadata.json");
        let content = std::fs::read_to_string(&metadata_path).unwrap();
        let metadata: HashMap<String, ImportedSessionEntry> =
            serde_json::from_str(&content).unwrap();
        let entry = metadata.get("oc:main:legacy").unwrap();
        assert!(entry.source_line_count > 0);
        assert_eq!(entry.id, "legacy-id-123"); // Preserved
        assert_eq!(entry.version, 1); // Bumped
    }

    #[test]
    fn preserves_source_timestamps_and_preview() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        let dest = tmp.path().join("sessions");
        let mem = tmp.path().join("memory").join("sessions");

        setup_session(home, "main", "timed", &[
            r#"{"type":"message","timestamp":"2026-01-28T06:46:35.768Z","message":{"role":"user","content":"hello from old openclaw","timestamp":1769582795764}}"#,
            r#"{"type":"message","timestamp":"2026-01-28T06:46:41.626Z","message":{"role":"assistant","content":"this is the assistant reply","timestamp":1769582801626}}"#,
        ]);

        let detection = make_detection(home);
        let report = import_sessions(&detection, &dest, &mem, &default_mapping());
        assert_eq!(report.status, ImportStatus::Success);
        assert_eq!(report.items_imported, 1);

        let converted = std::fs::read_to_string(dest.join("oc_main_timed.jsonl")).unwrap();
        let mut lines = converted.lines();
        let first: serde_json::Value = serde_json::from_str(lines.next().unwrap()).unwrap();
        let second: serde_json::Value = serde_json::from_str(lines.next().unwrap()).unwrap();
        assert_eq!(first["created_at"], 1769582795764_u64);
        assert_eq!(second["created_at"], 1769582801626_u64);

        let metadata: HashMap<String, ImportedSessionEntry> =
            serde_json::from_str(&std::fs::read_to_string(dest.join("metadata.json")).unwrap())
                .unwrap();
        let entry = metadata.get("oc:main:timed").unwrap();
        assert_eq!(entry.created_at, 1769582795764_u64);
        assert_eq!(entry.updated_at, 1769582801626_u64);
        assert!(
            entry
                .preview
                .as_deref()
                .is_some_and(|p| p.contains("hello from old openclaw"))
        );
    }

    fn write_sessions_json(home: &Path, agent: &str, content: &str) {
        let dir = home
            .join("agents")
            .join(agent)
            .join("agent")
            .join("sessions");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("sessions.json"), content).unwrap();
    }

    #[test]
    fn session_labels_from_index() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        let dest = tmp.path().join("sessions");
        let mem = tmp.path().join("memory").join("sessions");

        let session_uuid = "d8da3601-1234-4abc-9def-aabbccddeeff";

        // Create sessions.json index with rich metadata
        write_sessions_json(
            home,
            "main",
            &serde_json::json!({
                "agent:main:main": {
                    "sessionId": session_uuid,
                    "updatedAt": 1770000000000_u64,
                    "origin": {
                        "label": "Fabien (@fabienpenso) id:377114917",
                        "chatType": "direct"
                    }
                }
            })
            .to_string(),
        );

        // Create matching JSONL file named by UUID
        setup_session(home, "main", session_uuid, &[
            r#"{"type":"message","message":{"role":"user","content":"Hello"}}"#,
            r#"{"type":"message","message":{"role":"assistant","content":"Hi!"}}"#,
        ]);

        let detection = make_detection(home);
        let report = import_sessions(&detection, &dest, &mem, &default_mapping());
        assert_eq!(report.status, ImportStatus::Success);
        assert_eq!(report.items_imported, 1);

        // Verify label is "Main" (from agent:main:main key pattern)
        let metadata: HashMap<String, ImportedSessionEntry> =
            serde_json::from_str(&std::fs::read_to_string(dest.join("metadata.json")).unwrap())
                .unwrap();
        let key = format!("oc:main:{session_uuid}");
        let entry = metadata.get(&key).unwrap();
        assert_eq!(entry.label.as_deref(), Some("Main"));
    }

    #[test]
    fn session_label_telegram_from_index() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        let dest = tmp.path().join("sessions");
        let mem = tmp.path().join("memory").join("sessions");

        let session_uuid = "aabb1122-3344-4556-8899-001122334455";

        write_sessions_json(
            home,
            "main",
            &serde_json::json!({
                "agent:main:telegram:dm:377114917": {
                    "sessionId": session_uuid,
                    "updatedAt": 1770000000000_u64,
                    "origin": {
                        "label": "Fabien (@fabienpenso) id:377114917",
                        "provider": "telegram",
                        "chatType": "direct"
                    }
                }
            })
            .to_string(),
        );

        setup_session(home, "main", session_uuid, &[
            r#"{"type":"message","message":{"role":"user","content":"Hi via Telegram"}}"#,
        ]);

        let detection = make_detection(home);
        let report = import_sessions(&detection, &dest, &mem, &default_mapping());
        assert_eq!(report.status, ImportStatus::Success);

        let metadata: HashMap<String, ImportedSessionEntry> =
            serde_json::from_str(&std::fs::read_to_string(dest.join("metadata.json")).unwrap())
                .unwrap();
        let key = format!("oc:main:{session_uuid}");
        let entry = metadata.get(&key).unwrap();
        assert_eq!(entry.label.as_deref(), Some("Telegram: Fabien"));
    }

    #[test]
    fn session_label_cron_from_index() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        let dest = tmp.path().join("sessions");
        let mem = tmp.path().join("memory").join("sessions");

        let session_uuid = "cron1234-5678-4abc-9def-000000000001";

        write_sessions_json(
            home,
            "main",
            &serde_json::json!({
                "agent:main:cron:daily-summary": {
                    "sessionId": session_uuid,
                    "updatedAt": 1770000000000_u64
                }
            })
            .to_string(),
        );

        setup_session(home, "main", session_uuid, &[
            r#"{"type":"message","message":{"role":"assistant","content":"Daily summary done"}}"#,
        ]);

        let detection = make_detection(home);
        let report = import_sessions(&detection, &dest, &mem, &default_mapping());
        assert_eq!(report.status, ImportStatus::Success);

        let metadata: HashMap<String, ImportedSessionEntry> =
            serde_json::from_str(&std::fs::read_to_string(dest.join("metadata.json")).unwrap())
                .unwrap();
        let key = format!("oc:main:{session_uuid}");
        let entry = metadata.get(&key).unwrap();
        assert_eq!(entry.label.as_deref(), Some("Cron"));
    }

    #[test]
    fn session_label_fallback_without_index() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        let dest = tmp.path().join("sessions");
        let mem = tmp.path().join("memory").join("sessions");

        // No sessions.json — just a JSONL file with a UUID-like name
        setup_session(home, "main", "abcd1234-dead-beef-cafe-112233445566", &[
            r#"{"type":"message","message":{"role":"user","content":"test"}}"#,
        ]);

        let detection = make_detection(home);
        let report = import_sessions(&detection, &dest, &mem, &default_mapping());
        assert_eq!(report.status, ImportStatus::Success);

        let metadata: HashMap<String, ImportedSessionEntry> =
            serde_json::from_str(&std::fs::read_to_string(dest.join("metadata.json")).unwrap())
                .unwrap();
        let entry = metadata
            .get("oc:main:abcd1234-dead-beef-cafe-112233445566")
            .unwrap();
        // Fallback: first 8 chars of the stem
        assert_eq!(entry.label.as_deref(), Some("OpenClaw: abcd1234"));
    }

    #[test]
    fn build_session_label_unit_tests() {
        // Main session
        assert_eq!(build_session_label("agent:main:main", None), "Main");

        // Telegram with label
        let origin = OpenClawSessionOrigin {
            label: Some("Alice (@alice) id:123".to_string()),
            provider: Some("telegram".to_string()),
            chat_type: Some("direct".to_string()),
        };
        assert_eq!(
            build_session_label("agent:main:telegram:dm:123", Some(&origin)),
            "Telegram: Alice"
        );

        // Telegram without label
        assert_eq!(
            build_session_label("agent:main:telegram:dm:123", None),
            "Telegram"
        );

        // Signal with label
        let signal_origin = OpenClawSessionOrigin {
            label: Some("Bob".to_string()),
            provider: Some("signal".to_string()),
            chat_type: Some("direct".to_string()),
        };
        assert_eq!(
            build_session_label("agent:main:signal:dm:+1234567890", Some(&signal_origin)),
            "Signal: Bob"
        );

        // Cron
        assert_eq!(build_session_label("agent:main:cron:daily", None), "Cron");

        // Unknown key with origin label
        let other_origin = OpenClawSessionOrigin {
            label: Some("Charlie".to_string()),
            provider: None,
            chat_type: None,
        };
        assert_eq!(
            build_session_label("agent:main:unknown:thing", Some(&other_origin)),
            "OpenClaw: Charlie"
        );

        // Unknown key without origin
        assert_eq!(
            build_session_label("agent:main:unknown:thing", None),
            "OpenClaw: agent:main:unknown:thing"
        );
    }

    #[test]
    fn load_session_label_map_parses_index() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();

        std::fs::write(
            dir.join("sessions.json"),
            serde_json::json!({
                "agent:main:main": {
                    "sessionId": "uuid-1",
                    "updatedAt": 1000,
                    "origin": { "label": "Test", "chatType": "direct" }
                },
                "agent:main:telegram:dm:99": {
                    "sessionId": "uuid-2",
                    "updatedAt": 2000,
                    "origin": { "label": "User99", "provider": "telegram" }
                },
                "agent:main:no-session-id": {
                    "updatedAt": 3000
                }
            })
            .to_string(),
        )
        .unwrap();

        let map = load_session_label_map(dir);
        assert_eq!(map.len(), 2); // no-session-id entry is excluded
        assert!(map.contains_key("uuid-1"));
        assert!(map.contains_key("uuid-2"));

        let (key, origin) = &map["uuid-1"];
        assert_eq!(key, "agent:main:main");
        assert_eq!(
            origin.as_ref().and_then(|o| o.label.as_deref()),
            Some("Test")
        );
    }
}
