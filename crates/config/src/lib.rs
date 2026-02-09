//! Configuration loading, validation, env substitution, and legacy migration.
//!
//! Config files: `moltis.toml`, `moltis.yaml`, or `moltis.json`
//! Searched in `./` then `~/.config/moltis/`.
//!
//! Supports `${ENV_VAR}` substitution in all string values.

pub mod env_subst;
pub mod loader;
pub mod migrate;
pub mod schema;
pub mod template;
pub mod validate;

pub use {
    loader::{
        agents_path, apply_env_overrides, clear_config_dir, clear_data_dir, config_dir, data_dir,
        discover_and_load, find_or_default_config_path, find_user_global_config_file,
        heartbeat_path, identity_path, load_agents_md, load_heartbeat_md, load_identity, load_soul,
        load_tools_md, load_user, save_config, save_identity, save_soul, save_user, set_config_dir,
        set_data_dir, soul_path, tools_path, update_config, user_global_config_dir,
        user_global_config_dir_if_different, user_path,
    },
    schema::{
        AgentIdentity, AuthConfig, ChatConfig, GeoLocation, MessageQueueMode, MoltisConfig,
        ResolvedIdentity, Timezone, UserProfile, VoiceConfig, VoiceElevenLabsConfig,
        VoiceOpenAiConfig, VoiceSttConfig, VoiceSttProvider, VoiceTtsConfig, VoiceWhisperConfig,
    },
    validate::{Diagnostic, Severity, ValidationResult},
};
