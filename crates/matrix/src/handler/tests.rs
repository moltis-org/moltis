use {
    super::{
        audio_format_from_metadata, checked_chat_type, extract_location_coordinates,
        first_selection, infer_audio_kind, infer_chat_type, is_bot_mentioned,
        location_dispatch_body, otp_request_message, parse_geo_uri, saved_audio_filename,
        should_auto_join_invite, should_ignore_initial_sync_history, update_utd_notice_window,
        utd_notice_message,
    },
    crate::{
        access,
        config::{AutoJoinPolicy, MatrixAccountConfig},
        state::{AccountState, AccountStateMap},
    },
    matrix_sdk::{
        Client,
        encryption::VerificationState,
        ruma::{
            events::room::message::{
                AudioMessageEventContent, LocationMessageEventContent, OriginalSyncRoomMessageEvent,
            },
            mxc_uri, owned_user_id,
            serde::Raw,
        },
    },
    moltis_channels::{
        gating::{DmPolicy, GroupPolicy},
        plugin::ChannelMessageKind,
    },
    moltis_common::types::ChatType,
    serde_json::json,
    std::{
        collections::HashMap,
        sync::{Arc, Mutex, RwLock, atomic::AtomicBool},
        time::{Duration, Instant},
    },
    tokio_util::sync::CancellationToken,
};

fn message_event(value: serde_json::Value) -> OriginalSyncRoomMessageEvent {
    Raw::from_json_string(value.to_string())
        .unwrap_or_else(|error| panic!("raw event: {error}"))
        .deserialize()
        .unwrap_or_else(|error| panic!("message event: {error}"))
}

fn account_state_map(initial_sync_complete: bool) -> AccountStateMap {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap_or_else(|error| panic!("matrix test runtime should build: {error}"));
    let client = runtime
        .block_on(
            Client::builder()
                .homeserver_url("https://matrix.example.com")
                .build(),
        )
        .unwrap_or_else(|error| panic!("matrix test client should build: {error}"));

    let mut accounts = HashMap::new();
    accounts.insert("test".into(), AccountState {
        account_id: "test".into(),
        config: MatrixAccountConfig::default(),
        client,
        message_log: None,
        event_sink: None,
        cancel: CancellationToken::new(),
        bot_user_id: "@bot:example.org".into(),
        ownership_startup_error: None,
        initial_sync_complete: AtomicBool::new(initial_sync_complete),
        pending_identity_reset: Mutex::new(None),
        otp: Mutex::new(moltis_channels::otp::OtpState::new(300)),
        verification: Mutex::new(Default::default()),
        oidc_pending: Mutex::new(None),
    });

    Arc::new(RwLock::new(accounts))
}

#[test]
fn bot_mention_detected_from_intentional_mentions() {
    let bot_user_id = owned_user_id!("@bot:example.org");
    let event = message_event(json!({
        "type": "m.room.message",
        "event_id": "$1",
        "room_id": "!room:example.org",
        "sender": "@alice:example.org",
        "origin_server_ts": 1,
        "content": {
            "msgtype": "m.text",
            "body": "hello",
            "m.mentions": {
                "user_ids": ["@bot:example.org"]
            }
        }
    }));

    assert!(is_bot_mentioned(&event, &bot_user_id, "hello"));
}

#[test]
fn bot_mention_detected_from_literal_mxid_fallback() {
    let bot_user_id = owned_user_id!("@bot:example.org");
    let event = message_event(json!({
        "type": "m.room.message",
        "event_id": "$1",
        "room_id": "!room:example.org",
        "sender": "@alice:example.org",
        "origin_server_ts": 1,
        "content": {
            "msgtype": "m.text",
            "body": "@bot:example.org hello"
        }
    }));

    assert!(is_bot_mentioned(
        &event,
        &bot_user_id,
        "@bot:example.org hello"
    ));
}

#[test]
fn room_mention_counts_as_mention() {
    let bot_user_id = owned_user_id!("@bot:example.org");
    let event = message_event(json!({
        "type": "m.room.message",
        "event_id": "$1",
        "room_id": "!room:example.org",
        "sender": "@alice:example.org",
        "origin_server_ts": 1,
        "content": {
            "msgtype": "m.text",
            "body": "@room hello",
            "m.mentions": {
                "room": true
            }
        }
    }));

    assert!(is_bot_mentioned(&event, &bot_user_id, "@room hello"));
}

#[test]
fn initial_sync_history_is_ignored_until_catch_up_finishes() {
    let pending_accounts = account_state_map(false);
    let live_accounts = account_state_map(true);

    assert!(should_ignore_initial_sync_history(
        &pending_accounts,
        "test"
    ));
    assert!(!should_ignore_initial_sync_history(&live_accounts, "test"));
}

#[test]
fn infer_chat_type_prefers_explicit_direct_flag() {
    assert_eq!(infer_chat_type(true, 5, 5), ChatType::Dm);
}

#[test]
fn infer_chat_type_treats_two_party_rooms_as_dms() {
    assert_eq!(infer_chat_type(false, 2, 2), ChatType::Dm);
    assert_eq!(infer_chat_type(false, 2, 1), ChatType::Dm);
    assert_eq!(infer_chat_type(false, 1, 2), ChatType::Dm);
}

#[test]
fn infer_chat_type_keeps_larger_rooms_as_groups() {
    assert_eq!(infer_chat_type(false, 3, 3), ChatType::Group);
}

#[test]
fn checked_chat_type_rejects_unallowlisted_dm_poll_sender() {
    let cfg = MatrixAccountConfig {
        dm_policy: DmPolicy::Allowlist,
        user_allowlist: vec!["@alice:example.org".into()],
        ..Default::default()
    };

    let result = checked_chat_type(
        &cfg,
        "@mallory:example.org",
        "!dm:example.org",
        true,
        2,
        2,
        true,
    );

    assert_eq!(result, Err(access::AccessDenied::NotOnAllowlist));
}

#[test]
fn checked_chat_type_allows_group_poll_response_without_fresh_mention() {
    let cfg = MatrixAccountConfig {
        room_policy: GroupPolicy::Allowlist,
        room_allowlist: vec!["!ops:example.org".into()],
        ..Default::default()
    };

    let result = checked_chat_type(
        &cfg,
        "@alice:example.org",
        "!ops:example.org",
        false,
        3,
        3,
        true,
    );

    assert_eq!(result, Ok(ChatType::Group));
}

#[test]
fn first_selection_returns_the_first_callback_choice() {
    let selections = vec!["agent_switch:2".to_string(), "agent_switch:3".to_string()];

    assert_eq!(
        first_selection(&selections),
        Some("agent_switch:2".to_string())
    );
    assert_eq!(first_selection(&[]), None);
}

#[test]
fn auto_join_policy_always_joins_invites() {
    let cfg = MatrixAccountConfig {
        auto_join: AutoJoinPolicy::Always,
        room_policy: GroupPolicy::Open,
        ..Default::default()
    };

    assert!(should_auto_join_invite(
        &cfg,
        "@alice:example.org",
        "!ops:example.org",
        false,
    ));
}

#[test]
fn auto_join_policy_off_ignores_invites() {
    let cfg = MatrixAccountConfig {
        auto_join: AutoJoinPolicy::Off,
        room_policy: GroupPolicy::Open,
        ..Default::default()
    };

    assert!(!should_auto_join_invite(
        &cfg,
        "@alice:example.org",
        "!ops:example.org",
        false,
    ));
}

#[test]
fn auto_join_allowlist_uses_existing_user_and_room_allowlists() {
    let cfg = MatrixAccountConfig {
        auto_join: AutoJoinPolicy::Allowlist,
        room_policy: GroupPolicy::Open,
        user_allowlist: vec!["@alice:example.org".into()],
        room_allowlist: vec!["!ops:example.org".into()],
        ..Default::default()
    };

    assert!(should_auto_join_invite(
        &cfg,
        "@alice:example.org",
        "!other:example.org",
        false,
    ));
    assert!(should_auto_join_invite(
        &cfg,
        "@bob:example.org",
        "!ops:example.org",
        false,
    ));
    assert!(!should_auto_join_invite(
        &cfg,
        "@mallory:example.org",
        "!other:example.org",
        false,
    ));
}

#[test]
fn auto_join_never_bypasses_room_allowlist() {
    let cfg = MatrixAccountConfig {
        auto_join: AutoJoinPolicy::Always,
        room_policy: GroupPolicy::Allowlist,
        room_allowlist: vec!["!ops:example.org".into()],
        user_allowlist: vec!["@alice:example.org".into()],
        ..Default::default()
    };

    assert!(should_auto_join_invite(
        &cfg,
        "@mallory:example.org",
        "!ops:example.org",
        false,
    ));
    assert!(!should_auto_join_invite(
        &cfg,
        "@alice:example.org",
        "!private:example.org",
        false,
    ));
}

#[test]
fn auto_join_respects_disabled_room_policy() {
    let cfg = MatrixAccountConfig {
        auto_join: AutoJoinPolicy::Always,
        room_policy: GroupPolicy::Disabled,
        ..Default::default()
    };

    assert!(!should_auto_join_invite(
        &cfg,
        "@alice:example.org",
        "!ops:example.org",
        false,
    ));
}

#[test]
fn dm_invite_uses_dm_policy_not_room_policy() {
    let cfg = MatrixAccountConfig {
        auto_join: AutoJoinPolicy::Always,
        dm_policy: DmPolicy::Open,
        room_policy: GroupPolicy::Disabled,
        ..Default::default()
    };

    // DM invite should succeed even when room_policy is disabled
    assert!(should_auto_join_invite(
        &cfg,
        "@alice:example.org",
        "!dm:example.org",
        true,
    ));
    // Group invite should still be blocked
    assert!(!should_auto_join_invite(
        &cfg,
        "@alice:example.org",
        "!dm:example.org",
        false,
    ));
}

#[test]
fn dm_invite_respects_disabled_dm_policy() {
    let cfg = MatrixAccountConfig {
        auto_join: AutoJoinPolicy::Always,
        dm_policy: DmPolicy::Disabled,
        room_policy: GroupPolicy::Open,
        ..Default::default()
    };

    assert!(!should_auto_join_invite(
        &cfg,
        "@alice:example.org",
        "!dm:example.org",
        true,
    ));
}

#[test]
fn dm_invite_allowlist_checks_user_allowlist() {
    let cfg = MatrixAccountConfig {
        auto_join: AutoJoinPolicy::Always,
        dm_policy: DmPolicy::Allowlist,
        user_allowlist: vec!["@alice:example.org".into()],
        ..Default::default()
    };

    assert!(should_auto_join_invite(
        &cfg,
        "@alice:example.org",
        "!dm:example.org",
        true,
    ));
    assert!(!should_auto_join_invite(
        &cfg,
        "@mallory:example.org",
        "!dm:example.org",
        true,
    ));
}

#[test]
fn dm_invite_open_policy_allows_any_user() {
    let cfg = MatrixAccountConfig {
        auto_join: AutoJoinPolicy::Always,
        dm_policy: DmPolicy::Open,
        ..Default::default()
    };

    assert!(should_auto_join_invite(
        &cfg,
        "@stranger:example.org",
        "!dm:example.org",
        true,
    ));
}

#[test]
fn dm_invite_still_respects_auto_join_off() {
    let cfg = MatrixAccountConfig {
        auto_join: AutoJoinPolicy::Off,
        dm_policy: DmPolicy::Open,
        ..Default::default()
    };

    assert!(!should_auto_join_invite(
        &cfg,
        "@alice:example.org",
        "!dm:example.org",
        true,
    ));
}

#[test]
fn parse_geo_uri_accepts_location_with_accuracy_suffix() {
    assert_eq!(
        parse_geo_uri("geo:51.5008,-0.1247;u=35"),
        Some((51.5008, -0.1247))
    );
}

#[test]
fn extract_location_coordinates_accepts_geo_text() {
    assert_eq!(
        extract_location_coordinates("geo:38.7223,-9.1393"),
        Some((38.7223, -9.1393))
    );
}

#[test]
fn extract_location_coordinates_accepts_map_link() {
    assert_eq!(
        extract_location_coordinates("https://maps.apple.com/?ll=34.0522,-118.2437&z=12"),
        Some((34.0522, -118.2437))
    );
}

#[test]
fn extract_location_coordinates_accepts_plain_pair() {
    assert_eq!(
        extract_location_coordinates("48.8566, 2.3522"),
        Some((48.8566, 2.3522))
    );
}

#[test]
fn audio_format_prefers_mimetype_then_filename() {
    assert_eq!(
        audio_format_from_metadata(Some("audio/webm"), Some("voice.ogg")),
        "webm"
    );
    assert_eq!(
        audio_format_from_metadata(None, Some("voice-note.opus")),
        "opus"
    );
    assert_eq!(audio_format_from_metadata(None, None), "ogg");
}

#[test]
fn infer_audio_kind_treats_opus_as_voice() {
    let audio = AudioMessageEventContent::plain(
        "voice-note.opus".to_string(),
        mxc_uri!("mxc://example.org/voice").to_owned(),
    );

    assert!(matches!(
        infer_audio_kind(&audio),
        ChannelMessageKind::Voice
    ));
}

#[test]
fn saved_audio_filename_uses_cleaned_original_name() {
    assert_eq!(
        saved_audio_filename("$event:example.org", Some("nested/path voice"), None, "ogg"),
        "path_voice.ogg"
    );
}

#[test]
fn location_dispatch_body_includes_coordinates() {
    let location = LocationMessageEventContent::new(
        "Meet me here".to_string(),
        "geo:38.7223,-9.1393".to_string(),
    );

    assert_eq!(
        location_dispatch_body(&location, 38.7223, -9.1393),
        "Meet me here\n\nShared location: 38.7223, -9.1393"
    );
}

#[test]
fn utd_notice_window_throttles_repeated_notices() {
    let mut notices = HashMap::new();
    let now = Instant::now();

    assert!(update_utd_notice_window(
        &mut notices,
        "!room:example.org",
        now
    ));
    assert!(!update_utd_notice_window(
        &mut notices,
        "!room:example.org",
        now + Duration::from_secs(60),
    ));
    assert!(update_utd_notice_window(
        &mut notices,
        "!room:example.org",
        now + Duration::from_secs(301),
    ));
}

#[test]
fn utd_notice_message_guides_verification_for_unverified_devices() {
    assert!(utd_notice_message(VerificationState::Unverified).contains("verify show"));
    assert!(utd_notice_message(VerificationState::Unverified).contains("same Matrix chat"));
    assert!(utd_notice_message(VerificationState::Unknown).contains("verification"));
    assert!(utd_notice_message(VerificationState::Verified).contains("room keys"));
}

#[test]
fn otp_request_message_does_not_leak_codes() {
    let message = otp_request_message();

    assert!(message.contains("please enter the verification code"));
    assert!(message.contains("Channels -> Senders"));
    assert!(!message.contains("approve code"));
    assert!(!message.contains("enter it here"));
}

#[test]
fn help_text_lists_all_commands() {
    use super::HELP_TEXT;
    for cmd in [
        "/new",
        "/sessions",
        "/agent",
        "/model",
        "/sandbox",
        "/sh",
        "/clear",
        "/compact",
        "/context",
        "/peek",
        "/stop",
        "/help",
    ] {
        assert!(HELP_TEXT.contains(cmd), "HELP_TEXT should mention {cmd}");
    }
}

#[test]
fn slash_prefix_detection_matches_commands() {
    let body = "/new";
    assert!(body.strip_prefix('/').is_some());

    let body = "/compact some args";
    if let Some(cmd) = body.strip_prefix('/') {
        assert!(cmd.starts_with("compact"));
    } else {
        panic!("expected slash-prefixed command");
    }

    // Not a command
    let body = "hello world";
    assert!(body.strip_prefix('/').is_none());
}
