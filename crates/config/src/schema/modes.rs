use {
    serde::{Deserialize, Deserializer, Serialize},
    std::collections::HashMap,
};

/// Session-scoped prompt overlays.
///
/// Modes are temporary workflow/personality overlays selected per session with
/// `/mode`. They do not create a new chat agent, do not affect sub-agents, and
/// do not change the selected agent's identity or memory.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ModesConfig {
    /// Named mode presets.
    #[serde(
        default = "default_mode_presets",
        deserialize_with = "deserialize_mode_presets"
    )]
    pub presets: HashMap<String, ModePreset>,
}

impl ModesConfig {
    /// Return a mode preset by id.
    pub fn get_preset(&self, name: &str) -> Option<&ModePreset> {
        self.presets.get(name)
    }
}

impl Default for ModesConfig {
    fn default() -> Self {
        Self {
            presets: default_mode_presets(),
        }
    }
}

/// Built-in modes available on every install.
///
/// The communication-style modes are adapted from Hermes' built-in
/// personalities. The workflow modes mirror common OpenClaw-style slash command
/// workflows: plan, build, review, research, and elevated execution.
#[must_use]
pub fn default_mode_presets() -> HashMap<String, ModePreset> {
    [
        (
            "concise",
            builtin_mode(
                "Concise",
                "brief, direct answers",
                "Keep responses brief and to the point. Preserve important caveats, but skip filler, repetition, and long setup.",
            ),
        ),
        (
            "technical",
            builtin_mode(
                "Technical",
                "detailed technical analysis",
                "Act as a technical expert. Be precise, include implementation details when useful, and call out assumptions, edge cases, and operational constraints.",
            ),
        ),
        (
            "creative",
            builtin_mode(
                "Creative",
                "broad ideation and alternatives",
                "Think creatively and explore non-obvious options. Offer concrete alternatives, explain tradeoffs, and avoid drifting into vague brainstorming.",
            ),
        ),
        (
            "teacher",
            builtin_mode(
                "Teacher",
                "patient explanations with examples",
                "Teach patiently. Explain concepts step by step, use examples, check for hidden prerequisites, and keep the explanation grounded in the user's goal.",
            ),
        ),
        (
            "plan",
            builtin_mode(
                "Plan",
                "think first, clarify scope",
                "Plan before acting. Identify goals, constraints, risks, and the smallest useful next steps. Do not make code changes unless the user confirms or explicitly asks you to proceed.",
            ),
        ),
        (
            "build",
            builtin_mode(
                "Build",
                "implementation-focused execution",
                "Focus on implementation. Read the relevant code first, make scoped changes, keep momentum, and verify the behavior with targeted checks.",
            ),
        ),
        (
            "review",
            builtin_mode(
                "Review",
                "bug-focused code review",
                "Review critically. Lead with bugs, regressions, security risks, and missing tests. Reference concrete files or behavior and avoid broad style commentary unless it affects correctness.",
            ),
        ),
        (
            "research",
            builtin_mode(
                "Research",
                "evidence-first investigation",
                "Research before concluding. Gather evidence from code, docs, or external sources as appropriate. Distinguish facts from inference and summarize sources and open questions.",
            ),
        ),
        (
            "elevated",
            builtin_mode(
                "Elevated",
                "extra care for risky operations",
                "Use extra caution. Treat destructive operations, secrets, credentials, security-sensitive changes, and production-impacting work as high risk. Prefer explicit confirmation before irreversible actions.",
            ),
        ),
    ]
    .into_iter()
    .map(|(name, preset)| (name.to_string(), preset))
    .collect()
}

fn deserialize_mode_presets<'de, D>(
    deserializer: D,
) -> Result<HashMap<String, ModePreset>, D::Error>
where
    D: Deserializer<'de>,
{
    let user_presets = HashMap::<String, ModePreset>::deserialize(deserializer)?;
    let mut presets = default_mode_presets();
    presets.extend(user_presets);
    Ok(presets)
}

fn builtin_mode(name: &str, description: &str, prompt: &str) -> ModePreset {
    ModePreset {
        name: Some(name.to_string()),
        description: Some(description.to_string()),
        prompt: prompt.to_string(),
    }
}

/// One selectable session mode.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ModePreset {
    /// Display name.
    pub name: Option<String>,
    /// Short user-facing description.
    pub description: Option<String>,
    /// Prompt overlay injected while this mode is active.
    pub prompt: String,
}
