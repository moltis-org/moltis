use serde::{Deserialize, Serialize};

/// External agent transport kind for session binding.
///
/// Defined in `moltis-protocol` so session storage, config validation, runtime
/// crates and native clients share one type.
/// Serialises to kebab-case strings matching the canonical agent identifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ExternalAgentKind {
    ClaudeCode,
    Opencode,
    Codex,
    PiAgent,
    Acp,
    AcpCopilot,
    AcpCodex,
    AcpClaude,
    AcpPi,
    AcpOpencode,
    AcpGemini,
    AcpAugment,
    AcpKiro,
    AcpOpenclaw,
    AcpOpenhands,
    AcpKimi,
    AcpMinimaxCode,
    AcpStakpak,
    AcpFastAgent,
}

impl ExternalAgentKind {
    pub const ALL: &'static [Self] = &[
        Self::ClaudeCode,
        Self::Opencode,
        Self::Codex,
        Self::PiAgent,
        Self::Acp,
        Self::AcpCopilot,
        Self::AcpCodex,
        Self::AcpClaude,
        Self::AcpPi,
        Self::AcpOpencode,
        Self::AcpGemini,
        Self::AcpAugment,
        Self::AcpKiro,
        Self::AcpOpenclaw,
        Self::AcpOpenhands,
        Self::AcpKimi,
        Self::AcpMinimaxCode,
        Self::AcpStakpak,
        Self::AcpFastAgent,
    ];

    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ClaudeCode => "claude-code",
            Self::Opencode => "opencode",
            Self::Codex => "codex",
            Self::PiAgent => "pi-agent",
            Self::Acp => "acp",
            Self::AcpCopilot => "acp-copilot",
            Self::AcpCodex => "acp-codex",
            Self::AcpClaude => "acp-claude",
            Self::AcpPi => "acp-pi",
            Self::AcpOpencode => "acp-opencode",
            Self::AcpGemini => "acp-gemini",
            Self::AcpAugment => "acp-augment",
            Self::AcpKiro => "acp-kiro",
            Self::AcpOpenclaw => "acp-openclaw",
            Self::AcpOpenhands => "acp-openhands",
            Self::AcpKimi => "acp-kimi",
            Self::AcpMinimaxCode => "acp-minimax-code",
            Self::AcpStakpak => "acp-stakpak",
            Self::AcpFastAgent => "acp-fast-agent",
        }
    }

    #[must_use]
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::ClaudeCode => "Claude Code",
            Self::Opencode => "opencode",
            Self::Codex => "Codex",
            Self::PiAgent => "Pi agent",
            Self::Acp => "ACP",
            Self::AcpCopilot => "ACP: Copilot",
            Self::AcpCodex => "ACP: Codex",
            Self::AcpClaude => "ACP: Claude",
            Self::AcpPi => "ACP: Pi",
            Self::AcpOpencode => "ACP: opencode",
            Self::AcpGemini => "ACP: Gemini",
            Self::AcpAugment => "ACP: Augment",
            Self::AcpKiro => "ACP: Kiro",
            Self::AcpOpenclaw => "ACP: OpenClaw",
            Self::AcpOpenhands => "ACP: OpenHands",
            Self::AcpKimi => "ACP: Kimi",
            Self::AcpMinimaxCode => "ACP: MiniMax Code",
            Self::AcpStakpak => "ACP: Stakpak",
            Self::AcpFastAgent => "ACP: fast-agent",
        }
    }

    #[must_use]
    pub fn is_acp(&self) -> bool {
        matches!(
            self,
            Self::Acp
                | Self::AcpCopilot
                | Self::AcpCodex
                | Self::AcpClaude
                | Self::AcpPi
                | Self::AcpOpencode
                | Self::AcpGemini
                | Self::AcpAugment
                | Self::AcpKiro
                | Self::AcpOpenclaw
                | Self::AcpOpenhands
                | Self::AcpKimi
                | Self::AcpMinimaxCode
                | Self::AcpStakpak
                | Self::AcpFastAgent
        )
    }
}

impl std::fmt::Display for ExternalAgentKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for ExternalAgentKind {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "claude-code" => Ok(Self::ClaudeCode),
            "opencode" => Ok(Self::Opencode),
            "codex" => Ok(Self::Codex),
            "pi-agent" => Ok(Self::PiAgent),
            "acp" => Ok(Self::Acp),
            "acp-copilot" => Ok(Self::AcpCopilot),
            "acp-codex" => Ok(Self::AcpCodex),
            "acp-claude" => Ok(Self::AcpClaude),
            "acp-pi" => Ok(Self::AcpPi),
            "acp-opencode" => Ok(Self::AcpOpencode),
            "acp-gemini" => Ok(Self::AcpGemini),
            "acp-augment" => Ok(Self::AcpAugment),
            "acp-kiro" => Ok(Self::AcpKiro),
            "acp-openclaw" => Ok(Self::AcpOpenclaw),
            "acp-openhands" => Ok(Self::AcpOpenhands),
            "acp-kimi" => Ok(Self::AcpKimi),
            "acp-minimax-code" => Ok(Self::AcpMinimaxCode),
            "acp-stakpak" => Ok(Self::AcpStakpak),
            "acp-fast-agent" => Ok(Self::AcpFastAgent),
            other => Err(format!("unknown external agent kind: {other}")),
        }
    }
}
