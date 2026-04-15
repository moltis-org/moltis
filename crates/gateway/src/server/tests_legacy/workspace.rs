use moltis_config::{AgentPreset, AgentsConfig, PresetToolPolicy};

#[test]
fn sync_persona_into_preset_creates_new_entry() {
    let mut agents = AgentsConfig::default();
    let persona = crate::agent_persona::AgentPersona {
        id: "writer".into(),
        name: "Creative Writer".into(),
        is_default: false,
        emoji: Some("✍️".into()),
        theme: Some("poetic".into()),
        description: None,
        created_at: 0,
        updated_at: 0,
    };

    crate::server::workspace::sync_persona_into_preset(&mut agents, &persona);

    let preset = agents.presets.get("writer").expect("preset should exist");
    assert_eq!(preset.identity.name.as_deref(), Some("Creative Writer"));
    assert_eq!(preset.identity.emoji.as_deref(), Some("✍️"));
    assert_eq!(preset.identity.theme.as_deref(), Some("poetic"));
}

#[test]
fn sync_persona_preserves_existing_preset_fields() {
    let mut agents = AgentsConfig::default();
    let existing = AgentPreset {
        model: Some("haiku".into()),
        timeout_secs: Some(30),
        tools: PresetToolPolicy {
            deny: vec!["exec".into()],
            ..Default::default()
        },
        ..Default::default()
    };
    agents.presets.insert("coder".into(), existing);

    let persona = crate::agent_persona::AgentPersona {
        id: "coder".into(),
        name: "Code Bot".into(),
        is_default: false,
        emoji: None,
        theme: None,
        description: None,
        created_at: 0,
        updated_at: 0,
    };

    crate::server::workspace::sync_persona_into_preset(&mut agents, &persona);

    let preset = agents.presets.get("coder").expect("preset should exist");
    assert_eq!(preset.identity.name.as_deref(), Some("Code Bot"));
    assert_eq!(preset.model.as_deref(), Some("haiku"));
    assert_eq!(preset.timeout_secs, Some(30));
    assert_eq!(preset.tools.deny, vec!["exec".to_string()]);
}
