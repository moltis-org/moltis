use {
    serde::{Deserialize, Serialize},
    std::collections::HashMap,
};

/// Hooks configuration section (shell hooks defined in config file).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HooksConfig {
    #[serde(default)]
    pub hooks: Vec<ShellHookConfigEntry>,
}

/// A single shell hook defined in the config file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShellHookConfigEntry {
    pub name: String,
    pub command: String,
    pub events: Vec<String>,
    #[serde(default = "default_hook_timeout")]
    pub timeout: u64,
    #[serde(default)]
    pub env: HashMap<String, String>,
}

fn default_hook_timeout() -> u64 {
    10
}
