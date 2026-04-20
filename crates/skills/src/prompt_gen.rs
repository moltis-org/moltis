use crate::{
    SIDECAR_SUBDIRS,
    types::{SkillMetadata, SkillSource},
};

/// Name of the native read tool advertised in the activation instruction.
/// Kept as a constant so the gateway can assert a parity invariant between
/// this string and the registered tool's [`AgentTool::name`] at test time.
pub const READ_SKILL_TOOL_NAME: &str = "read_skill";

/// Generate the `<available_skills>` XML block for injection into the system prompt.
///
/// The block lists each enabled skill's name, source, and description. It
/// deliberately does **not** include the absolute `SKILL.md` path: the model
/// should activate a skill by calling the native `read_skill` tool with the
/// skill name, which resolves through the same discoverer the prompt block
/// was built from.
pub fn generate_skills_prompt(skills: &[SkillMetadata]) -> String {
    if skills.is_empty() {
        return String::new();
    }

    let mut out = String::from("## Available Skills\n\n<available_skills>\n");
    for skill in skills {
        let source = if skill.source.as_ref() == Some(&SkillSource::Plugin) {
            "plugin"
        } else {
            "skill"
        };
        out.push_str(&format!(
            "<skill name=\"{}\" source=\"{}\">\n{}\n</skill>\n",
            skill.name, source, skill.description,
        ));
    }
    out.push_str("</available_skills>\n\n");
    // Format the per-subdir list directly from the shared SIDECAR_SUBDIRS
    // constant so adding a subdir in `moltis_skills::SIDECAR_SUBDIRS`
    // automatically updates the instruction. No drift between what the
    // prompt advertises and what the read tool actually walks.
    let subdir_list = SIDECAR_SUBDIRS
        .iter()
        .map(|s| format!("{s}/"))
        .collect::<Vec<_>>()
        .join(", ");
    out.push_str(&format!(
        "To activate a skill, call the `{READ_SKILL_TOOL_NAME}` tool with its name \
         (e.g. `{READ_SKILL_TOOL_NAME}(name=\"<skill-name>\")`). To load a sidecar \
         file inside a skill directory ({subdir_list}), pass the `file_path` \
         argument as well \
         (e.g. `{READ_SKILL_TOOL_NAME}(name=\"<skill-name>\", file_path=\"references/api.md\")`).\n\n",
    ));
    out
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

    #[test]
    fn test_empty_skills_produces_empty_string() {
        assert_eq!(generate_skills_prompt(&[]), "");
    }

    #[test]
    fn test_activation_instruction_mentions_all_sidecar_dirs() {
        // Parity guard: every entry in `moltis_skills::SIDECAR_SUBDIRS` must
        // be mentioned in the activation instruction. Iterating over the
        // shared constant means adding a new subdir in one place is enough
        // — the drift path is closed at compile time. Without this check,
        // models following the system prompt would never know to ask for
        // a new agentskills.io-standard sidecar.
        let skills = vec![SkillMetadata {
            name: "demo".into(),
            description: "demo".into(),
            path: PathBuf::from("/a"),
            source: Some(SkillSource::Personal),
            ..Default::default()
        }];
        let prompt = generate_skills_prompt(&skills);
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
        // The activation instruction must name `READ_SKILL_TOOL_NAME`
        // verbatim so the gateway's tool-registration/parity assertion can
        // pin the string. If someone ever renames the tool, this test (and
        // the gateway-side parity test) will fail together.
        let skills = vec![SkillMetadata {
            name: "demo".into(),
            description: "demo".into(),
            path: PathBuf::from("/a"),
            source: Some(SkillSource::Personal),
            ..Default::default()
        }];
        let prompt = generate_skills_prompt(&skills);
        assert!(prompt.contains(READ_SKILL_TOOL_NAME));
        // Also assert the concrete call shape so documentation stays in
        // sync: `read_skill(name="...")`.
        assert!(
            prompt.contains(&format!("{READ_SKILL_TOOL_NAME}(name=\"")),
            "instruction must include a concrete call example: {prompt}"
        );
    }

    #[test]
    fn test_single_skill_prompt() {
        let skills = vec![SkillMetadata {
            name: "commit".into(),
            description: "Create git commits".into(),
            path: PathBuf::from("/home/user/.moltis/skills/commit"),
            source: Some(SkillSource::Personal),
            ..Default::default()
        }];
        let prompt = generate_skills_prompt(&skills);
        assert!(prompt.contains("<available_skills>"));
        assert!(prompt.contains("name=\"commit\""));
        assert!(prompt.contains("source=\"skill\""));
        assert!(prompt.contains("Create git commits"));
        assert!(prompt.contains("</available_skills>"));
        assert!(
            prompt.contains("read_skill"),
            "activation instruction should name the read_skill tool"
        );
    }

    #[test]
    fn test_prompt_does_not_leak_absolute_paths() {
        // The prompt must never include absolute paths — that was the bug.
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
        // The <skill> element must not carry a path= attribute. The
        // activation instruction still mentions `file_path=` for sidecar
        // reads (which is fine — it's not a `<skill path="...">` attribute),
        // so we check for the exact quote-path-quote sequence that would
        // appear on a `<skill>` element.
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
        let skills = vec![
            SkillMetadata {
                name: "commit".into(),
                description: "Commits".into(),
                path: PathBuf::from("/a"),
                source: Some(SkillSource::Personal),
                ..Default::default()
            },
            SkillMetadata {
                name: "review".into(),
                description: "Reviews".into(),
                path: PathBuf::from("/b"),
                source: Some(SkillSource::Personal),
                ..Default::default()
            },
        ];
        let prompt = generate_skills_prompt(&skills);
        assert!(prompt.contains("name=\"commit\""));
        assert!(prompt.contains("name=\"review\""));
        // The activation instruction (which mentions `read_skill`) is emitted
        // once, not per-skill, so the match count should not grow with the
        // number of skills.
        let single_skill_prompt = generate_skills_prompt(&skills[..1]);
        assert_eq!(
            prompt.matches("read_skill").count(),
            single_skill_prompt.matches("read_skill").count()
        );
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
