//! Session export: save sanitized session transcripts to memory for cross-run recall.
//!
//! This module provides functionality to export chat sessions as markdown files
//! that can be indexed by the memory system, enabling recall of past conversations.

use std::path::{Path, PathBuf};

use {
    chrono::{DateTime, Utc},
    tokio::fs,
    tracing::{debug, info, warn},
};

/// Configuration for session export.
#[derive(Debug, Clone)]
pub struct SessionExportConfig {
    /// Directory where session exports are stored.
    pub export_dir: PathBuf,
    /// Maximum number of exports to keep (0 = unlimited).
    pub max_exports: usize,
    /// Maximum age of exports in days (0 = unlimited).
    pub max_age_days: u64,
}

impl Default for SessionExportConfig {
    fn default() -> Self {
        Self {
            export_dir: PathBuf::from("memory/sessions"),
            max_exports: 100,
            max_age_days: 30,
        }
    }
}

/// A turn in a session transcript.
#[derive(Debug, Clone)]
pub struct SessionTurn {
    /// Role: "user" or "assistant"
    pub role: String,
    /// The message content (sanitized)
    pub content: String,
    /// Optional timestamp
    pub timestamp: Option<DateTime<Utc>>,
}

/// A session transcript for export.
#[derive(Debug, Clone)]
pub struct SessionTranscript {
    /// Session identifier
    pub session_id: String,
    /// Session title or topic
    pub title: Option<String>,
    /// Project name (if associated)
    pub project: Option<String>,
    /// When the session was created
    pub created_at: DateTime<Utc>,
    /// The conversation turns
    pub turns: Vec<SessionTurn>,
}

impl SessionTranscript {
    /// Create a new session transcript.
    pub fn new(session_id: impl Into<String>) -> Self {
        Self {
            session_id: session_id.into(),
            title: None,
            project: None,
            created_at: Utc::now(),
            turns: Vec::new(),
        }
    }

    /// Add a turn to the transcript.
    pub fn add_turn(&mut self, role: impl Into<String>, content: impl Into<String>) {
        self.turns.push(SessionTurn {
            role: role.into(),
            content: sanitize_content(&content.into()),
            timestamp: Some(Utc::now()),
        });
    }

    /// Convert to markdown format for storage.
    pub fn to_markdown(&self) -> String {
        let mut md = String::new();

        // Header
        md.push_str("---\n");
        md.push_str(&format!("session_id: {}\n", self.session_id));
        if let Some(ref title) = self.title {
            md.push_str(&format!("title: {}\n", title));
        }
        if let Some(ref project) = self.project {
            md.push_str(&format!("project: {}\n", project));
        }
        md.push_str(&format!("created_at: {}\n", self.created_at.to_rfc3339()));
        md.push_str("type: session_export\n");
        md.push_str("---\n\n");

        // Title
        if let Some(ref title) = self.title {
            md.push_str(&format!("# {}\n\n", title));
        } else {
            md.push_str(&format!(
                "# Session {}\n\n",
                &self.session_id[..8.min(self.session_id.len())]
            ));
        }

        // Metadata
        if let Some(ref project) = self.project {
            md.push_str(&format!("**Project:** {}\n", project));
        }
        md.push_str(&format!(
            "**Date:** {}\n\n",
            self.created_at.format("%Y-%m-%d %H:%M UTC")
        ));

        md.push_str("---\n\n");

        // Conversation turns
        for turn in &self.turns {
            let role_label = match turn.role.as_str() {
                "user" => "**User:**",
                "assistant" => "**Assistant:**",
                _ => &turn.role,
            };
            md.push_str(role_label);
            md.push('\n');
            md.push_str(&turn.content);
            md.push_str("\n\n");
        }

        md
    }
}

/// Sanitize content for export.
/// Removes potentially sensitive information like:
/// - Tool call results with raw data
/// - System messages
/// - Internal metadata
fn sanitize_content(content: &str) -> String {
    // Keep the content mostly as-is, but:
    // 1. Remove any JSON tool result blocks
    // 2. Truncate very long content
    // 3. Remove control characters

    let mut sanitized = String::with_capacity(content.len());

    for line in content.lines() {
        // Skip lines that look like tool result JSON
        if line.trim_start().starts_with('{') && line.contains("\"type\"") {
            continue;
        }
        // Skip system message markers
        if line.starts_with("<system") || line.starts_with("</system") {
            continue;
        }
        sanitized.push_str(line);
        sanitized.push('\n');
    }

    // Truncate to reasonable length (preserve meaningful content)
    const MAX_CONTENT_LEN: usize = 10_000;
    if sanitized.len() > MAX_CONTENT_LEN {
        let truncated = &sanitized[..MAX_CONTENT_LEN];
        // Find last complete sentence or paragraph
        if let Some(pos) = truncated.rfind("\n\n") {
            sanitized = truncated[..pos].to_string();
        } else if let Some(pos) = truncated.rfind(". ") {
            sanitized = truncated[..=pos].to_string();
        } else {
            // No good break point found, just truncate at limit
            sanitized = truncated.to_string();
        }
        sanitized.push_str("\n\n[Content truncated...]");
    }

    sanitized.trim().to_string()
}

/// Session exporter handles writing transcripts to disk.
pub struct SessionExporter {
    config: SessionExportConfig,
}

impl SessionExporter {
    /// Create a new session exporter with the given configuration.
    pub fn new(config: SessionExportConfig) -> Self {
        Self { config }
    }

    /// Create with default configuration rooted at the given data directory.
    pub fn with_data_dir(data_dir: &Path) -> Self {
        Self {
            config: SessionExportConfig {
                export_dir: data_dir.join("memory").join("sessions"),
                ..Default::default()
            },
        }
    }

    /// Export a session transcript to a markdown file.
    pub async fn export(&self, transcript: &SessionTranscript) -> anyhow::Result<PathBuf> {
        // Ensure export directory exists
        fs::create_dir_all(&self.config.export_dir).await?;

        // Generate filename from date and session ID
        let date = transcript.created_at.format("%Y-%m-%d");
        let short_id = &transcript.session_id[..8.min(transcript.session_id.len())];
        let filename = format!("session-{}-{}.md", date, short_id);
        let filepath = self.config.export_dir.join(&filename);

        // Write the markdown content
        let content = transcript.to_markdown();
        fs::write(&filepath, &content).await?;

        info!(
            session_id = %transcript.session_id,
            path = %filepath.display(),
            turns = transcript.turns.len(),
            "exported session to memory"
        );

        // Clean up old exports if needed
        if (self.config.max_exports > 0 || self.config.max_age_days > 0)
            && let Err(e) = self.cleanup().await
        {
            warn!(error = %e, "failed to cleanup old session exports");
        }

        Ok(filepath)
    }

    /// Clean up old exports based on configuration limits.
    async fn cleanup(&self) -> anyhow::Result<()> {
        let mut entries = Vec::new();
        let mut dir = fs::read_dir(&self.config.export_dir).await?;

        while let Some(entry) = dir.next_entry().await? {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "md")
                && path
                    .file_name()
                    .is_some_and(|n| n.to_string_lossy().starts_with("session-"))
                && let Ok(meta) = entry.metadata().await
                && let Ok(modified) = meta.modified()
            {
                entries.push((path, modified));
            }
        }

        // Sort by modification time (newest first)
        entries.sort_by_key(|e| std::cmp::Reverse(e.1));

        let now = std::time::SystemTime::now();
        let max_age = std::time::Duration::from_secs(self.config.max_age_days * 24 * 60 * 60);

        for (i, (path, modified)) in entries.iter().enumerate() {
            let should_remove = (self.config.max_exports > 0 && i >= self.config.max_exports)
                || (self.config.max_age_days > 0
                    && now.duration_since(*modified).unwrap_or_default() > max_age);

            if should_remove {
                debug!(path = %path.display(), "removing old session export");
                let _ = fs::remove_file(&path).await;
            }
        }

        Ok(())
    }
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use {super::*, tempfile::TempDir};

    #[test]
    fn test_sanitize_content_basic() {
        let content = "Hello, this is a test message.";
        assert_eq!(sanitize_content(content), content);
    }

    #[test]
    fn test_sanitize_content_removes_system_tags() {
        let content = "<system-reminder>\nSome system text\n</system-reminder>\nActual content";
        let sanitized = sanitize_content(content);
        assert!(!sanitized.contains("<system"));
        assert!(sanitized.contains("Actual content"));
    }

    #[test]
    fn test_sanitize_content_truncates_long_content() {
        let long_content = "x".repeat(20_000);
        let sanitized = sanitize_content(&long_content);
        assert!(sanitized.len() < 15_000);
        assert!(sanitized.contains("[Content truncated...]"));
    }

    #[test]
    fn test_transcript_to_markdown() {
        let mut transcript = SessionTranscript::new("test-session-123");
        transcript.title = Some("Test Conversation".into());
        transcript.project = Some("my-project".into());
        transcript.add_turn("user", "Hello, how are you?");
        transcript.add_turn("assistant", "I'm doing well, thank you!");

        let md = transcript.to_markdown();

        assert!(md.contains("session_id: test-session-123"));
        assert!(md.contains("title: Test Conversation"));
        assert!(md.contains("project: my-project"));
        assert!(md.contains("**User:**"));
        assert!(md.contains("Hello, how are you?"));
        assert!(md.contains("**Assistant:**"));
        assert!(md.contains("I'm doing well, thank you!"));
    }

    #[tokio::test]
    async fn test_export_creates_file() {
        let tmp = TempDir::new().unwrap();
        let exporter = SessionExporter::with_data_dir(tmp.path());

        let mut transcript = SessionTranscript::new("abc12345");
        transcript.add_turn("user", "Test message");

        let path = exporter.export(&transcript).await.unwrap();

        assert!(path.exists());
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("session_id: abc12345"));
        assert!(content.contains("Test message"));
    }

    #[tokio::test]
    async fn test_cleanup_removes_excess_exports() {
        let tmp = TempDir::new().unwrap();
        let config = SessionExportConfig {
            export_dir: tmp.path().join("sessions"),
            max_exports: 2,
            max_age_days: 0,
        };
        let exporter = SessionExporter::new(config);

        // Create 3 exports with unique session IDs (first 8 chars must differ!)
        let session_ids = ["aaaaaaaa-1", "bbbbbbbb-2", "cccccccc-3"];
        for (i, sid) in session_ids.iter().enumerate() {
            let mut transcript = SessionTranscript::new(*sid);
            transcript.add_turn("user", format!("Message {}", i));
            exporter.export(&transcript).await.unwrap();
            // Delay to ensure different modification times
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }

        // Wait a bit for cleanup to complete
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        // Should only have 2 files (max_exports = 2)
        let mut count = 0;
        let mut dir = fs::read_dir(tmp.path().join("sessions")).await.unwrap();
        while dir.next_entry().await.unwrap().is_some() {
            count += 1;
        }
        assert_eq!(count, 2);
    }
}
