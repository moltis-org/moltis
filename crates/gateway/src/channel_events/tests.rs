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
        channel_binding: None,
        parent_session_key: None,
        fork_point: None,
        mcp_disabled: None,
        preview: None,
        agent_id: None,
        model: None,
        node_id: None,
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
            channel_binding: None,
            parent_session_key: None,
            fork_point: None,
            mcp_disabled: None,
            preview: None,
            agent_id: None,
            model: None,
            node_id: None,
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
            channel_binding: None,
            parent_session_key: None,
            fork_point: None,
            mcp_disabled: None,
            preview: None,
            agent_id: None,
            model: None,
            node_id: None,
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
