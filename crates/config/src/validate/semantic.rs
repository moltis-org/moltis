use {super::*, crate::schema::MoltisConfig, secrecy::ExposeSecret, std::path::Path};

const KNOWN_PROVIDER_NAMES: &[&str] = &[
    "anthropic",
    "openai",
    "gemini",
    "groq",
    "xai",
    "deepseek",
    "fireworks",
    "mistral",
    "openrouter",
    "cerebras",
    "minimax",
    "moonshot",
    "venice",
    "ollama",
    "lmstudio",
];

const PROVIDERS_META_KEYS: &[&str] = &["offered", "show_legacy_models"];

pub(super) fn check_deprecated_fields(
    toml_value: &toml::Value,
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<&'static str> {
    let Some(memory) = toml_value.get("memory").and_then(|value| value.as_table()) else {
        return Vec::new();
    };

    let mut conflicting_replacements = Vec::new();
    if check_deprecated_memory_field(memory, "embedding_provider", "provider", diagnostics) {
        conflicting_replacements.push("provider");
    }
    if check_deprecated_memory_field(memory, "embedding_base_url", "base_url", diagnostics) {
        conflicting_replacements.push("base_url");
    }
    if check_deprecated_memory_field(memory, "embedding_model", "model", diagnostics) {
        conflicting_replacements.push("model");
    }
    if check_deprecated_memory_field(memory, "embedding_api_key", "api_key", diagnostics) {
        conflicting_replacements.push("api_key");
    }
    check_deprecated_ignored_memory_field(
        memory,
        "embedding_dimensions",
        "deprecated field; ignored because embedding dimensions are determined by the provider response",
        diagnostics,
    );
    conflicting_replacements
}

pub(super) fn should_suppress_deprecated_conflict_type_error(
    message: &str,
    conflicting_replacements: &[&str],
) -> bool {
    conflicting_replacements
        .iter()
        .any(|replacement| message.contains(&format!("duplicate field `{replacement}`")))
}

fn check_deprecated_memory_field(
    memory: &toml::map::Map<String, toml::Value>,
    legacy: &str,
    replacement: &str,
    diagnostics: &mut Vec<Diagnostic>,
) -> bool {
    if !memory.contains_key(legacy) {
        return false;
    }

    if memory.contains_key(replacement) {
        diagnostics.push(Diagnostic {
            severity: Severity::Error,
            category: "deprecated-field",
            path: format!("memory.{legacy}"),
            message: format!(
                "deprecated field conflicts with \"memory.{replacement}\"; remove \"memory.{legacy}\""
            ),
        });
        return true;
    }

    diagnostics.push(Diagnostic {
        severity: Severity::Warning,
        category: "deprecated-field",
        path: format!("memory.{legacy}"),
        message: format!("deprecated field; use \"memory.{replacement}\" instead"),
    });
    false
}

fn check_deprecated_ignored_memory_field(
    memory: &toml::map::Map<String, toml::Value>,
    legacy: &str,
    message: &str,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if memory.contains_key(legacy) {
        diagnostics.push(Diagnostic {
            severity: Severity::Warning,
            category: "deprecated-field",
            path: format!("memory.{legacy}"),
            message: message.into(),
        });
    }
}

/// Check provider names under `[providers]` and warn about unknown ones.
pub(super) fn check_provider_names(
    providers: &toml::map::Map<String, toml::Value>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    for name in providers.keys() {
        if PROVIDERS_META_KEYS.contains(&name.as_str()) {
            continue;
        }
        // Custom providers (user-added OpenAI-compatible endpoints) are valid.
        if name.starts_with("custom-") {
            continue;
        }
        if !KNOWN_PROVIDER_NAMES.contains(&name.as_str()) {
            let suggestion = suggest(name, KNOWN_PROVIDER_NAMES, 3);
            let msg = if let Some(s) = suggestion {
                format!("unknown provider name (did you mean \"{s}\"?)")
            } else {
                "unknown provider name (custom providers are valid, but check for typos)".into()
            };
            diagnostics.push(Diagnostic {
                severity: Severity::Warning,
                category: "unknown-provider",
                path: format!("providers.{name}"),
                message: msg,
            });
        }
    }
}

/// Run semantic checks on a successfully parsed config.
pub(super) fn check_semantic_warnings(config: &MoltisConfig, diagnostics: &mut Vec<Diagnostic>) {
    let is_localhost = config.server.bind == "127.0.0.1"
        || config.server.bind == "localhost"
        || config.server.bind == "::1";

    // auth.disabled + non-localhost
    if config.auth.disabled && !is_localhost {
        diagnostics.push(Diagnostic {
            severity: Severity::Warning,
            category: "security",
            path: "auth".into(),
            message: format!(
                "authentication is disabled while binding to {}",
                config.server.bind
            ),
        });
    }

    // TLS disabled + non-localhost
    if !config.tls.enabled && !is_localhost {
        diagnostics.push(Diagnostic {
            severity: Severity::Warning,
            category: "security",
            path: "tls".into(),
            message: format!("TLS is disabled while binding to {}", config.server.bind),
        });
    }

    // TLS cert without key or vice versa
    let has_cert = config.tls.cert_path.is_some();
    let has_key = config.tls.key_path.is_some();
    if has_cert && !has_key {
        diagnostics.push(Diagnostic {
            severity: Severity::Error,
            category: "security",
            path: "tls".into(),
            message: "tls.cert_path is set but tls.key_path is missing".into(),
        });
    }
    if has_key && !has_cert {
        diagnostics.push(Diagnostic {
            severity: Severity::Error,
            category: "security",
            path: "tls".into(),
            message: "tls.key_path is set but tls.cert_path is missing".into(),
        });
    }

    // Sandbox mode off
    if config.tools.exec.sandbox.mode == "off" {
        diagnostics.push(Diagnostic {
            severity: Severity::Warning,
            category: "security",
            path: "tools.exec.sandbox.mode".into(),
            message: "sandbox mode is disabled — commands run without isolation".into(),
        });
    }

    // tools.fs: must_read_before_write requires track_reads
    if config.tools.fs.must_read_before_write && !config.tools.fs.track_reads {
        diagnostics.push(Diagnostic {
            severity: Severity::Error,
            category: "invalid-value",
            path: "tools.fs".into(),
            message: "must_read_before_write=true requires track_reads=true".into(),
        });
    }

    // tools.fs.workspace_root must be absolute when set
    if let Some(ref root) = config.tools.fs.workspace_root
        && !Path::new(root).is_absolute()
    {
        diagnostics.push(Diagnostic {
            severity: Severity::Error,
            category: "invalid-value",
            path: "tools.fs.workspace_root".into(),
            message: format!("workspace_root must be an absolute path (got '{root}')"),
        });
    }

    // tools.fs.allow_paths / deny_paths entries should be absolute
    for (idx, entry) in config.tools.fs.allow_paths.iter().enumerate() {
        if !entry.starts_with('/') && !entry.starts_with("**") {
            diagnostics.push(Diagnostic {
                severity: Severity::Warning,
                category: "invalid-value",
                path: format!("tools.fs.allow_paths[{idx}]"),
                message: format!(
                    "allow_paths entries should be absolute path globs starting with '/' (got '{entry}')"
                ),
            });
        }
    }
    for (idx, entry) in config.tools.fs.deny_paths.iter().enumerate() {
        if !entry.starts_with('/') && !entry.starts_with("**") {
            diagnostics.push(Diagnostic {
                severity: Severity::Warning,
                category: "invalid-value",
                path: format!("tools.fs.deny_paths[{idx}]"),
                message: format!(
                    "deny_paths entries should be absolute path globs starting with '/' (got '{entry}')"
                ),
            });
        }
    }

    // upstream_proxy: must be a valid URL with a supported scheme.
    if let Some(ref proxy) = config.upstream_proxy {
        let url = proxy.expose_secret();
        let valid = url.starts_with("http://")
            || url.starts_with("https://")
            || url.starts_with("socks5://")
            || url.starts_with("socks5h://");
        if !valid {
            // Extract only the scheme portion (before "://") to avoid leaking
            // credentials that may be embedded in the URL.
            let scheme = url.split("://").next().unwrap_or("<unknown>");
            diagnostics.push(Diagnostic {
                severity: Severity::Error,
                category: "invalid-value",
                path: "upstream_proxy".into(),
                message: format!(
                    "upstream_proxy must use http://, https://, socks5://, or socks5h:// scheme (got \"{scheme}://\")"
                ),
            });
        }
    }

    // Loop limit must be positive to avoid immediate run failures.
    if config.tools.agent_max_iterations == 0 {
        diagnostics.push(Diagnostic {
            severity: Severity::Error,
            category: "invalid-value",
            path: "tools.agent_max_iterations".into(),
            message: "tools.agent_max_iterations must be at least 1".into(),
        });
    }

    // A zero workspace file limit would silently drop all AGENTS.md / TOOLS.md content.
    if config.chat.workspace_file_max_chars == 0 {
        diagnostics.push(Diagnostic {
            severity: Severity::Warning,
            category: "invalid-value",
            path: "chat.workspace_file_max_chars".into(),
            message: "chat.workspace_file_max_chars is 0 — AGENTS.md and TOOLS.md will be empty in the prompt".into(),
        });
    }

    // Compaction config sanity.
    let compaction = &config.chat.compaction;
    if !(0.1..=0.95).contains(&compaction.threshold_percent) {
        diagnostics.push(Diagnostic {
            severity: Severity::Warning,
            category: "invalid-value",
            path: "chat.compaction.threshold_percent".into(),
            message: format!(
                "chat.compaction.threshold_percent = {} is outside the supported 0.1–0.95 range; the default (0.95) will be used",
                compaction.threshold_percent
            ),
        });
    }
    if !(0.05..=0.80).contains(&compaction.tail_budget_ratio) {
        diagnostics.push(Diagnostic {
            severity: Severity::Warning,
            category: "invalid-value",
            path: "chat.compaction.tail_budget_ratio".into(),
            message: format!(
                "chat.compaction.tail_budget_ratio = {} is outside the supported 0.05–0.80 range; the default (0.20) will be used",
                compaction.tail_budget_ratio
            ),
        });
    }
    if compaction.protect_head > 32 {
        diagnostics.push(Diagnostic {
            severity: Severity::Warning,
            category: "invalid-value",
            path: "chat.compaction.protect_head".into(),
            message: "chat.compaction.protect_head > 32 will leave little room for the compacted middle region on typical sessions".into(),
        });
    }
    // All four CompactionMode variants are now implemented. Any future
    // "not-implemented" markers would go here; leave the match explicit so
    // adding a new variant forces a decision at compile time.
    match compaction.mode {
        crate::schema::CompactionMode::Deterministic
        | crate::schema::CompactionMode::RecencyPreserving
        | crate::schema::CompactionMode::Structured
        | crate::schema::CompactionMode::LlmReplace => {},
    }

    if config.mcp.request_timeout_secs == 0 {
        diagnostics.push(Diagnostic {
            severity: Severity::Error,
            category: "invalid-value",
            path: "mcp.request_timeout_secs".into(),
            message: "mcp.request_timeout_secs must be at least 1".into(),
        });
    }

    for (name, server) in &config.mcp.servers {
        if server.request_timeout_secs == Some(0) {
            diagnostics.push(Diagnostic {
                severity: Severity::Error,
                category: "invalid-value",
                path: format!("mcp.servers.{name}.request_timeout_secs"),
                message: "mcp server request_timeout_secs must be at least 1".into(),
            });
        }

        if !server.transport.is_empty()
            && !matches!(
                server.transport.as_str(),
                "stdio" | "sse" | "streamable-http" | "streamable_http" | "http"
            )
        {
            diagnostics.push(Diagnostic {
                severity: Severity::Warning,
                category: "invalid-value",
                path: format!("mcp.servers.{name}.transport"),
                message: format!(
                    "unknown transport type \"{}\"; expected \"stdio\", \"sse\", or \"streamable-http\"",
                    server.transport
                ),
            });
        }
    }

    // Firecrawl as search provider requires an API key.  We cannot check
    // the FIRECRAWL_API_KEY env var here (static validation), so only emit
    // an Info-level hint when neither config path supplies a key.
    if config.tools.web.search.provider == crate::schema::SearchProvider::Firecrawl
        && config.tools.web.firecrawl.api_key.is_none()
        && config.tools.web.search.api_key.is_none()
        && !config.tools.web.search.duckduckgo_fallback
    {
        diagnostics.push(Diagnostic {
            severity: Severity::Info,
            category: "unknown-provider",
            path: "tools.web.search.provider".into(),
            message: "search provider is 'firecrawl' but no API key found in config \
                      (may be supplied at runtime via FIRECRAWL_API_KEY env var)"
                .into(),
        });
    }

    // agents.default_preset should reference an existing preset key.
    if let Some(default_preset) = config.agents.default_preset.as_deref()
        && !config.agents.presets.contains_key(default_preset)
    {
        diagnostics.push(Diagnostic {
            severity: Severity::Warning,
            category: "unknown-field",
            path: "agents.default_preset".into(),
            message: format!(
                "default preset \"{default_preset}\" is not defined in agents.presets"
            ),
        });
    }

    // Silent-misconfiguration trap: `[agents.presets.*]` tool policies apply
    // ONLY to sub-agents spawned via `spawn_agent`. They do NOT filter tools
    // for the main agent session — that is controlled exclusively by
    // `[tools.policy]`. Users hardening their deployment often put a deny
    // list under a preset and expect it to apply to the main session; it
    // silently doesn't. Warn when at least one preset declares
    // `tools.allow`/`tools.deny` while `[tools.policy]` is entirely empty.
    {
        let main_policy_empty = config.tools.policy.allow.is_empty()
            && config.tools.policy.deny.is_empty()
            && config.tools.policy.profile.is_none();
        if main_policy_empty {
            let mut offending: Vec<&str> = config
                .agents
                .presets
                .iter()
                .filter(|(_, preset)| {
                    !preset.tools.allow.is_empty() || !preset.tools.deny.is_empty()
                })
                .map(|(name, _)| name.as_str())
                .collect();
            if !offending.is_empty() {
                offending.sort_unstable();
                let quoted: Vec<String> =
                    offending.iter().map(|name| format!("\"{name}\"")).collect();
                diagnostics.push(Diagnostic {
                    severity: Severity::Warning,
                    category: "security",
                    path: "agents.presets".into(),
                    message: format!(
                        "preset(s) [{}] declare tools.allow/tools.deny, but \
                         [tools.policy] is empty. Preset tool policies apply \
                         ONLY to sub-agents spawned via the spawn_agent tool; \
                         they do NOT filter tools for the main agent session. \
                         To allow/deny tools for the main session, set \
                         tools.policy.allow, tools.policy.deny, or \
                         tools.policy.profile.",
                        quoted.join(", ")
                    ),
                });
            }
        }
    }

    // agents.presets.*.reasoning_effort is now a typed enum (ReasoningEffort)
    // and validated at deserialization time (step 4). No semantic check needed.

    // SSRF allowlist CIDR validation
    for (idx, entry) in config.tools.web.fetch.ssrf_allowlist.iter().enumerate() {
        if entry.parse::<ipnet::IpNet>().is_err() {
            diagnostics.push(Diagnostic {
                severity: Severity::Error,
                category: "security",
                path: format!("tools.web.fetch.ssrf_allowlist[{idx}]"),
                message: format!(
                    "\"{entry}\" is not a valid CIDR range (expected e.g. \"172.22.0.0/16\")"
                ),
            });
        }
    }
    if !config.tools.web.fetch.ssrf_allowlist.is_empty() {
        diagnostics.push(Diagnostic {
            severity: Severity::Warning,
            category: "security",
            path: "tools.web.fetch.ssrf_allowlist".into(),
            message: "ssrf_allowlist is set — SSRF protection is relaxed for the listed ranges. Ensure these are trusted networks.".into(),
        });
    }

    // Unknown tool_mode values on provider entries
    // Note: serde rejects truly invalid values at deserialization, but if a
    // provider entry somehow comes through with a non-standard string we still
    // want to warn at the TOML level.  The enum is auto/native/text/off.

    // Unknown channel types in channels.offered — accept built-in types plus
    // any dynamically configured types from `[channels.<type>]` sections.
    let mut valid_channel_types: Vec<&str> = crate::schema::KNOWN_CHANNEL_TYPES.to_vec();
    for ct in config.channels.extra.keys() {
        valid_channel_types.push(ct.as_str());
    }
    for (idx, entry) in config.channels.offered.iter().enumerate() {
        if !valid_channel_types.contains(&entry.as_str()) {
            diagnostics.push(Diagnostic {
                severity: Severity::Warning,
                category: "unknown-field",
                path: format!("channels.offered[{idx}]"),
                message: format!(
                    "unknown channel type \"{entry}\"; expected one of: {}",
                    valid_channel_types.join(", ")
                ),
            });
        }
    }

    // Unknown tailscale mode
    let valid_ts_modes = ["off", "serve", "funnel"];
    if !valid_ts_modes.contains(&config.tailscale.mode.as_str()) {
        diagnostics.push(Diagnostic {
            severity: Severity::Warning,
            category: "unknown-field",
            path: "tailscale.mode".into(),
            message: format!(
                "unknown tailscale mode \"{}\"; expected one of: {}",
                config.tailscale.mode,
                valid_ts_modes.join(", ")
            ),
        });
    }

    if config.ngrok.enabled && config.auth.disabled {
        diagnostics.push(Diagnostic {
            severity: Severity::Warning,
            category: "security",
            path: "ngrok.enabled".into(),
            message: "ngrok is enabled while auth.disabled is true; remote visitors will be blocked with setup required until authentication is configured".into(),
        });
    }

    // Unknown sandbox backend
    let valid_sandbox_backends = [
        "auto",
        "docker",
        "podman",
        "apple-container",
        "restricted-host",
        "wasm",
    ];
    if !valid_sandbox_backends.contains(&config.tools.exec.sandbox.backend.as_str()) {
        diagnostics.push(Diagnostic {
            severity: Severity::Warning,
            category: "unknown-field",
            path: "tools.exec.sandbox.backend".into(),
            message: format!(
                "unknown sandbox backend \"{}\"; expected one of: {}",
                config.tools.exec.sandbox.backend,
                valid_sandbox_backends.join(", ")
            ),
        });
    }

    // Unknown sandbox network policy
    if !config.tools.exec.sandbox.network.is_empty() {
        let valid_network_policies = ["blocked", "trusted", "bypass"];
        if !valid_network_policies.contains(&config.tools.exec.sandbox.network.as_str()) {
            diagnostics.push(Diagnostic {
                severity: Severity::Warning,
                category: "unknown-field",
                path: "tools.exec.sandbox.network".into(),
                message: format!(
                    "unknown sandbox network policy \"{}\"; expected one of: {}",
                    config.tools.exec.sandbox.network,
                    valid_network_policies.join(", ")
                ),
            });
        }
    }

    // Unknown CalDAV provider
    let valid_caldav_providers = ["fastmail", "icloud", "generic"];
    for (name, account) in &config.caldav.accounts {
        if let Some(ref provider) = account.provider
            && !valid_caldav_providers.contains(&provider.as_str())
        {
            diagnostics.push(Diagnostic {
                severity: Severity::Warning,
                category: "unknown-field",
                path: format!("caldav.accounts.{name}.provider"),
                message: format!(
                    "unknown CalDAV provider \"{provider}\"; expected one of: {}",
                    valid_caldav_providers.join(", ")
                ),
            });
        }
    }

    // Unknown exec host
    let valid_exec_hosts = ["local", "node", "ssh"];
    if !valid_exec_hosts.contains(&config.tools.exec.host.as_str()) {
        diagnostics.push(Diagnostic {
            severity: Severity::Warning,
            category: "unknown-field",
            path: "tools.exec.host".into(),
            message: format!(
                "unknown exec host \"{}\"; expected one of: {}",
                config.tools.exec.host,
                valid_exec_hosts.join(", ")
            ),
        });
    }

    // Warn if host=node but no node specified
    if config.tools.exec.host == "node" && config.tools.exec.node.is_none() {
        diagnostics.push(Diagnostic {
            severity: Severity::Warning,
            category: "unknown-field",
            path: "tools.exec.node".into(),
            message: "tools.exec.host is \"node\" but no default node is specified; commands will fail unless a node connects".into(),
        });
    }

    if config.tools.exec.host == "ssh" && config.tools.exec.ssh_target.is_none() {
        diagnostics.push(Diagnostic {
            severity: Severity::Warning,
            category: "unknown-field",
            path: "tools.exec.ssh_target".into(),
            message:
                "tools.exec.host is \"ssh\" but no SSH target is specified; commands will fail"
                    .into(),
        });
    }

    // Unknown exec security level
    let valid_security_levels = ["allowlist", "permissive", "strict"];
    if !valid_security_levels.contains(&config.tools.exec.security_level.as_str()) {
        diagnostics.push(Diagnostic {
            severity: Severity::Warning,
            category: "unknown-field",
            path: "tools.exec.security_level".into(),
            message: format!(
                "unknown security level \"{}\"; expected one of: {}",
                config.tools.exec.security_level,
                valid_security_levels.join(", ")
            ),
        });
    }

    // Unknown voice TTS providers list values
    let valid_voice_tts_providers = [
        "elevenlabs",
        "openai",
        "openai-tts",
        "google",
        "google-tts",
        "piper",
        "coqui",
    ];
    for (idx, provider) in config.voice.tts.providers.iter().enumerate() {
        if !valid_voice_tts_providers.contains(&provider.as_str()) {
            diagnostics.push(Diagnostic {
                severity: Severity::Warning,
                category: "unknown-field",
                path: format!("voice.tts.providers[{idx}]"),
                message: format!(
                    "unknown TTS provider \"{provider}\"; expected one of: {}",
                    valid_voice_tts_providers.join(", ")
                ),
            });
        }
    }

    // Unknown voice STT providers list values
    let valid_voice_stt_providers = [
        "whisper",
        "groq",
        "deepgram",
        "google",
        "mistral",
        "elevenlabs",
        "elevenlabs-stt",
        "voxtral-local",
        "whisper-cli",
        "sherpa-onnx",
    ];
    for (idx, provider) in config.voice.stt.providers.iter().enumerate() {
        if !valid_voice_stt_providers.contains(&provider.as_str()) {
            diagnostics.push(Diagnostic {
                severity: Severity::Warning,
                category: "unknown-field",
                path: format!("voice.stt.providers[{idx}]"),
                message: format!(
                    "unknown STT provider \"{provider}\"; expected one of: {}",
                    valid_voice_stt_providers.join(", ")
                ),
            });
        }
    }

    // Unknown hook event names
    let valid_hook_events = [
        "BeforeAgentStart",
        "AgentEnd",
        "BeforeLLMCall",
        "AfterLLMCall",
        "BeforeCompaction",
        "AfterCompaction",
        "MessageReceived",
        "MessageSending",
        "MessageSent",
        "BeforeToolCall",
        "AfterToolCall",
        "ToolResultPersist",
        "SessionStart",
        "SessionEnd",
        "GatewayStart",
        "GatewayStop",
        "Command",
    ];
    if let Some(ref hooks_config) = config.hooks {
        for (hook_idx, hook) in hooks_config.hooks.iter().enumerate() {
            for (ev_idx, event) in hook.events.iter().enumerate() {
                if !valid_hook_events.contains(&event.as_str()) {
                    diagnostics.push(Diagnostic {
                        severity: Severity::Warning,
                        category: "unknown-field",
                        path: format!("hooks.hooks[{hook_idx}].events[{ev_idx}]"),
                        message: format!(
                            "unknown hook event \"{event}\"; expected one of: {}",
                            valid_hook_events.join(", ")
                        ),
                    });
                }
            }
        }
    }

    // Browser profile_dir should be an absolute path
    if let Some(ref dir) = config.tools.browser.profile_dir
        && !Path::new(dir).is_absolute()
    {
        diagnostics.push(Diagnostic {
            severity: Severity::Warning,
            category: "invalid-value",
            path: "tools.browser.profile_dir".into(),
            message: "profile_dir should be an absolute path".into(),
        });
    }

    // port == 0
    if config.server.port == 0 {
        diagnostics.push(Diagnostic {
            severity: Severity::Info,
            category: "security",
            path: "server.port".into(),
            message: "port is 0; a random port will be assigned at startup".into(),
        });
    }

    // Validate global model overrides.
    // Negative values are rejected by u32 deserialization (type error),
    // so we only need to guard zero and unusually large values here.
    for (model_id, override_cfg) in &config.models {
        validate_context_window(
            override_cfg.context_window,
            &format!("models.{model_id}.context_window"),
            diagnostics,
        );
    }

    // Validate provider-scoped model overrides.
    for (provider_name, provider_entry) in &config.providers.providers {
        for (model_id, override_cfg) in &provider_entry.model_overrides {
            validate_context_window(
                override_cfg.context_window,
                &format!("providers.{provider_name}.model_overrides.{model_id}.context_window"),
                diagnostics,
            );
        }
    }

    // tools: overflow_ratio must not be zero (budget becomes 0, every iteration fails)
    if config.tools.preemptive_overflow_ratio == 0 {
        diagnostics.push(Diagnostic {
            severity: Severity::Warning,
            category: "invalid-value",
            path: "tools.preemptive_overflow_ratio".into(),
            message: "preemptive_overflow_ratio = 0 means the overflow budget is always \
                      0 tokens; the agent loop will fail immediately on every iteration"
                .into(),
        });
    }

    // tools: ratio fields should not exceed 100 (percentages)
    if config.tools.tool_result_compaction_ratio > 100 {
        diagnostics.push(Diagnostic {
            severity: Severity::Warning,
            category: "invalid-value",
            path: "tools.tool_result_compaction_ratio".into(),
            message: format!(
                "tool_result_compaction_ratio ({}) exceeds 100 — compaction will trigger on every iteration regardless of context usage",
                config.tools.tool_result_compaction_ratio
            ),
        });
    }
    if config.tools.preemptive_overflow_ratio > 100 {
        diagnostics.push(Diagnostic {
            severity: Severity::Warning,
            category: "invalid-value",
            path: "tools.preemptive_overflow_ratio".into(),
            message: format!(
                "preemptive_overflow_ratio ({}) exceeds 100 — overflow protection will always trigger",
                config.tools.preemptive_overflow_ratio
            ),
        });
    }

    // tools: overflow_ratio must be greater than compaction_ratio
    if config.tools.tool_result_compaction_ratio > 0
        && config.tools.preemptive_overflow_ratio <= config.tools.tool_result_compaction_ratio
    {
        diagnostics.push(Diagnostic {
            severity: Severity::Warning,
            category: "invalid-value",
            path: "tools.preemptive_overflow_ratio".into(),
            message: format!(
                "preemptive_overflow_ratio ({}) should be greater than tool_result_compaction_ratio ({}) to avoid context overflow on every iteration",
                config.tools.preemptive_overflow_ratio, config.tools.tool_result_compaction_ratio
            ),
        });
    }
}

/// Validate a `context_window` override value (optional field).
fn validate_context_window(value: Option<u32>, path: &str, diagnostics: &mut Vec<Diagnostic>) {
    let Some(cw) = value else {
        return;
    };
    if cw == 0 {
        diagnostics.push(Diagnostic {
            severity: Severity::Error,
            category: "invalid-value",
            path: path.into(),
            message: "context_window must be at least 1".into(),
        });
    } else if cw > 10_000_000 {
        diagnostics.push(Diagnostic {
            severity: Severity::Warning,
            category: "invalid-value",
            path: path.into(),
            message: format!("context_window is {cw}, which is unusually large (> 10M)"),
        });
    }
}

/// Check that file paths referenced in TLS config exist on disk.
pub(super) fn check_file_references(
    toml_str: &str,
    _config_path: &Path,
    diagnostics: &mut Vec<Diagnostic>,
) {
    // Only check if we can parse the config
    let Ok(config) = toml::from_str::<MoltisConfig>(toml_str) else {
        return;
    };

    let file_refs: &[(&str, &Option<String>)] = &[
        ("tls.cert_path", &config.tls.cert_path),
        ("tls.key_path", &config.tls.key_path),
        ("tls.ca_cert_path", &config.tls.ca_cert_path),
    ];

    for (path_name, value) in file_refs {
        if let Some(file_path) = value {
            let p = Path::new(file_path);
            if !p.exists() {
                diagnostics.push(Diagnostic {
                    severity: Severity::Warning,
                    category: "file-ref",
                    path: (*path_name).into(),
                    message: format!("file not found: {file_path}"),
                });
            }
        }
    }
}
