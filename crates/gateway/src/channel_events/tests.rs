#![allow(clippy::unwrap_used)]

use {super::*, crate::channel_events::commands::formatting::unique_providers};

use moltis_channels::ChannelType;

#[test]
fn channel_event_serialization() {
    let event = ChannelEvent::InboundMessage {
        channel_type: ChannelType::Telegram,
        account_id: "bot1".into(),
        peer_id: "123".into(),
        username: Some("alice".into()),
        sender_name: Some("Alice".into()),
        message_count: Some(5),
        access_granted: true,
    };
    let json = serde_json::to_value(&event).unwrap();
    assert_eq!(json["kind"], "inbound_message");
    assert_eq!(json["channel_type"], "telegram");
    assert_eq!(json["account_id"], "bot1");
    assert_eq!(json["peer_id"], "123");
    assert_eq!(json["username"], "alice");
    assert_eq!(json["sender_name"], "Alice");
    assert_eq!(json["message_count"], 5);
    assert_eq!(json["access_granted"], true);
}

#[test]
fn channel_session_key_format() {
    let target = ChannelReplyTarget {
        ack_message_id: None,
        channel_type: ChannelType::Telegram,
        account_id: "bot1".into(),
        chat_id: "12345".into(),
        message_id: None,
        thread_id: None,
    };
    assert_eq!(default_channel_session_key(&target), "telegram:bot1:12345");
}

#[test]
fn channel_session_key_group() {
    let target = ChannelReplyTarget {
        ack_message_id: None,
        channel_type: ChannelType::Telegram,
        account_id: "bot1".into(),
        chat_id: "-100999".into(),
        message_id: None,
        thread_id: None,
    };
    assert_eq!(
        default_channel_session_key(&target),
        "telegram:bot1:-100999"
    );
}

#[test]
fn channel_session_key_forum_topic() {
    let target = ChannelReplyTarget {
        ack_message_id: None,
        channel_type: ChannelType::Telegram,
        account_id: "bot1".into(),
        chat_id: "-100999".into(),
        message_id: None,
        thread_id: Some("42".into()),
    };
    assert_eq!(
        default_channel_session_key(&target),
        "telegram:bot1:-100999:42"
    );
}

#[test]
fn channel_event_serialization_nulls() {
    let event = ChannelEvent::InboundMessage {
        channel_type: ChannelType::Telegram,
        account_id: "bot1".into(),
        peer_id: "123".into(),
        username: None,
        sender_name: None,
        message_count: None,
        access_granted: false,
    };
    let json = serde_json::to_value(&event).unwrap();
    assert_eq!(json["kind"], "inbound_message");
    assert!(json["username"].is_null());
    assert_eq!(json["access_granted"], false);
}

#[test]
fn shell_mode_rewrite_plain_text() {
    assert_eq!(
        rewrite_for_shell_mode("uname -a").as_deref(),
        Some("/sh uname -a")
    );
}

#[test]
fn shell_mode_rewrite_skips_control_commands() {
    assert!(rewrite_for_shell_mode("/context").is_none());
    assert!(rewrite_for_shell_mode("/attach").is_none());
    assert!(rewrite_for_shell_mode("/sh uname -a").is_none());
}

#[test]
fn peek_and_stop_are_control_commands() {
    assert!(is_channel_control_command_name("peek"));
    assert!(is_channel_control_command_name("stop"));
    assert!(is_channel_control_command_name("attach"));
    assert!(is_channel_control_command_name("approvals"));
    assert!(is_channel_control_command_name("approve"));
    assert!(is_channel_control_command_name("deny"));
}

#[test]
fn shell_mode_rewrite_skips_peek_and_stop() {
    assert!(rewrite_for_shell_mode("/peek").is_none());
    assert!(rewrite_for_shell_mode("/stop").is_none());
}

// ── shell access gating ────────────────────────────────────────

/// `dispatch_to_chat` blocks `/sh <cmd>` from non-operators using
/// `explicit_shell_command`. It must recognise every form the agent runner
/// force-executes, or a guest could slip a command past the gate and still
/// have it run.
#[test]
fn explicit_shell_command_detects_forms_the_runner_executes() {
    use moltis_agents::runner::explicit_shell_command;

    assert_eq!(explicit_shell_command("/sh pwd").as_deref(), Some("pwd"));
    assert_eq!(
        explicit_shell_command("/sh   cat /etc/passwd").as_deref(),
        Some("cat /etc/passwd")
    );
    // Channel-mention form (Telegram/Discord append the bot name).
    assert_eq!(
        explicit_shell_command("/sh@mybot uname -a").as_deref(),
        Some("uname -a")
    );
    // Case-insensitive head.
    assert_eq!(
        explicit_shell_command("/SH whoami").as_deref(),
        Some("whoami")
    );
    // Leading whitespace must not hide the command from the gate.
    assert_eq!(explicit_shell_command("  /sh id").as_deref(), Some("id"));
}

#[test]
fn explicit_shell_command_ignores_ordinary_chat() {
    use moltis_agents::runner::explicit_shell_command;

    assert!(explicit_shell_command("hello there").is_none());
    assert!(explicit_shell_command("/sh").is_none());
    assert!(explicit_shell_command("/shell pwd").is_none());
    assert!(explicit_shell_command("/context").is_none());
    // Talking *about* the command is not a request to run it.
    assert!(explicit_shell_command("what does /sh do?").is_none());
}

#[test]
fn unknown_and_group_chat_types_are_shared() {
    let discord = ChannelReplyTarget {
        channel_type: ChannelType::Discord,
        account_id: "bot".into(),
        chat_id: "123".into(),
        message_id: None,
        thread_id: None,
        ack_message_id: None,
    };
    let telegram_group = ChannelReplyTarget {
        channel_type: ChannelType::Telegram,
        chat_id: "-123".into(),
        ..discord.clone()
    };
    assert!(is_shared_channel_target(&discord));
    assert!(is_shared_channel_target(&telegram_group));
}

#[test]
fn proven_direct_chat_is_not_shared() {
    let target = ChannelReplyTarget {
        channel_type: ChannelType::Telegram,
        account_id: "bot".into(),
        chat_id: "123".into(),
        message_id: None,
        thread_id: None,
        ack_message_id: None,
    };
    assert!(!is_shared_channel_target(&target));
}

#[test]
fn only_operator_direct_turns_are_trusted() {
    let direct = ChannelReplyTarget {
        channel_type: ChannelType::Telegram,
        account_id: "bot".into(),
        chat_id: "123".into(),
        message_id: None,
        thread_id: None,
        ack_message_id: None,
    };
    let shared = ChannelReplyTarget {
        chat_id: "-123".into(),
        ..direct.clone()
    };

    assert!(is_trusted_channel_turn(
        ChannelSenderRole::Operator,
        &direct
    ));
    assert!(!is_trusted_channel_turn(ChannelSenderRole::Guest, &direct));
    assert!(!is_trusted_channel_turn(
        ChannelSenderRole::Operator,
        &shared
    ));
}

#[test]
fn command_authorization_matches_privilege_and_conversation_scope() {
    let direct = ChannelReplyTarget {
        channel_type: ChannelType::Telegram,
        account_id: "bot".into(),
        chat_id: "123".into(),
        message_id: None,
        thread_id: None,
        ack_message_id: None,
    };
    let shared = ChannelReplyTarget {
        chat_id: "-123".into(),
        ..direct.clone()
    };

    for command in moltis_channels::commands::all_commands() {
        let privilege = command.privilege();
        assert_eq!(
            is_channel_command_authorized(privilege, ChannelSenderRole::Guest, &direct),
            matches!(
                privilege,
                moltis_channels::commands::CommandPrivilege::Public
            ),
            "guest direct-chat authorization drifted for /{}",
            command.name
        );
        assert_eq!(
            is_channel_command_authorized(privilege, ChannelSenderRole::Operator, &shared),
            matches!(
                privilege,
                moltis_channels::commands::CommandPrivilege::Public
            ),
            "operator shared-chat authorization drifted for /{}",
            command.name
        );
        assert!(
            is_channel_command_authorized(privilege, ChannelSenderRole::Operator, &direct),
            "operator direct-chat authorization drifted for /{}",
            command.name
        );
    }
}

#[test]
fn untrusted_channel_context_denies_every_tool_and_private_context() {
    let mut params = serde_json::json!({
        "_tool_policy": {"allow": ["*"]},
        "_private_context": true,
    });

    apply_untrusted_channel_context(&mut params);

    assert_eq!(params["_tool_audience"], "public");
    assert_eq!(params["_tool_policy"]["deny"], serde_json::json!(["*"]));
    assert_eq!(params["_private_context"], false);
}

#[test]
fn untrusted_ceiling_defaults_match_the_unconfigured_behaviour() {
    let mut configured = serde_json::json!({});
    let mut unconfigured = serde_json::json!({});

    apply_untrusted_channel_context_with(
        &mut configured,
        UntrustedAudience::default(),
        UntrustedTools::default(),
    );
    apply_untrusted_channel_context(&mut unconfigured);

    assert_eq!(
        configured, unconfigured,
        "defaults must not change behaviour for an unconfigured account"
    );
}

/// The most permissive ceiling leaves the params carrying no tool policy at
/// all, so nothing on the request side can deny `exec`.
///
/// That is why the `/sh` guard in `dispatch_to_chat` stays tied to
/// `trusted_channel_turn` and not to this ceiling. `run_explicit_shell_command`
/// takes `exec` straight from the request registry, before the
/// `[tools.policy]` layers are consulted, so a guard that widened with the
/// ceiling would hand out an `exec` that no `deny` can take back.
#[test]
fn the_lifted_ceiling_cannot_deny_exec_on_the_request() {
    let mut params = serde_json::json!({});

    apply_untrusted_channel_context_with(
        &mut params,
        UntrustedAudience::Trusted,
        UntrustedTools::Policy,
    );

    assert!(params.get("_tool_policy").is_none());
    assert!(params.get("_tool_audience").is_none());
    assert_eq!(params["_private_context"], false);
}

#[test]
fn each_axis_is_lifted_on_its_own() {
    let mut both = serde_json::json!({ "_private_context": true });
    apply_untrusted_channel_context_with(
        &mut both,
        UntrustedAudience::Trusted,
        UntrustedTools::Policy,
    );
    assert!(both.get("_tool_audience").is_none());
    assert!(both.get("_tool_policy").is_none());
    assert_eq!(
        both["_private_context"], false,
        "owner-private context is never configurable for a channel turn"
    );

    let mut audience_only = serde_json::json!({});
    apply_untrusted_channel_context_with(
        &mut audience_only,
        UntrustedAudience::Trusted,
        UntrustedTools::default(),
    );
    assert_eq!(
        audience_only["_tool_policy"]["deny"],
        serde_json::json!(["*"]),
        "lifting the audience alone must still deny every tool by name"
    );

    let mut tools_only = serde_json::json!({});
    apply_untrusted_channel_context_with(
        &mut tools_only,
        UntrustedAudience::default(),
        UntrustedTools::Policy,
    );
    assert_eq!(
        tools_only["_tool_audience"], "public",
        "dropping the name policy alone must still hold the audience ceiling"
    );
}

#[test]
fn public_audience_tools_require_explicit_registration() {
    const REGISTRATION: &str = include_str!("../server/prepare_core/post_state.rs");

    assert_eq!(
        REGISTRATION.matches("register_public(").count(),
        3,
        "only reviewed tools should enter the public audience"
    );
    for tool in ["CalcTool", "WebSearchTool", "WebFetchTool"] {
        assert!(
            REGISTRATION.contains(tool),
            "{tool} must have an explicit public registration"
        );
    }
}

// ── unique_providers ───────────────────────────────────────────

/// Regression test for GitHub issue #637: providers must be deduplicated
/// even when duplicates are not adjacent in the model list. Prior to the
/// fix, a bare `Vec::dedup` left non-consecutive duplicates in place,
/// surfacing as duplicate Telegram `/model` inline keyboard buttons.
#[test]
fn unique_providers_dedups_non_adjacent() {
    let models = vec![
        serde_json::json!({"id": "gpt-4o", "provider": "openai"}),
        serde_json::json!({"id": "claude-3.5", "provider": "anthropic"}),
        serde_json::json!({"id": "gpt-4o-mini", "provider": "openai"}),
        serde_json::json!({"id": "gemini-pro", "provider": "google"}),
        serde_json::json!({"id": "claude-3.7", "provider": "anthropic"}),
    ];
    let providers = unique_providers(&models);
    assert_eq!(providers, vec!["anthropic", "google", "openai"]);
}

#[test]
fn unique_providers_sorted_alphabetically() {
    let models = vec![
        serde_json::json!({"id": "m1", "provider": "zeta"}),
        serde_json::json!({"id": "m2", "provider": "alpha"}),
        serde_json::json!({"id": "m3", "provider": "mu"}),
    ];
    assert_eq!(unique_providers(&models), vec!["alpha", "mu", "zeta"]);
}

#[test]
fn unique_providers_skips_entries_without_provider() {
    let models = vec![
        serde_json::json!({"id": "m1"}),
        serde_json::json!({"id": "m2", "provider": "openai"}),
        serde_json::json!({"id": "m3", "provider": serde_json::Value::Null}),
    ];
    assert_eq!(unique_providers(&models), vec!["openai"]);
}

#[test]
fn unique_providers_empty_input() {
    assert!(unique_providers(&[]).is_empty());
}

#[test]
fn attachable_session_filter_skips_archived_and_cron_sessions() {
    let archived = SessionEntry {
        id: "1".into(),
        key: "session:archived".into(),
        label: None,
        created_at: 0,
        updated_at: 0,
        message_count: 0,
        last_seen_message_count: 0,
        project_id: None,
        archived: true,
        worktree_branch: None,
        sandbox_enabled: None,
        sandbox_image: None,
        sandbox_backend: None,
        channel_binding: None,
        parent_session_key: None,
        fork_point: None,
        mcp_disabled: None,
        preview: None,
        agent_id: None,
        mode_id: None,
        model: None,
        node_id: None,
        external_agent_kind: None,
        external_session_id: None,
        version: 0,
    };
    let cron = SessionEntry {
        key: "cron:heartbeat".into(),
        archived: false,
        ..archived.clone()
    };
    let normal = SessionEntry {
        key: "session:normal".into(),
        archived: false,
        ..archived.clone()
    };

    assert!(!is_attachable_session(&cron));
    assert!(!is_attachable_session(&archived));
    assert!(is_attachable_session(&normal));
}

#[test]
fn format_attachable_sessions_shows_session_keys_when_labels_are_present() {
    let sessions = vec![
        SessionEntry {
            id: "1".into(),
            key: "main".into(),
            label: None,
            created_at: 0,
            updated_at: 0,
            message_count: 3,
            last_seen_message_count: 0,
            project_id: None,
            archived: false,
            worktree_branch: None,
            sandbox_enabled: None,
            sandbox_image: None,
            sandbox_backend: None,
            channel_binding: None,
            parent_session_key: None,
            fork_point: None,
            mcp_disabled: None,
            preview: None,
            agent_id: None,
            mode_id: None,
            model: None,
            node_id: None,
            external_agent_kind: None,
            external_session_id: None,
            version: 0,
        },
        SessionEntry {
            id: "2".into(),
            key: "session:abc".into(),
            label: Some("Build Fix".into()),
            created_at: 0,
            updated_at: 0,
            message_count: 12,
            last_seen_message_count: 0,
            project_id: None,
            archived: false,
            worktree_branch: None,
            sandbox_enabled: None,
            sandbox_image: None,
            sandbox_backend: None,
            channel_binding: None,
            parent_session_key: None,
            fork_point: None,
            mcp_disabled: None,
            preview: None,
            agent_id: None,
            mode_id: None,
            model: None,
            node_id: None,
            external_agent_kind: None,
            external_session_id: None,
            version: 0,
        },
    ];

    let rendered = format_attachable_sessions_list(&sessions, "session:abc");
    assert!(rendered.contains("1. main (3 msgs)"));
    assert!(rendered.contains("2. Build Fix [session:abc] (12 msgs) *"));
    assert!(rendered.contains("Use /attach N to move an existing session to this chat."));
}

#[test]
fn format_pending_approvals_renders_numbered_commands() {
    let approvals = vec![
        PendingApprovalView {
            id: "1".into(),
            command: "git status".into(),
            session_key: Some("session:a".into()),
        },
        PendingApprovalView {
            id: "2".into(),
            command: "rm -rf /tmp/build".into(),
            session_key: Some("session:a".into()),
        },
    ];

    let rendered = format_pending_approvals_list(&approvals);
    assert!(rendered.contains("1. `git status`"));
    assert!(rendered.contains("2. `rm -rf /tmp/build`"));
    assert!(rendered.contains("Use /approve N or /deny N."));
}

#[test]
fn channel_session_defaults_use_sender_override_for_group_commands() {
    let config = serde_json::json!({
        "model": "default-model",
        "agent_id": "default-agent",
        "channel_overrides": {
            "group-1": {
                "model": "channel-model",
                "agent_id": "channel-agent"
            }
        },
        "user_overrides": {
            "user-42": {
                "model": "user-model",
                "agent_id": "user-agent"
            }
        }
    });

    let defaults =
        resolve_channel_session_defaults_from_config(&config, "group-1", Some("user-42"));
    assert_eq!(defaults.model.as_deref(), Some("user-model"));
    assert_eq!(defaults.agent_id.as_deref(), Some("user-agent"));
}

#[test]
fn channel_session_defaults_use_chat_id_for_dm_commands() {
    let config = serde_json::json!({
        "model": "default-model",
        "agent_id": "default-agent",
        "user_overrides": {
            "dm-1": {
                "model": "dm-model",
                "agent_id": "dm-agent"
            }
        }
    });

    let defaults = resolve_channel_session_defaults_from_config(&config, "dm-1", Some("dm-1"));
    assert_eq!(defaults.model.as_deref(), Some("dm-model"));
    assert_eq!(defaults.agent_id.as_deref(), Some("dm-agent"));
}

#[test]
fn operator_denial_tells_the_owner_how_to_unlock_the_command() {
    let message = operator_denied_message("/sh", Some("400347514466992128"));

    assert!(message.starts_with("/sh is restricted to this bot's operators in direct chats."));
    assert!(message.contains("Shared chats"));
    assert!(
        message.contains("Settings → Channels"),
        "must point at the web UI: {message}"
    );
    assert!(
        message.contains("operators"),
        "must name the config field: {message}"
    );
    // Operator entries are exact platform IDs, so echoing the sender's own ID
    // back saves an owner from having to hunt for it.
    assert!(
        message.contains("Your sender ID here is: 400347514466992128"),
        "must echo the sender id: {message}"
    );
}

#[test]
fn operator_denial_omits_an_absent_or_blank_sender_id() {
    for sender_id in [None, Some(""), Some("   ")] {
        let message = operator_denied_message("/update", sender_id);
        assert!(
            !message.contains("Your sender ID"),
            "unattributed senders have no id to show: {message}"
        );
    }
}
