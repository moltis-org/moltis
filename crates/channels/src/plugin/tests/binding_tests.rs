use super::*;

#[test]
fn resolve_session_channel_binding_classifies_special_sessions() {
    let heartbeat = resolve_session_channel_binding("cron:heartbeat", None)
        .unwrap_or_else(|error| panic!("heartbeat binding should resolve: {error}"));
    assert_eq!(heartbeat.surface.as_deref(), Some("heartbeat"));
    assert_eq!(heartbeat.session_kind.as_deref(), Some("cron"));

    let cron = resolve_session_channel_binding("cron:nightly", None)
        .unwrap_or_else(|error| panic!("cron binding should resolve: {error}"));
    assert_eq!(cron.surface.as_deref(), Some("cron"));
    assert_eq!(cron.session_kind.as_deref(), Some("cron"));

    let web = resolve_session_channel_binding("main", None)
        .unwrap_or_else(|error| panic!("web binding should resolve: {error}"));
    assert_eq!(web.surface.as_deref(), Some("web"));
    assert_eq!(web.session_kind.as_deref(), Some("web"));
}

#[test]
fn resolve_session_channel_binding_extracts_channel_target() {
    let binding_json = serde_json::to_string(&ChannelReplyTarget {
        ack_message_id: None,
        channel_type: ChannelType::Telegram,
        account_id: "bot-main".into(),
        chat_id: "-100123".into(),
        message_id: Some("11".into()),
        thread_id: None,
    })
    .unwrap_or_else(|error| panic!("serialize binding: {error}"));

    let binding = resolve_session_channel_binding("telegram:bot-main:-100123", Some(&binding_json))
        .unwrap_or_else(|error| panic!("channel binding should resolve: {error}"));

    assert_eq!(binding.surface.as_deref(), Some("telegram"));
    assert_eq!(binding.session_kind.as_deref(), Some("channel"));
    assert_eq!(binding.channel_type.as_deref(), Some("telegram"));
    assert_eq!(binding.account_id.as_deref(), Some("bot-main"));
    assert_eq!(binding.chat_id.as_deref(), Some("-100123"));
    assert_eq!(binding.chat_type.as_deref(), Some("channel_or_supergroup"));
}

#[test]
fn resolve_session_channel_binding_returns_error_for_invalid_json() {
    let error = resolve_session_channel_binding("telegram:bot-main:-100123", Some("{not-json"))
        .err()
        .unwrap_or_else(|| panic!("invalid binding json should fail"));
    assert!(error.is_syntax());
}
