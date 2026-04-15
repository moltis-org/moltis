//! Configuration loading, validation, env substitution, and legacy migration.
//!
//! Config files: `moltis.toml`, `moltis.yaml`, or `moltis.json`
//! Searched in `./` then `~/.config/moltis/`.
//!
//! Supports `${ENV_VAR}` substitution in all string values.

pub mod agent_defs;
pub mod env_subst;
pub mod error;
pub mod loader;
pub mod migrate;
pub mod provider_env;
pub mod schema;
pub mod template;
pub mod validate;
pub mod version;

pub use version::VERSION;

pub use {
    error::{Error, Result},
    loader::{
        DEFAULT_SOUL, LoadedWorkspaceMarkdown, WorkspaceMarkdownSource, agent_workspace_dir,
        agents_path, apply_env_overrides, boot_path, clear_config_dir, clear_data_dir,
        clear_share_dir, config_dir, data_dir, discover_and_load, extract_yaml_frontmatter,
        find_or_default_config_path, find_user_global_config_file, guidelines_path, heartbeat_path,
        home_dir, identity_path, load_agents_md, load_agents_md_for_agent, load_boot_md,
        load_boot_md_for_agent, load_guidelines_md, load_guidelines_md_for_agent,
        load_heartbeat_md, load_identity, load_identity_for_agent, load_memory_md,
        load_memory_md_for_agent, load_memory_md_for_agent_with_source, load_soul,
        load_soul_for_agent, load_tools_md, load_tools_md_for_agent, load_user, memory_path,
        normalize_workspace_markdown_content, resolve_identity, resolve_identity_from_config,
        resolve_user_profile, resolve_user_profile_from_config, save_config, save_identity,
        save_identity_for_agent, save_raw_config, save_soul, save_soul_for_agent, save_user,
        save_user_with_mode, set_config_dir, set_data_dir, set_share_dir, share_dir, soul_path,
        tools_path, update_config, user_global_config_dir, user_global_config_dir_if_different,
        user_path,
    },
    provider_env::{
        GenericProviderEnv, env_value_with_overrides, generic_provider_api_key_from_env,
        generic_provider_env, generic_provider_env_source_for_provider, normalize_provider_name,
    },
    schema::{
        AgentIdentity, AgentMemoryWriteMode, AgentPreset, AgentsConfig, AuthConfig, CacheRetention,
        CalDavAccountConfig, CalDavConfig, ChannelToolPolicyOverride, ChannelsConfig, ChatConfig,
        CompactionConfig, CompactionMode, GeoLocation, GroupToolPolicy, MemoryBackend,
        MemoryCitationsMode, MemoryProvider, MemoryScope, MemorySearchMergeStrategy, MemoryStyle,
        MessageQueueMode, MoltisConfig, NgrokConfig, PresetMemoryConfig, PresetToolPolicy,
        PromptMemoryMode, ResolvedIdentity, SessionAccessPolicyConfig, SessionExportMode, Timezone,
        ToolMode, ToolPolicyConfig, ToolRegistryMode, UserProfile, UserProfileWriteMode,
        VoiceConfig, VoiceElevenLabsConfig, VoiceOpenAiConfig, VoiceSttConfig, VoiceSttProvider,
        VoiceTtsConfig, VoiceWhisperConfig, WireApi,
    },
    validate::{Diagnostic, Severity, ValidationResult},
};
