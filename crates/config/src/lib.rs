//! Configuration loading, validation, env substitution, and legacy migration.
//!
//! Config files: `moltis.toml`, `moltis.yaml`, or `moltis.json`
//! Searched in `./` then `~/.config/moltis/`.
//!
//! Supports `${ENV_VAR}` substitution in all string values.

pub mod env_subst;
pub mod error;
pub mod loader;
pub mod migrate;
pub mod schema;
pub mod template;
pub mod validate;

pub use {
    error::{Error, Result},
    loader::{
        DEFAULT_SOUL, agent_workspace_dir, agents_path, apply_env_overrides, clear_config_dir,
        clear_data_dir, config_dir, data_dir, discover_and_load, extract_yaml_frontmatter,
        find_or_default_config_path, find_user_global_config_file, heartbeat_path, home_dir,
        identity_path, load_agents_md, load_agents_md_for_agent, load_heartbeat_md, load_identity,
        load_identity_for_agent, load_memory_md, load_memory_md_for_agent, load_soul,
        load_soul_for_agent, load_tools_md, load_tools_md_for_agent, load_user, memory_path,
        resolve_identity, resolve_identity_from_config, save_config, save_identity,
        save_identity_for_agent, save_raw_config, save_soul, save_user, set_config_dir,
        set_data_dir, soul_path, tools_path, update_config, user_global_config_dir,
        user_global_config_dir_if_different, user_path,
    },
    schema::{
        AgentIdentity, AgentPresetConfig, AgentsConfig, AuthConfig, CalDavAccountConfig,
        CalDavConfig, ChatConfig, GeoLocation, MessageQueueMode, MoltisConfig, ResolvedIdentity,
        Timezone, ToolMode, UserProfile, VoiceConfig, VoiceElevenLabsConfig, VoiceOpenAiConfig,
        VoiceSttConfig, VoiceSttProvider, VoiceTtsConfig, VoiceWhisperConfig,
    },
    validate::{Diagnostic, Severity, ValidationResult},
};
