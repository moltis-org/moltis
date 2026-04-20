#![allow(clippy::expect_used, clippy::unwrap_used)]

use {
    crate::{
        prompt::{
            ModelFamily, PromptBuildLimits, PromptHostRuntimeContext, PromptRuntimeContext,
            PromptSandboxRuntimeContext, build_system_prompt, build_system_prompt_minimal_runtime,
            build_system_prompt_with_session_runtime,
            build_system_prompt_with_session_runtime_details,
            formatting::{format_compact_tool_schema, tool_call_guidance},
            runtime_datetime_message,
        },
        tool_registry::ToolRegistry,
    },
    moltis_config::{AgentIdentity, UserProfile},
    moltis_skills::types::SkillMetadata,
};

#[test]
fn test_native_prompt_does_not_include_tool_call_format() {
    let tools = ToolRegistry::new();
    let prompt = build_system_prompt(&tools, true, None);
    assert!(!prompt.contains("```tool_call"));
}

#[test]
fn test_fallback_prompt_includes_tool_call_format() {
    let mut tools = ToolRegistry::new();
    struct Dummy;
    #[async_trait::async_trait]
    impl crate::tool_registry::AgentTool for Dummy {
        fn name(&self) -> &str {
            "test"
        }

        fn description(&self) -> &str {
            "A test tool"
        }

        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({"type": "object", "properties": {}})
        }

        async fn execute(&self, _: serde_json::Value) -> anyhow::Result<serde_json::Value> {
            Ok(serde_json::json!({}))
        }
    }
    tools.register(Box::new(Dummy));

    let prompt = build_system_prompt(&tools, false, None);
    assert!(prompt.contains("```tool_call"));
    assert!(prompt.contains("### test"));
}

#[test]
fn test_native_prompt_uses_compact_tool_list() {
    let mut tools = ToolRegistry::new();
    struct Dummy;
    #[async_trait::async_trait]
    impl crate::tool_registry::AgentTool for Dummy {
        fn name(&self) -> &str {
            "test"
        }

        fn description(&self) -> &str {
            "A test tool"
        }

        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({"type": "object", "properties": {"cmd": {"type": "string"}}})
        }

        async fn execute(&self, _: serde_json::Value) -> anyhow::Result<serde_json::Value> {
            Ok(serde_json::json!({}))
        }
    }
    tools.register(Box::new(Dummy));

    let prompt = build_system_prompt(&tools, true, None);
    assert!(prompt.contains("## Available Tools"));
    assert!(prompt.contains("- `test`: A test tool"));
    assert!(!prompt.contains("Parameters:"));
}

#[test]
fn test_skills_injected_into_prompt() {
    let tools = ToolRegistry::new();
    let skills = vec![SkillMetadata {
        name: "commit".into(),
        description: "Create git commits".into(),
        path: std::path::PathBuf::from("/skills/commit"),
        ..Default::default()
    }];
    let prompt = build_system_prompt_with_session_runtime(
        &tools, true, None, &skills, None, None, None, None, None, None, None, None, None,
    );
    assert!(prompt.contains("<available_skills>"));
    assert!(prompt.contains("commit"));
    assert!(prompt.contains("read_skill"));
    assert!(!prompt.contains("/skills/commit"));
}

#[test]
fn test_no_skills_block_when_empty() {
    let tools = ToolRegistry::new();
    let prompt = build_system_prompt_with_session_runtime(
        &tools,
        true,
        None,
        &[],
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
    );
    assert!(!prompt.contains("<available_skills>"));
}

#[test]
fn test_identity_injected_into_prompt() {
    let tools = ToolRegistry::new();
    let identity = AgentIdentity {
        name: Some("Momo".into()),
        emoji: Some("🦜".into()),
        theme: Some("cheerful parrot".into()),
    };
    let user = UserProfile {
        name: Some("Alice".into()),
        timezone: None,
        location: None,
    };
    let prompt = build_system_prompt_with_session_runtime(
        &tools,
        true,
        None,
        &[],
        Some(&identity),
        Some(&user),
        None,
        None,
        None,
        None,
        None,
        None,
        None,
    );
    assert!(prompt.contains("Your name is Momo 🦜."));
    assert!(prompt.contains("Your theme: cheerful parrot."));
    assert!(prompt.contains("The user's name is Alice."));
    assert!(prompt.contains("## Soul"));
    assert!(prompt.contains("Be genuinely helpful"));
}

#[test]
fn test_custom_soul_injected() {
    let tools = ToolRegistry::new();
    let identity = AgentIdentity {
        name: Some("Rex".into()),
        ..Default::default()
    };
    let prompt = build_system_prompt_with_session_runtime(
        &tools,
        true,
        None,
        &[],
        Some(&identity),
        None,
        Some("You are a loyal companion who loves fetch."),
        None,
        None,
        None,
        None,
        None,
        None,
    );
    assert!(prompt.contains("## Soul"));
    assert!(prompt.contains("loyal companion who loves fetch"));
    assert!(!prompt.contains("Be genuinely helpful"));
}

#[test]
fn test_no_identity_no_extra_lines() {
    let tools = ToolRegistry::new();
    let prompt = build_system_prompt_with_session_runtime(
        &tools,
        true,
        None,
        &[],
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
    );
    assert!(!prompt.contains("Your name is"));
    assert!(!prompt.contains("The user's name is"));
    assert!(!prompt.contains("## Soul"));
}

#[test]
fn test_workspace_files_injected_when_provided() {
    let tools = ToolRegistry::new();
    let prompt = build_system_prompt_with_session_runtime(
        &tools,
        true,
        None,
        &[],
        None,
        None,
        None,
        None,
        Some("Follow workspace agent instructions."),
        Some("Prefer read-only tools first."),
        None,
        None,
        None,
    );
    assert!(prompt.contains("## Workspace Files"));
    assert!(prompt.contains("### AGENTS.md (workspace)"));
    assert!(prompt.contains("Follow workspace agent instructions."));
    assert!(prompt.contains("### TOOLS.md (workspace)"));
    assert!(prompt.contains("Prefer read-only tools first."));
}

#[test]
fn test_workspace_file_metadata_marks_truncation() {
    let tools = ToolRegistry::new();
    let output = build_system_prompt_with_session_runtime_details(
        &tools,
        true,
        None,
        &[],
        None,
        None,
        None,
        None,
        Some("abcdefghijklmnopqrstuvwxyz"),
        None,
        None,
        None,
        PromptBuildLimits {
            workspace_file_max_chars: 10,
            ..Default::default()
        },
        None,
    );

    assert!(output.metadata.truncated());
    assert_eq!(output.metadata.workspace_files.len(), 1);
    let status = &output.metadata.workspace_files[0];
    assert_eq!(status.name, "AGENTS.md");
    assert_eq!(status.original_chars, 26);
    assert_eq!(status.included_chars, 10);
    assert_eq!(status.limit_chars, 10);
    assert_eq!(status.truncated_chars, 16);
    assert!(status.truncated);
    assert!(
        output
            .prompt
            .contains("AGENTS.md truncated for prompt size")
    );
}

#[test]
fn test_runtime_context_injected_when_provided() {
    let tools = ToolRegistry::new();
    let runtime = PromptRuntimeContext {
        host: PromptHostRuntimeContext {
            host: Some("moltis-devbox".into()),
            os: Some("macos".into()),
            arch: Some("aarch64".into()),
            shell: Some("zsh".into()),
            time: Some("2026-02-17 16:18:00 CET".into()),
            today: Some("2026-02-17".into()),
            provider: Some("openai".into()),
            model: Some("gpt-5".into()),
            session_key: Some("main".into()),
            surface: None,
            session_kind: None,
            channel_type: None,
            channel_account_id: None,
            channel_chat_id: None,
            channel_chat_type: None,
            channel_sender_id: None,
            data_dir: Some("/home/moltis/.moltis".into()),
            sudo_non_interactive: Some(true),
            sudo_status: Some("passwordless".into()),
            timezone: Some("Europe/Paris".into()),
            accept_language: Some("en-US,fr;q=0.9".into()),
            remote_ip: Some("203.0.113.42".into()),
            location: None,
        },
        sandbox: Some(PromptSandboxRuntimeContext {
            exec_sandboxed: true,
            mode: Some("all".into()),
            backend: Some("docker".into()),
            scope: Some("session".into()),
            image: Some("moltis-sandbox:abc123".into()),
            home: Some("/home/sandbox".into()),
            workspace_mount: Some("ro".into()),
            workspace_path: Some("/home/moltis/.moltis".into()),
            no_network: Some(true),
            session_override: Some(true),
        }),
        nodes: None,
    };

    let prompt = build_system_prompt_with_session_runtime(
        &tools,
        true,
        None,
        &[],
        None,
        None,
        None,
        None,
        None,
        None,
        Some(&runtime),
        None,
        None,
    );

    assert!(prompt.contains("## Runtime"));
    assert!(prompt.contains("Host: host=moltis-devbox"));
    assert!(!prompt.contains("time=2026-02-17 16:18:00 CET"));
    assert!(!prompt.contains("today="));
    assert!(!prompt.contains("Today is"));
    assert!(!prompt.contains("The current user datetime is"));
    assert!(prompt.contains("provider=openai"));
    assert!(prompt.contains("model=gpt-5"));
    assert!(prompt.contains("data_dir=/home/moltis/.moltis"));
    assert!(prompt.contains("sudo_non_interactive=true"));
    assert!(prompt.contains("sudo_status=passwordless"));
    assert!(prompt.contains("timezone=Europe/Paris"));
    assert!(prompt.contains("accept_language=en-US,fr;q=0.9"));
    assert!(prompt.contains("remote_ip=203.0.113.42"));
    assert!(prompt.contains("Sandbox(exec): enabled=true"));
    assert!(prompt.contains("backend=docker"));
    assert!(prompt.contains("home=/home/sandbox"));
    assert!(prompt.contains("workspace_path=/home/moltis/.moltis"));
    assert!(prompt.contains("network=disabled"));
    assert!(prompt.contains("Execution routing:"));
    assert!(prompt.contains("`~` and relative paths resolve under"));
    assert!(prompt.contains("Sandbox/host routing changes are expected runtime behavior"));
    assert!(prompt.contains("sudo_non_interactive=true` means non-interactive sudo"));
}

#[test]
fn test_runtime_context_sandbox_without_sudo_omits_sudo_hint() {
    let tools = ToolRegistry::new();
    let runtime = PromptRuntimeContext {
        host: PromptHostRuntimeContext {
            host: Some("devbox".into()),
            ..Default::default()
        },
        sandbox: Some(PromptSandboxRuntimeContext {
            exec_sandboxed: true,
            mode: Some("all".into()),
            backend: Some("docker".into()),
            home: Some("/home/sandbox".into()),
            ..Default::default()
        }),
        nodes: None,
    };

    let prompt = build_system_prompt_with_session_runtime(
        &tools,
        true,
        None,
        &[],
        None,
        None,
        None,
        None,
        None,
        None,
        Some(&runtime),
        None,
        None,
    );

    assert!(prompt.contains("Sandbox(exec): enabled=true"));
    assert!(prompt.contains("Execution routing:"));
    assert!(prompt.contains("runs inside sandbox"));
    assert!(prompt.contains("Sandbox/host routing changes are expected runtime behavior"));
    assert!(!prompt.contains("sudo_non_interactive=true` means non-interactive sudo"));
}

#[test]
fn test_runtime_context_no_sandbox_uses_host_only_routing() {
    let tools = ToolRegistry::new();
    let runtime = PromptRuntimeContext {
        host: PromptHostRuntimeContext {
            host: Some("container-host".into()),
            os: Some("linux".into()),
            ..Default::default()
        },
        sandbox: None,
        nodes: None,
    };

    let prompt = build_system_prompt_with_session_runtime(
        &tools,
        true,
        None,
        &[],
        None,
        None,
        None,
        None,
        None,
        None,
        Some(&runtime),
        None,
        None,
    );

    assert!(prompt.contains("## Runtime"));
    assert!(prompt.contains("Host: host=container-host"));
    assert!(!prompt.contains("Sandbox(exec)"));
    assert!(prompt.contains("Execution routing:"));
    assert!(prompt.contains("`exec` runs on the host"));
    assert!(!prompt.contains("runs inside sandbox"));
    assert!(!prompt.contains("Sandbox/host routing changes"));
    assert!(!prompt.contains("sudo_non_interactive"));
}

#[test]
fn test_runtime_context_no_sandbox_with_sudo_includes_sudo_hint() {
    let tools = ToolRegistry::new();
    let runtime = PromptRuntimeContext {
        host: PromptHostRuntimeContext {
            host: Some("container-host".into()),
            sudo_non_interactive: Some(true),
            ..Default::default()
        },
        sandbox: None,
        nodes: None,
    };

    let prompt = build_system_prompt_with_session_runtime(
        &tools,
        true,
        None,
        &[],
        None,
        None,
        None,
        None,
        None,
        None,
        Some(&runtime),
        None,
        None,
    );

    assert!(prompt.contains("`exec` runs on the host"));
    assert!(!prompt.contains("runs inside sandbox"));
    assert!(prompt.contains("sudo_non_interactive=true` means non-interactive sudo"));
}

#[test]
fn test_runtime_context_includes_location_when_set() {
    let tools = ToolRegistry::new();
    let runtime = PromptRuntimeContext {
        host: PromptHostRuntimeContext {
            host: Some("devbox".into()),
            location: Some("48.8566,2.3522".into()),
            ..Default::default()
        },
        sandbox: None,
        nodes: None,
    };

    let prompt = build_system_prompt_with_session_runtime(
        &tools,
        true,
        None,
        &[],
        None,
        None,
        None,
        None,
        None,
        None,
        Some(&runtime),
        None,
        None,
    );

    assert!(prompt.contains("location=48.8566,2.3522"));
}

#[test]
fn test_runtime_context_includes_channel_surface_fields_when_set() {
    let tools = ToolRegistry::new();
    let runtime = PromptRuntimeContext {
        host: PromptHostRuntimeContext {
            session_key: Some("telegram:bot-main:123456".into()),
            surface: Some("telegram".into()),
            session_kind: Some("channel".into()),
            channel_type: Some("telegram".into()),
            channel_account_id: Some("bot-main".into()),
            channel_chat_id: Some("123456".into()),
            channel_chat_type: Some("private".into()),
            ..Default::default()
        },
        sandbox: None,
        nodes: None,
    };

    let prompt = build_system_prompt_with_session_runtime(
        &tools,
        true,
        None,
        &[],
        None,
        None,
        None,
        None,
        None,
        None,
        Some(&runtime),
        None,
        None,
    );

    assert!(prompt.contains("surface=telegram"));
    assert!(prompt.contains("session_kind=channel"));
    assert!(prompt.contains("channel_type=telegram"));
    assert!(prompt.contains("channel_account=bot-main"));
    assert!(prompt.contains("channel_chat_id=123456"));
    assert!(prompt.contains("channel_chat_type=private"));
}

#[test]
fn test_runtime_context_omits_location_when_none() {
    let tools = ToolRegistry::new();
    let runtime = PromptRuntimeContext {
        host: PromptHostRuntimeContext {
            host: Some("devbox".into()),
            location: None,
            ..Default::default()
        },
        sandbox: None,
        nodes: None,
    };

    let prompt = build_system_prompt_with_session_runtime(
        &tools,
        true,
        None,
        &[],
        None,
        None,
        None,
        None,
        None,
        None,
        Some(&runtime),
        None,
        None,
    );

    assert!(!prompt.contains("location="));
}

#[test]
fn test_minimal_prompt_runtime_does_not_add_exec_routing_block() {
    let runtime = PromptRuntimeContext {
        host: PromptHostRuntimeContext {
            host: Some("moltis-devbox".into()),
            ..Default::default()
        },
        sandbox: None,
        nodes: None,
    };

    let prompt = build_system_prompt_minimal_runtime(
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        Some(&runtime),
        None,
        None,
    );

    assert!(prompt.contains("## Runtime"));
    assert!(prompt.contains("Host: host=moltis-devbox"));
    assert!(!prompt.contains("Sandbox(exec)"));
    assert!(!prompt.contains("Execution routing:"));
}

#[test]
fn test_silent_replies_section_in_tool_prompt() {
    let tools = ToolRegistry::new();
    let prompt = build_system_prompt(&tools, true, None);
    assert!(prompt.contains("## Silent Replies"));
    assert!(prompt.contains("empty response"));
    assert!(prompt.contains("Do not call tools for greetings"));
    assert!(prompt.contains("`/sh `"));
    assert!(prompt.contains("run it with `exec` exactly as written"));
    assert!(prompt.contains("Do not express surprise about sandbox vs host execution"));
    assert!(!prompt.contains("__SILENT__"));
}

#[test]
fn test_silent_replies_not_in_minimal_prompt() {
    let prompt = build_system_prompt_minimal_runtime(
        None, None, None, None, None, None, None, None, None, None,
    );
    assert!(!prompt.contains("## Silent Replies"));
}

#[test]
fn test_memory_text_injected_into_prompt() {
    let tools = ToolRegistry::new();
    let memory = "## User Facts\n- Lives in Paris\n- Speaks French";
    let prompt = build_system_prompt_with_session_runtime(
        &tools,
        true,
        None,
        &[],
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        Some(memory),
        None,
    );
    assert!(prompt.contains("## Long-Term Memory"));
    assert!(prompt.contains("Lives in Paris"));
    assert!(prompt.contains("Speaks French"));
    assert!(prompt.contains("information above is what you already know"));
}

#[test]
fn test_boot_text_injected_into_prompt() {
    let tools = ToolRegistry::new();
    let boot = "Run health check on startup.\n- Verify API key configured";
    let prompt = build_system_prompt_with_session_runtime(
        &tools,
        true,
        None,
        &[],
        None,
        None,
        None,
        Some(boot),
        None,
        None,
        None,
        None,
        None,
    );
    assert!(prompt.contains("## Startup Context (BOOT.md)"));
    assert!(prompt.contains("Run health check on startup."));
    assert!(prompt.contains("Verify API key configured"));
}

#[test]
fn test_memory_text_truncated_at_limit() {
    let tools = ToolRegistry::new();
    let large_memory = "x".repeat(8_000 + 500);
    let prompt = build_system_prompt_with_session_runtime(
        &tools,
        true,
        None,
        &[],
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        Some(&large_memory),
        None,
    );
    assert!(prompt.contains("## Long-Term Memory"));
    assert!(prompt.contains("MEMORY.md truncated"));
    assert!(!prompt.contains(&large_memory));
}

#[test]
fn test_no_memory_section_without_memory_or_tools() {
    let tools = ToolRegistry::new();
    let prompt = build_system_prompt_with_session_runtime(
        &tools,
        true,
        None,
        &[],
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
    );
    assert!(!prompt.contains("## Long-Term Memory"));
}

#[test]
fn test_memory_text_in_minimal_prompt() {
    let memory = "## Notes\n- Important fact";
    let prompt = build_system_prompt_minimal_runtime(
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        Some(memory),
        None,
    );
    assert!(prompt.contains("## Long-Term Memory"));
    assert!(prompt.contains("Important fact"));
    assert!(!prompt.contains("memory_search"));
}

fn registry_with_tools(names: &[&'static str]) -> ToolRegistry {
    struct NamedStub(&'static str);
    #[async_trait::async_trait]
    impl crate::tool_registry::AgentTool for NamedStub {
        fn name(&self) -> &str {
            self.0
        }

        fn description(&self) -> &str {
            "stub"
        }

        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({"type": "object", "properties": {}})
        }

        async fn execute(&self, _: serde_json::Value) -> anyhow::Result<serde_json::Value> {
            Ok(serde_json::json!({}))
        }
    }
    let mut reg = ToolRegistry::new();
    for name in names {
        reg.register(Box::new(NamedStub(name)));
    }
    reg
}

#[test]
fn test_memory_save_hint_injected_when_tool_registered() {
    let tools = registry_with_tools(&["memory_save"]);
    let prompt = build_system_prompt_with_session_runtime(
        &tools,
        true,
        None,
        &[],
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
    );
    assert!(prompt.contains("## Long-Term Memory"));
    assert!(prompt.contains("MUST call `memory_save`"));
}

#[test]
fn test_memory_forget_hint_injected_when_tool_registered() {
    let tools = registry_with_tools(&["memory_forget"]);
    let prompt = build_system_prompt_with_session_runtime(
        &tools,
        true,
        None,
        &[],
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
    );
    assert!(prompt.contains("## Long-Term Memory"));
    assert!(prompt.contains("MUST call `memory_forget`"));
}

#[test]
fn test_memory_delete_hint_injected_when_tool_registered() {
    let tools = registry_with_tools(&["memory_delete"]);
    let prompt = build_system_prompt_with_session_runtime(
        &tools,
        true,
        None,
        &[],
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
    );
    assert!(prompt.contains("## Long-Term Memory"));
    assert!(prompt.contains("Use `memory_delete` only when"));
}

#[test]
fn test_memory_save_hint_absent_without_tool() {
    let tools = ToolRegistry::new();
    let prompt = build_system_prompt_with_session_runtime(
        &tools,
        true,
        None,
        &[],
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
    );
    assert!(!prompt.contains("memory_save"));
}

#[test]
fn test_memory_search_and_save_hints_both_present() {
    let tools = registry_with_tools(&["memory_search", "memory_save"]);
    let memory = "## User Facts\n- Likes coffee";
    let prompt = build_system_prompt_with_session_runtime(
        &tools,
        true,
        None,
        &[],
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        Some(memory),
        None,
    );
    assert!(prompt.contains("## Long-Term Memory"));
    assert!(prompt.contains("Likes coffee"));
    assert!(prompt.contains("memory_search"));
    assert!(prompt.contains("MUST call `memory_save`"));
}

#[test]
fn test_memory_search_save_forget_and_delete_hints_all_present() {
    let tools = registry_with_tools(&[
        "memory_search",
        "memory_save",
        "memory_forget",
        "memory_delete",
    ]);
    let memory = "## User Facts\n- Likes coffee";
    let prompt = build_system_prompt_with_session_runtime(
        &tools,
        true,
        None,
        &[],
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        Some(memory),
        None,
    );
    assert!(prompt.contains("memory_search"));
    assert!(prompt.contains("MUST call `memory_save`"));
    assert!(prompt.contains("MUST call `memory_forget`"));
    assert!(prompt.contains("Use `memory_delete` only when"));
}

#[test]
fn test_system_prompt_does_not_contain_datetime() {
    let tools = ToolRegistry::new();
    let runtime = PromptRuntimeContext {
        host: PromptHostRuntimeContext {
            time: Some("2026-02-17 16:18:00 CET".into()),
            today: Some("2026-02-17".into()),
            ..Default::default()
        },
        sandbox: None,
        nodes: None,
    };

    let prompt = build_system_prompt_with_session_runtime(
        &tools,
        true,
        None,
        &[],
        None,
        None,
        None,
        None,
        None,
        None,
        Some(&runtime),
        None,
        None,
    );

    assert!(!prompt.contains("The current user datetime is"));
    assert!(!prompt.contains("The current user date is"));
}

#[test]
fn test_runtime_datetime_message_returns_time_when_present() {
    let runtime = PromptRuntimeContext {
        host: PromptHostRuntimeContext {
            time: Some("2026-02-17 16:18:00 CET".into()),
            today: Some("2026-02-17".into()),
            ..Default::default()
        },
        sandbox: None,
        nodes: None,
    };

    let msg = runtime_datetime_message(Some(&runtime));
    assert_eq!(
        msg.as_deref(),
        Some("The current user datetime is 2026-02-17 16:18:00 CET.")
    );
}

#[test]
fn test_runtime_datetime_message_falls_back_to_today() {
    let runtime = PromptRuntimeContext {
        host: PromptHostRuntimeContext {
            today: Some("2026-02-17".into()),
            ..Default::default()
        },
        sandbox: None,
        nodes: None,
    };

    let msg = runtime_datetime_message(Some(&runtime));
    assert_eq!(msg.as_deref(), Some("The current user date is 2026-02-17."));
}

#[test]
fn test_runtime_datetime_message_returns_none_without_time_or_date() {
    let runtime = PromptRuntimeContext {
        host: PromptHostRuntimeContext::default(),
        sandbox: None,
        nodes: None,
    };

    assert!(runtime_datetime_message(Some(&runtime)).is_none());
    assert!(runtime_datetime_message(None).is_none());
}

#[test]
fn model_family_detects_llama() {
    assert_eq!(
        ModelFamily::from_model_id("llama3.1:8b"),
        ModelFamily::Llama
    );
    assert_eq!(
        ModelFamily::from_model_id("meta-llama/Llama-3.3-70B"),
        ModelFamily::Llama,
    );
}

#[test]
fn model_family_detects_qwen() {
    assert_eq!(ModelFamily::from_model_id("qwen2.5:7b"), ModelFamily::Qwen);
    assert_eq!(
        ModelFamily::from_model_id("Qwen/Qwen2.5-Coder-32B"),
        ModelFamily::Qwen,
    );
}

#[test]
fn model_family_detects_mistral() {
    assert_eq!(
        ModelFamily::from_model_id("mistral:latest"),
        ModelFamily::Mistral
    );
    assert_eq!(
        ModelFamily::from_model_id("mixtral-8x7b"),
        ModelFamily::Mistral
    );
}

#[test]
fn model_family_detects_others() {
    assert_eq!(
        ModelFamily::from_model_id("deepseek-coder-v2:16b"),
        ModelFamily::DeepSeek,
    );
    assert_eq!(ModelFamily::from_model_id("gemma:7b"), ModelFamily::Gemma);
    assert_eq!(ModelFamily::from_model_id("phi-3:mini"), ModelFamily::Phi);
}

#[test]
fn model_family_unknown_for_unrecognized() {
    assert_eq!(ModelFamily::from_model_id("gpt-4o"), ModelFamily::Unknown);
    assert_eq!(
        ModelFamily::from_model_id("claude-3-opus"),
        ModelFamily::Unknown,
    );
}

#[test]
fn compact_schema_formats_required_and_optional_params() {
    let schema = serde_json::json!({
        "name": "exec",
        "description": "Run a shell command",
        "parameters": {
            "type": "object",
            "properties": {
                "command": {"type": "string"},
                "timeout": {"type": "integer"}
            },
            "required": ["command"]
        }
    });
    let out = format_compact_tool_schema(&schema);
    assert!(out.contains("### exec"));
    assert!(out.contains("Run a shell command"));
    assert!(out.contains("command (string, required)"));
    assert!(out.contains("timeout (integer)"));
}

#[test]
fn compact_schema_no_params_section_when_empty() {
    let schema = serde_json::json!({
        "name": "noop",
        "description": "Does nothing",
        "parameters": {"type": "object", "properties": {}}
    });
    let out = format_compact_tool_schema(&schema);
    assert!(out.contains("### noop"));
    assert!(!out.contains("Params:"));
}

#[test]
fn tool_call_guidance_includes_fenced_example() {
    let g = tool_call_guidance(Some("llama3.1:8b"));
    assert!(g.contains("```tool_call"));
    assert!(g.contains("\"tool\":"));
    assert!(g.contains("Example:"));
}

#[test]
fn tool_call_guidance_works_with_no_model() {
    let g = tool_call_guidance(None);
    assert!(g.contains("## How to call tools"));
    assert!(g.contains("```tool_call"));
}

#[test]
fn text_mode_prompt_uses_compact_schema() {
    let mut tools = ToolRegistry::new();
    struct ParamTool;
    #[async_trait::async_trait]
    impl crate::tool_registry::AgentTool for ParamTool {
        fn name(&self) -> &str {
            "exec"
        }

        fn description(&self) -> &str {
            "Run a shell command"
        }

        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({
                "type": "object",
                "properties": {
                    "command": {"type": "string"},
                    "timeout": {"type": "integer"}
                },
                "required": ["command"]
            })
        }

        async fn execute(&self, _: serde_json::Value) -> anyhow::Result<serde_json::Value> {
            Ok(serde_json::json!({}))
        }
    }
    tools.register(Box::new(ParamTool));

    let prompt = build_system_prompt(&tools, false, None);
    assert!(prompt.contains("### exec"));
    assert!(prompt.contains("Params: command (string, required)"));
    assert!(prompt.contains("## How to call tools"));
    assert!(prompt.contains("```tool_call"));
}

#[test]
fn test_custom_guidelines_replaces_hardcoded() {
    let tools = ToolRegistry::new();
    let custom = "## My Guidelines\n- Always be terse.\n";
    let prompt = build_system_prompt_with_session_runtime(
        &tools,
        true,
        None,
        &[],
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        Some(custom),
    );
    assert!(prompt.contains("My Guidelines"));
    assert!(!prompt.contains("## Silent Replies")); // hardcoded TOOL_GUIDELINES absent
}

#[test]
fn test_none_guidelines_uses_tool_guidelines() {
    let tools = ToolRegistry::new();
    let prompt = build_system_prompt_with_session_runtime(
        &tools,
        true,
        None,
        &[],
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
    );
    assert!(prompt.contains("## Silent Replies"));
}

#[test]
fn test_empty_string_guidelines_falls_through_to_hardcoded() {
    let tools = ToolRegistry::new();
    let prompt = build_system_prompt_with_session_runtime(
        &tools,
        true,
        None,
        &[],
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        Some(""),
    );
    // Some("") should fall through to TOOL_GUIDELINES, not produce no guidelines
    assert!(prompt.contains("## Silent Replies"));
}

#[test]
fn test_skill_self_improvement_included_when_enabled() {
    let tools = ToolRegistry::new();
    let skills = vec![SkillMetadata {
        name: "demo".into(),
        description: "Demo skill".into(),
        path: std::path::PathBuf::from("/skills/demo"),
        ..Default::default()
    }];
    let output = build_system_prompt_with_session_runtime_details(
        &tools,
        true,
        None,
        &skills,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        PromptBuildLimits {
            enable_skill_self_improvement: true,
            ..Default::default()
        },
        None,
    );
    assert!(
        output.prompt.contains("Skill Self-Improvement"),
        "self-improvement guidance should be present when enabled"
    );
}

#[test]
fn test_skill_self_improvement_omitted_when_disabled() {
    let tools = ToolRegistry::new();
    let skills = vec![SkillMetadata {
        name: "demo".into(),
        description: "Demo skill".into(),
        path: std::path::PathBuf::from("/skills/demo"),
        ..Default::default()
    }];
    let output = build_system_prompt_with_session_runtime_details(
        &tools,
        true,
        None,
        &skills,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        PromptBuildLimits {
            enable_skill_self_improvement: false,
            ..Default::default()
        },
        None,
    );
    assert!(
        !output.prompt.contains("Skill Self-Improvement"),
        "self-improvement guidance should be omitted when disabled"
    );
    // Skills block itself should still be present.
    assert!(output.prompt.contains("<available_skills>"));
}
