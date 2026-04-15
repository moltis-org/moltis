use {
    serde::{Deserialize, Serialize},
    tracing::debug,
};

/// Glob-based allow/deny policy for tool access.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ToolPolicy {
    #[serde(default)]
    pub allow: Vec<String>,
    #[serde(default)]
    pub deny: Vec<String>,
}

/// Context for resolving which policy layers apply.
#[derive(Debug, Clone, Default)]
pub struct PolicyContext {
    pub agent_id: String,
    pub provider: Option<String>,
    pub channel: Option<String>,
    pub channel_account_id: Option<String>,
    pub group_id: Option<String>,
    pub sender_id: Option<String>,
    pub sandboxed: bool,
}

/// Predefined tool profiles.
pub fn profile_tools(profile: &str) -> ToolPolicy {
    match profile {
        "minimal" => ToolPolicy {
            allow: vec!["exec".into()],
            deny: Vec::new(),
        },
        "coding" => ToolPolicy {
            allow: vec!["exec".into(), "browser".into(), "memory".into()],
            deny: Vec::new(),
        },
        "full" => ToolPolicy {
            allow: vec!["*".into()],
            deny: Vec::new(),
        },
        _ => ToolPolicy::default(),
    }
}

/// Check if a tool name matches a glob pattern (supports `*` wildcard).
fn pattern_matches(pattern: &str, name: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    if let Some(prefix) = pattern.strip_suffix('*') {
        return name.starts_with(prefix);
    }
    pattern == name
}

impl ToolPolicy {
    /// Returns true if the given tool name is allowed by this policy.
    /// Deny always wins over allow.
    pub fn is_allowed(&self, tool_name: &str) -> bool {
        // Check deny first — deny wins.
        for pattern in &self.deny {
            if pattern_matches(pattern, tool_name) {
                return false;
            }
        }
        // If allow is empty, everything not denied is allowed.
        if self.allow.is_empty() {
            return true;
        }
        // Otherwise, must match an allow pattern.
        for pattern in &self.allow {
            if pattern_matches(pattern, tool_name) {
                return true;
            }
        }
        false
    }

    /// Merge another policy on top of this one (the `other` has higher precedence).
    /// Non-empty lists from `other` replace those from `self`.
    pub fn merge_with(&self, other: &ToolPolicy) -> ToolPolicy {
        ToolPolicy {
            allow: if other.allow.is_empty() {
                self.allow.clone()
            } else {
                other.allow.clone()
            },
            deny: {
                let mut combined = self.deny.clone();
                combined.extend(other.deny.iter().cloned());
                combined
            },
        }
    }
}

/// Build a `ToolPolicy` from a `ToolPolicyConfig`, expanding the optional
/// `profile` field before applying explicit allow/deny.
fn policy_from_config(cfg: &moltis_config::schema::ToolPolicyConfig) -> ToolPolicy {
    let mut p = if let Some(profile) = cfg.profile.as_deref()
        && !profile.is_empty()
    {
        profile_tools(profile)
    } else {
        ToolPolicy::default()
    };
    let explicit = ToolPolicy {
        allow: cfg.allow.clone(),
        deny: cfg.deny.clone(),
    };
    if !explicit.allow.is_empty() || !explicit.deny.is_empty() {
        p = p.merge_with(&explicit);
    }
    p
}

/// Resolve the effective tool policy by merging layers from typed config.
///
/// Layer precedence (later wins for allow, deny always accumulates):
/// 1. Global — `[tools.policy]`
/// 2. Per-provider — `[providers.<name>.policy]`
/// 3. Per-agent preset — `[agents.presets.<agent_id>.tools]`
/// 4. Per-channel-group — `[channels.<type>.<account>.tools.groups.<chat_type>]`
/// 5. Per-sender in group — `[channels.<type>.<account>.tools.groups.<chat_type>.by_sender.<id>]`
/// 6. Sandbox overrides — `[tools.exec.sandbox.tools_policy]` (only when `context.sandboxed`)
pub fn resolve_effective_policy(
    config: &moltis_config::MoltisConfig,
    context: &PolicyContext,
) -> ToolPolicy {
    let mut effective = ToolPolicy::default();

    // Layer 1: Global — [tools.policy] (profile + allow/deny)
    if let Some(profile) = config.tools.policy.profile.as_deref()
        && !profile.is_empty()
    {
        effective = effective.merge_with(&profile_tools(profile));
        debug!("policy: applied global profile '{profile}'");
    }
    let global = ToolPolicy {
        allow: config.tools.policy.allow.clone(),
        deny: config.tools.policy.deny.clone(),
    };
    if !global.allow.is_empty() || !global.deny.is_empty() {
        effective = effective.merge_with(&global);
        debug!("policy: applied global layer");
    }

    // Layer 2: Per-provider — [providers.<name>.policy]
    if let Some(ref provider_name) = context.provider
        && let Some(entry) = config.providers.providers.get(provider_name)
        && let Some(ref provider_policy) = entry.policy
    {
        let p = policy_from_config(provider_policy);
        if !p.allow.is_empty() || !p.deny.is_empty() {
            effective = effective.merge_with(&p);
            debug!(provider = provider_name, "policy: applied provider layer");
        }
    }

    // Layer 3: Per-agent preset — [agents.presets.<agent_id>.tools]
    if let Some(preset) = config.agents.get_preset(&context.agent_id) {
        let p = ToolPolicy {
            allow: preset.tools.allow.clone(),
            deny: preset.tools.deny.clone(),
        };
        if !p.allow.is_empty() || !p.deny.is_empty() {
            effective = effective.merge_with(&p);
            debug!(agent_id = %context.agent_id, "policy: applied agent preset layer");
        }
    }

    // Layer 4: Per-channel-group — [channels.<type>.<account>.tools.groups.<chat_type>]
    if let Some(ref channel) = context.channel
        && let Some(ref account_id) = context.channel_account_id
        && let Some(ref group_id) = context.group_id
        && let Some(override_config) = config.channels.tool_policy_for_account(channel, account_id)
        && let Some(group_policy) = override_config.groups.get(group_id.as_str())
    {
        let p = ToolPolicy {
            allow: group_policy.allow.clone(),
            deny: group_policy.deny.clone(),
        };
        if !p.allow.is_empty() || !p.deny.is_empty() {
            effective = effective.merge_with(&p);
            debug!(channel, account_id, group_id, "policy: applied group layer");
        }

        // Layer 5: Per-sender — [...groups.<chat_type>.by_sender.<sender_id>]
        if let Some(ref sender_id) = context.sender_id
            && let Some(sender_policy) = group_policy.by_sender.get(sender_id.as_str())
        {
            let p = policy_from_config(sender_policy);
            if !p.allow.is_empty() || !p.deny.is_empty() {
                effective = effective.merge_with(&p);
                debug!(channel, group_id, sender_id, "policy: applied sender layer");
            }
        }
    }

    // Layer 6: Sandbox overrides — [tools.exec.sandbox.tools_policy]
    if context.sandboxed
        && let Some(ref sandbox_policy) = config.tools.exec.sandbox.tools_policy
    {
        let p = policy_from_config(sandbox_policy);
        if !p.allow.is_empty() || !p.deny.is_empty() {
            effective = effective.merge_with(&p);
            debug!("policy: applied sandbox layer");
        }
    }

    effective
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_allow_all() {
        let policy = ToolPolicy {
            allow: vec!["*".into()],
            deny: Vec::new(),
        };
        assert!(policy.is_allowed("exec"));
        assert!(policy.is_allowed("browser"));
    }

    #[test]
    fn test_deny_wins() {
        let policy = ToolPolicy {
            allow: vec!["*".into()],
            deny: vec!["exec".into()],
        };
        assert!(!policy.is_allowed("exec"));
        assert!(policy.is_allowed("browser"));
    }

    #[test]
    fn test_prefix_pattern() {
        let policy = ToolPolicy {
            allow: vec!["browser*".into()],
            deny: Vec::new(),
        };
        assert!(policy.is_allowed("browser"));
        assert!(policy.is_allowed("browser.fetch"));
        assert!(!policy.is_allowed("exec"));
    }

    #[test]
    fn test_empty_allow_permits_all() {
        let policy = ToolPolicy {
            allow: Vec::new(),
            deny: Vec::new(),
        };
        assert!(policy.is_allowed("exec"));
    }

    #[test]
    fn test_merge_deny_accumulates() {
        let base = ToolPolicy {
            allow: vec!["*".into()],
            deny: vec!["dangerous".into()],
        };
        let overlay = ToolPolicy {
            allow: Vec::new(),
            deny: vec!["exec".into()],
        };
        let merged = base.merge_with(&overlay);
        assert!(!merged.is_allowed("dangerous"));
        assert!(!merged.is_allowed("exec"));
        assert!(merged.is_allowed("browser"));
    }

    #[test]
    fn test_profiles() {
        let minimal = profile_tools("minimal");
        assert!(minimal.is_allowed("exec"));
        assert!(!minimal.is_allowed("browser"));

        let coding = profile_tools("coding");
        assert!(coding.is_allowed("exec"));
        assert!(!coding.is_allowed("Read"));
        assert!(!coding.is_allowed("Write"));

        let full = profile_tools("full");
        assert!(full.is_allowed("anything"));
    }

    #[test]
    fn test_resolve_global_policy() {
        let mut cfg = moltis_config::MoltisConfig::default();
        cfg.tools.policy.allow = vec!["exec".into()];
        cfg.tools.policy.deny = vec!["dangerous".into()];

        let ctx = PolicyContext {
            agent_id: "test".into(),
            ..Default::default()
        };
        let policy = resolve_effective_policy(&cfg, &ctx);
        assert!(policy.is_allowed("exec"));
        assert!(!policy.is_allowed("dangerous"));
        assert!(!policy.is_allowed("browser"));
    }

    #[test]
    fn test_resolve_provider_deny_wins_across_layers() {
        let mut cfg = moltis_config::MoltisConfig::default();
        cfg.tools.policy.allow = vec!["*".into()];

        cfg.providers
            .providers
            .insert("openai".into(), moltis_config::schema::ProviderEntry {
                policy: Some(moltis_config::schema::ToolPolicyConfig {
                    allow: Vec::new(),
                    deny: vec!["exec".into()],
                    profile: None,
                }),
                ..Default::default()
            });

        let ctx = PolicyContext {
            agent_id: "test".into(),
            provider: Some("openai".into()),
            ..Default::default()
        };
        let policy = resolve_effective_policy(&cfg, &ctx);
        assert!(!policy.is_allowed("exec"));
        assert!(policy.is_allowed("browser"));
    }

    #[test]
    fn test_resolve_agent_preset_layer() {
        let mut cfg = moltis_config::MoltisConfig::default();
        cfg.tools.policy.allow = vec!["*".into()];
        cfg.agents
            .presets
            .insert("researcher".into(), moltis_config::schema::AgentPreset {
                tools: moltis_config::schema::PresetToolPolicy {
                    allow: vec!["web_search".into(), "web_fetch".into()],
                    deny: Vec::new(),
                },
                ..Default::default()
            });

        let ctx = PolicyContext {
            agent_id: "researcher".into(),
            ..Default::default()
        };
        let policy = resolve_effective_policy(&cfg, &ctx);
        assert!(policy.is_allowed("web_search"));
        assert!(policy.is_allowed("web_fetch"));
        assert!(!policy.is_allowed("exec"));
    }

    #[test]
    fn test_resolve_channel_group_policy() {
        let mut cfg = moltis_config::MoltisConfig::default();
        cfg.tools.policy.allow = vec!["*".into()];

        // Set up channel config with tool policy.
        let account_config = serde_json::json!({
            "token": "bot-token",
            "tools": {
                "groups": {
                    "group": {
                        "deny": ["exec", "browser"],
                        "by_sender": {
                            "trusted_user_123": {
                                "allow": ["*"],
                                "deny": []
                            }
                        }
                    }
                }
            }
        });
        cfg.channels
            .telegram
            .insert("my-bot".into(), account_config);

        // Group chat, untrusted sender — exec should be denied.
        let ctx = PolicyContext {
            agent_id: "main".into(),
            channel: Some("telegram".into()),
            channel_account_id: Some("my-bot".into()),
            group_id: Some("group".into()),
            sender_id: Some("random_user".into()),
            ..Default::default()
        };
        let policy = resolve_effective_policy(&cfg, &ctx);
        assert!(!policy.is_allowed("exec"));
        assert!(!policy.is_allowed("browser"));
        assert!(policy.is_allowed("web_search"));

        // Trusted sender — exec allowed via by_sender override.
        let ctx_trusted = PolicyContext {
            sender_id: Some("trusted_user_123".into()),
            ..ctx.clone()
        };
        let policy_trusted = resolve_effective_policy(&cfg, &ctx_trusted);
        // Deny from group layer accumulates, but sender layer replaces allow.
        // Deny always wins: group denied exec+browser, sender can't override denies.
        assert!(!policy_trusted.is_allowed("exec"));
        assert!(!policy_trusted.is_allowed("browser"));
        // But a new allow pattern from sender layer works for non-denied tools.
        assert!(policy_trusted.is_allowed("web_search"));
    }

    #[test]
    fn test_resolve_channel_group_sender_override_allow() {
        let mut cfg = moltis_config::MoltisConfig::default();
        // Restrict global to web_search only.
        cfg.tools.policy.allow = vec!["web_search".into()];

        let account_config = serde_json::json!({
            "tools": {
                "groups": {
                    "private": {
                        "by_sender": {
                            "admin_42": {
                                "allow": ["*"]
                            }
                        }
                    }
                }
            }
        });
        cfg.channels.telegram.insert("bot1".into(), account_config);

        // DM from admin — sender layer gives allow = ["*"].
        let ctx = PolicyContext {
            agent_id: "main".into(),
            channel: Some("telegram".into()),
            channel_account_id: Some("bot1".into()),
            group_id: Some("private".into()),
            sender_id: Some("admin_42".into()),
            ..Default::default()
        };
        let policy = resolve_effective_policy(&cfg, &ctx);
        assert!(policy.is_allowed("exec"));
        assert!(policy.is_allowed("browser"));
        assert!(policy.is_allowed("web_search"));
    }

    #[test]
    fn test_no_channel_context_skips_layers_4_5() {
        let mut cfg = moltis_config::MoltisConfig::default();
        cfg.tools.policy.allow = vec!["*".into()];

        // Even with channel config, no channel in context means layers 4-5 skipped.
        let account_config = serde_json::json!({
            "tools": {
                "groups": {
                    "group": {
                        "deny": ["exec"]
                    }
                }
            }
        });
        cfg.channels.telegram.insert("bot1".into(), account_config);

        let ctx = PolicyContext {
            agent_id: "main".into(),
            // No channel context (e.g. web UI session).
            ..Default::default()
        };
        let policy = resolve_effective_policy(&cfg, &ctx);
        assert!(policy.is_allowed("exec")); // Not denied — channel layers skipped.
    }

    #[test]
    fn test_resolve_sandbox_layer_overrides() {
        let mut cfg = moltis_config::MoltisConfig::default();
        cfg.tools.policy.allow = vec!["*".into()];

        // Configure sandbox-specific tool policy that denies browser.
        cfg.tools.exec.sandbox.tools_policy = Some(moltis_config::schema::ToolPolicyConfig {
            allow: vec!["exec".into()],
            deny: vec!["browser".into()],
            profile: None,
        });

        // Without sandboxed flag — layer 6 is skipped.
        let ctx = PolicyContext {
            agent_id: "main".into(),
            sandboxed: false,
            ..Default::default()
        };
        let policy = resolve_effective_policy(&cfg, &ctx);
        assert!(policy.is_allowed("exec"));
        assert!(policy.is_allowed("browser")); // Not denied — sandbox layer skipped.

        // With sandboxed flag — layer 6 applies.
        let ctx_sandboxed = PolicyContext {
            agent_id: "main".into(),
            sandboxed: true,
            ..Default::default()
        };
        let policy_sandboxed = resolve_effective_policy(&cfg, &ctx_sandboxed);
        assert!(policy_sandboxed.is_allowed("exec"));
        assert!(!policy_sandboxed.is_allowed("browser")); // Denied by sandbox layer.
        assert!(!policy_sandboxed.is_allowed("web_search")); // Not in sandbox allow list.
    }

    #[test]
    fn test_profile_expanded_in_sender_and_provider_layers() {
        let mut cfg = moltis_config::MoltisConfig::default();
        cfg.tools.policy.allow = vec!["web_search".into()];

        // Layer 2: provider with profile = "full"
        cfg.providers
            .providers
            .insert("openai".into(), moltis_config::schema::ProviderEntry {
                policy: Some(moltis_config::schema::ToolPolicyConfig {
                    allow: Vec::new(),
                    deny: Vec::new(),
                    profile: Some("full".into()),
                }),
                ..Default::default()
            });

        let ctx = PolicyContext {
            agent_id: "main".into(),
            provider: Some("openai".into()),
            ..Default::default()
        };
        let policy = resolve_effective_policy(&cfg, &ctx);
        // "full" profile expands to allow = ["*"], so exec is now allowed.
        assert!(policy.is_allowed("exec"));
        assert!(policy.is_allowed("web_search"));

        // Layer 5: sender with profile = "full" in a restricted group.
        let mut cfg2 = moltis_config::MoltisConfig::default();
        cfg2.tools.policy.allow = vec!["web_search".into()];
        let account_config = serde_json::json!({
            "tools": {
                "groups": {
                    "group": {
                        "by_sender": {
                            "admin_42": {
                                "profile": "full"
                            }
                        }
                    }
                }
            }
        });
        cfg2.channels.telegram.insert("bot1".into(), account_config);

        let ctx2 = PolicyContext {
            agent_id: "main".into(),
            channel: Some("telegram".into()),
            channel_account_id: Some("bot1".into()),
            group_id: Some("group".into()),
            sender_id: Some("admin_42".into()),
            ..Default::default()
        };
        let policy2 = resolve_effective_policy(&cfg2, &ctx2);
        // Sender's "full" profile expands to allow = ["*"].
        assert!(policy2.is_allowed("exec"));
        assert!(policy2.is_allowed("browser"));
    }
}
