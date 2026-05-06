use crate::{
    SIDECAR_SUBDIRS,
    types::{SkillMetadata, SkillSource},
};

/// Name of the native read tool advertised in the activation instruction.
/// Kept as a constant so the gateway can assert a parity invariant between
/// this string and the registered tool's [`AgentTool::name`] at test time.
pub const READ_SKILL_TOOL_NAME: &str = "read_skill";

/// Default character budget for the skills prompt block. At ~4 chars/token
/// this is ~7,500 tokens — generous for 100+ skills in full format.
const DEFAULT_MAX_CHARS: usize = 30_000;

/// Generate the `<available_skills>` XML block for injection into the system prompt.
///
/// Uses a two-tier format strategy with a character budget (default 30 KB):
///
/// 1. **Full format** — each skill gets `name`, `source`, `category`, and
///    description. Used when all skills fit within the budget.
/// 2. **Compact format** — drops descriptions, keeps only `name`, `source`,
///    and `category`. Triggered when full format exceeds the budget. Preserves
///    awareness of all skills before dropping any.
///
/// If even compact format exceeds the budget, skills are truncated (lowest
/// priority last — bundled skills are appended after user skills).
pub fn generate_skills_prompt(skills: &[SkillMetadata]) -> String {
    generate_skills_prompt_with_budget(skills, DEFAULT_MAX_CHARS)
}

/// Generate the skills prompt with an explicit character budget.
pub fn generate_skills_prompt_with_budget(skills: &[SkillMetadata], max_chars: usize) -> String {
    if skills.is_empty() {
        return String::new();
    }

    let activation = build_activation_instruction();

    // Try full format first.
    let full = build_skills_block(skills, Format::Full);
    if full.len() + activation.len() <= max_chars {
        return format!("## Available Skills\n\n{full}\n{activation}");
    }

    // Fall back to compact format (drop descriptions).
    let compact = build_skills_block(skills, Format::Compact);
    if compact.len() + activation.len() <= max_chars {
        return format!(
            "## Available Skills (compact — call `{READ_SKILL_TOOL_NAME}` to see full descriptions)\n\n\
             {compact}\n{activation}"
        );
    }

    // Compact still too large — binary search for the largest prefix that fits.
    let mut lo = 0usize;
    let mut hi = skills.len();
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        let block = build_skills_block(&skills[..mid], Format::Compact);
        if block.len() + activation.len() <= max_chars {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    let truncated = build_skills_block(&skills[..lo], Format::Compact);
    format!(
        "## Available Skills (compact, showing {lo} of {} — call `{READ_SKILL_TOOL_NAME}` to browse all)\n\n\
         {truncated}\n{activation}",
        skills.len()
    )
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Format {
    Full,
    Compact,
}

fn build_skills_block(skills: &[SkillMetadata], format: Format) -> String {
    let mut out = String::from("<available_skills>\n");
    for skill in skills {
        let source = match skill.source.as_ref() {
            Some(SkillSource::Plugin) => "plugin",
            Some(SkillSource::Bundled) => "bundled",
            _ => "skill",
        };
        let category_attr = skill
            .category
            .as_deref()
            .map(|c| format!(" category=\"{c}\""))
            .unwrap_or_default();
        match format {
            Format::Full => {
                out.push_str(&format!(
                    "<skill name=\"{}\" source=\"{}\"{category_attr}>\n{}\n</skill>\n",
                    skill.name, source, skill.description,
                ));
            },
            Format::Compact => {
                out.push_str(&format!(
                    "<skill name=\"{}\" source=\"{}\"{category_attr} />\n",
                    skill.name, source,
                ));
            },
        }
    }
    out.push_str("</available_skills>\n");
    out
}

fn build_activation_instruction() -> String {
    let subdir_list = SIDECAR_SUBDIRS
        .iter()
        .map(|s| format!("{s}/"))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "\nTo activate a skill, call the `{READ_SKILL_TOOL_NAME}` tool with its name \
         (e.g. `{READ_SKILL_TOOL_NAME}(name=\"<skill-name>\")`). To load a sidecar \
         file inside a skill directory ({subdir_list}), pass the `file_path` \
         argument as well \
         (e.g. `{READ_SKILL_TOOL_NAME}(name=\"<skill-name>\", file_path=\"references/api.md\")`). \
         If `{READ_SKILL_TOOL_NAME}` is not currently listed in Available Tools, activate it first with \
         `tool_search(name=\"{READ_SKILL_TOOL_NAME}\")`.\n\n",
    )
}

/// Generate the self-improvement guidance section for the system prompt.
///
/// This instructs the agent to proactively create and maintain skills after
/// complex tasks. Appended after the `<available_skills>` block when
/// `[skills] enable_self_improvement = true` (the default).
pub fn generate_skill_self_improvement_prompt() -> &'static str {
    "\
## Skill Self-Improvement

You have tools to create, read, update, and delete personal skills. Use them proactively:

- After completing a complex task (5+ tool calls), consider saving the approach as a reusable skill with `create_skill`
- After fixing a tricky error or discovering a non-obvious workflow, save it so you don't have to rediscover it
- When a skill you're using has stale or incorrect instructions, fix it with `patch_skill` (surgical find/replace) or `update_skill` (full rewrite)
- When you notice a skill could benefit from reference data, use `write_skill_files` to add sidecar files

Do NOT create skills for trivial or one-off tasks. Good skills encode multi-step procedures, domain-specific knowledge, or workflows that are likely to recur.
"
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn skill(name: &str, desc: &str) -> SkillMetadata {
        SkillMetadata {
            name: name.into(),
            description: desc.into(),
            path: PathBuf::from("/a"),
            source: Some(SkillSource::Personal),
            ..Default::default()
        }
    }

    #[test]
    fn test_empty_skills_produces_empty_string() {
        assert_eq!(generate_skills_prompt(&[]), "");
    }

    #[test]
    fn test_activation_instruction_mentions_all_sidecar_dirs() {
        let prompt = generate_skills_prompt(&[skill("demo", "demo")]);
        for sub in SIDECAR_SUBDIRS {
            let needle = format!("{sub}/");
            assert!(
                prompt.contains(&needle),
                "activation instruction should mention {needle}: {prompt}"
            );
        }
    }

    #[test]
    fn test_activation_instruction_uses_read_skill_tool_name_constant() {
        let prompt = generate_skills_prompt(&[skill("demo", "demo")]);
        assert!(prompt.contains(READ_SKILL_TOOL_NAME));
        assert!(
            prompt.contains(&format!("{READ_SKILL_TOOL_NAME}(name=\"")),
            "instruction must include a concrete call example: {prompt}"
        );
        assert!(
            prompt.contains(&format!("tool_search(name=\"{READ_SKILL_TOOL_NAME}\")")),
            "lazy-mode instruction must explain how to activate read_skill: {prompt}"
        );
    }

    #[test]
    fn test_single_skill_prompt() {
        let prompt = generate_skills_prompt(&[skill("commit", "Create git commits")]);
        assert!(prompt.contains("<available_skills>"));
        assert!(prompt.contains("name=\"commit\""));
        assert!(prompt.contains("source=\"skill\""));
        assert!(prompt.contains("Create git commits"));
        assert!(prompt.contains("</available_skills>"));
        assert!(prompt.contains("read_skill"));
    }

    #[test]
    fn test_prompt_does_not_leak_absolute_paths() {
        let skills = vec![SkillMetadata {
            name: "demo".into(),
            description: "A demo skill".into(),
            path: PathBuf::from("/home/secretuser/.moltis/skills/demo"),
            source: Some(SkillSource::Personal),
            ..Default::default()
        }];
        let prompt = generate_skills_prompt(&skills);
        assert!(
            !prompt.contains("/home/secretuser"),
            "prompt leaked absolute path: {prompt}"
        );
        assert!(
            !prompt.contains("SKILL.md"),
            "prompt should no longer mention SKILL.md: {prompt}"
        );
        assert!(
            !prompt.contains("\" path=\""),
            "prompt should not include a path= attribute on the <skill> element: {prompt}"
        );
    }

    #[test]
    fn test_plugin_source_is_labelled_as_plugin() {
        let skills = vec![SkillMetadata {
            name: "plugin-helper".into(),
            description: "Helper plugin".into(),
            path: PathBuf::from("/opt/plugins/helper.md"),
            source: Some(SkillSource::Plugin),
            ..Default::default()
        }];
        let prompt = generate_skills_prompt(&skills);
        assert!(prompt.contains("source=\"plugin\""));
        assert!(!prompt.contains("/opt/plugins"));
    }

    #[test]
    fn test_multiple_skills() {
        let skills = vec![skill("commit", "Commits"), skill("review", "Reviews")];
        let prompt = generate_skills_prompt(&skills);
        assert!(prompt.contains("name=\"commit\""));
        assert!(prompt.contains("name=\"review\""));
        let single_skill_prompt = generate_skills_prompt(&skills[..1]);
        assert_eq!(
            prompt.matches("read_skill").count(),
            single_skill_prompt.matches("read_skill").count()
        );
    }

    #[test]
    fn test_category_attribute() {
        let skills = vec![SkillMetadata {
            name: "arxiv".into(),
            description: "Search papers".into(),
            category: Some("research".into()),
            path: PathBuf::from("/a"),
            source: Some(SkillSource::Bundled),
            ..Default::default()
        }];
        let prompt = generate_skills_prompt(&skills);
        assert!(prompt.contains("category=\"research\""));
        assert!(prompt.contains("source=\"bundled\""));
    }

    // ── Format fallback tests ───────────────────────────────────────

    #[test]
    fn full_format_within_budget() {
        let skills = vec![skill("a", "desc a"), skill("b", "desc b")];
        let prompt = generate_skills_prompt_with_budget(&skills, 10_000);
        // Full format includes descriptions.
        assert!(prompt.contains("desc a"));
        assert!(prompt.contains("desc b"));
        assert!(prompt.contains("## Available Skills\n"));
        assert!(!prompt.contains("compact"));
    }

    #[test]
    fn compact_fallback_when_full_exceeds_budget() {
        let skills: Vec<_> = (0..50)
            .map(|i| skill(&format!("skill-{i}"), &"x".repeat(200)))
            .collect();
        // Tiny budget forces compact.
        let prompt = generate_skills_prompt_with_budget(&skills, 3_000);
        assert!(prompt.contains("compact"));
        // Compact uses self-closing tags, no descriptions.
        assert!(prompt.contains("/>"));
        assert!(!prompt.contains(&"x".repeat(200)));
        // All skills still present.
        assert!(prompt.contains("skill-0"));
        assert!(prompt.contains("skill-49"));
    }

    #[test]
    fn truncation_when_compact_still_exceeds_budget() {
        let skills: Vec<_> = (0..200)
            .map(|i| skill(&format!("skill-{i:03}"), "d"))
            .collect();
        // Very tiny budget.
        let prompt = generate_skills_prompt_with_budget(&skills, 1_500);
        assert!(prompt.contains("compact"));
        assert!(prompt.contains("showing"));
        assert!(prompt.contains("of 200"));
        // First skill present, last skill truncated.
        assert!(prompt.contains("skill-000"));
    }

    #[test]
    fn default_budget_fits_100_skills() {
        let skills: Vec<_> = (0..100)
            .map(|i| {
                skill(
                    &format!("skill-{i}"),
                    &format!("Description of skill {i} that is moderately long"),
                )
            })
            .collect();
        let prompt = generate_skills_prompt(&skills);
        // With default 30KB budget, 100 skills should fit in full format.
        assert!(!prompt.contains("compact"));
        assert!(prompt.contains("skill-0"));
        assert!(prompt.contains("skill-99"));
    }

    #[test]
    fn test_self_improvement_prompt_contains_key_guidance() {
        let prompt = generate_skill_self_improvement_prompt();
        assert!(prompt.contains("Skill Self-Improvement"));
        assert!(prompt.contains("create_skill"));
        assert!(prompt.contains("patch_skill"));
        assert!(prompt.contains("update_skill"));
        assert!(
            prompt.contains("5+ tool calls"),
            "should mention the complexity threshold"
        );
        assert!(
            prompt.contains("Do NOT create skills for trivial"),
            "should discourage trivial skill creation"
        );
    }
}
