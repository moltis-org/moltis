#[cfg(feature = "claude-code")]
pub mod claude_code;

#[cfg(feature = "opencode")]
pub mod opencode;

#[cfg(feature = "codex")]
pub mod codex;

#[cfg(feature = "pi-agent")]
pub mod pi_agent;

#[cfg(feature = "acp")]
pub mod acp;

mod process;
