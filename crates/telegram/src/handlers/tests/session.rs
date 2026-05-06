use super::*;

#[test]
fn session_key_dm() {
    let key = build_session_key("bot1", &ChatType::Dm, "user123", None);
    assert_eq!(key, "telegram:bot1:dm:user123");
}

#[test]
fn session_key_group() {
    let key = build_session_key("bot1", &ChatType::Group, "user123", Some("-100999"));
    assert_eq!(key, "telegram:bot1:group:-100999");
}

#[test]
fn intercepts_shell_mode_control_commands_only() {
    assert!(should_intercept_slash_command("sh", "sh"));
    assert!(should_intercept_slash_command("sh", "sh on"));
    assert!(should_intercept_slash_command("sh", "sh off"));
    assert!(should_intercept_slash_command("sh", "sh exit"));
    assert!(should_intercept_slash_command("sh", "sh status"));
}

#[test]
fn shell_command_payloads_are_not_intercepted() {
    assert!(!should_intercept_slash_command("sh", "sh uname -a"));
    assert!(!should_intercept_slash_command("sh", "sh ls -la"));
}

/// Security: the OTP challenge message sent to the Telegram user must
/// NEVER contain the verification code.  The code should only be visible
/// to the admin in the web UI.  If this test fails, unauthenticated users
/// can self-approve without admin involvement.
#[test]
fn security_otp_challenge_message_does_not_contain_code() {
    let msg = OTP_CHALLENGE_MSG;

    // Must not contain any 6-digit numeric sequences (OTP codes are 6 digits).
    let has_six_digits = msg
        .as_bytes()
        .windows(6)
        .any(|w| w.iter().all(|b| b.is_ascii_digit()));
    assert!(
        !has_six_digits,
        "OTP challenge message must not contain a 6-digit code: {msg}"
    );

    // Must not contain format placeholders that could interpolate a code.
    assert!(
        !msg.contains("{code}") && !msg.contains("{0}"),
        "OTP challenge message must not contain format placeholders: {msg}"
    );

    // Must contain instructions pointing to the web UI.
    assert!(
        msg.contains("Channels") && msg.contains("Senders"),
        "OTP challenge message must tell the user where to find the code"
    );
}
