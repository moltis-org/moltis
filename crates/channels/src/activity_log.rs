//! User-facing channel Activity log visibility.

/// User-facing activity log visibility for channel replies.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActivityLogMode {
    /// Show all buffered activity entries.
    #[default]
    All,
    /// Show failed tool/activity entries only.
    ErrorsOnly,
    /// Do not append or send activity log entries.
    Off,
}

impl ActivityLogMode {
    pub fn includes(self, kind: ChannelStatusLogKind) -> bool {
        match self {
            Self::All => true,
            Self::ErrorsOnly => kind == ChannelStatusLogKind::Error,
            Self::Off => false,
        }
    }

    pub fn is_all(&self) -> bool {
        *self == Self::All
    }
}

/// Type of buffered channel activity entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelStatusLogKind {
    Info,
    Error,
}

/// Buffered activity entry appended to channel replies when enabled.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ChannelStatusLogEntry {
    pub kind: ChannelStatusLogKind,
    pub message: String,
}

impl ChannelStatusLogEntry {
    pub fn info(message: impl Into<String>) -> Self {
        Self {
            kind: ChannelStatusLogKind::Info,
            message: message.into(),
        }
    }

    pub fn error(message: impl Into<String>) -> Self {
        Self {
            kind: ChannelStatusLogKind::Error,
            message: message.into(),
        }
    }
}
