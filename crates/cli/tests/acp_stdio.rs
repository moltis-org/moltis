//! `moltis acp` speaks JSON-RPC on stdout, so nothing else may write there.
//!
//! The loopback tests in `moltis-acp` prove the protocol layer keeps its own
//! stream clean. They cannot catch the failure that actually bites: a `tracing`
//! subscriber, a stray `println!`, or a startup banner in the *binary* landing
//! on the same file descriptor. That needs the real process, so this drives it
//! end to end with logging turned all the way up.

#![cfg(feature = "acp")]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{
    io::Write,
    process::{Command, Stdio},
    time::Duration,
};

/// Reads `child`'s stdout to EOF after feeding it `input`, returning stdout and
/// stderr. Kills the child if it outlives the deadline.
fn run_acp(args: &[&str], input: &str) -> (String, String) {
    let temp = tempfile::tempdir().expect("temp dir");
    let mut child = Command::new(env!("CARGO_BIN_EXE_moltis"))
        .args(["--log-level", "trace"])
        // Keep the test off the developer's real ~/.moltis.
        .args(["--config-dir", &temp.path().to_string_lossy()])
        .args(["--data-dir", &temp.path().to_string_lossy()])
        .args(args)
        .env("RUST_LOG", "trace")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn moltis acp");

    {
        let mut stdin = child.stdin.take().expect("stdin");
        stdin.write_all(input.as_bytes()).expect("write request");
        stdin.flush().expect("flush");
        // Dropping stdin signals EOF, which ends the protocol loop.
    }

    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    loop {
        match child.try_wait().expect("wait") {
            Some(_) => break,
            None if std::time::Instant::now() >= deadline => {
                let _ = child.kill();
                panic!("moltis acp did not exit after stdin closed");
            },
            None => std::thread::sleep(Duration::from_millis(50)),
        }
    }

    let output = child.wait_with_output().expect("collect output");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

const INITIALIZE: &str = concat!(
    r#"{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":1}}"#,
    "\n",
);

#[test]
fn stdout_carries_only_protocol_framing_with_logging_at_trace() {
    let (stdout, stderr) = run_acp(&["acp", "--echo"], INITIALIZE);

    let mut frames = 0;
    for line in stdout.lines().filter(|line| !line.trim().is_empty()) {
        let frame: serde_json::Value = serde_json::from_str(line).unwrap_or_else(|error| {
            panic!("stdout must be pure JSON-RPC framing, got {line:?}: {error}")
        });
        assert_eq!(
            frame.get("jsonrpc").and_then(serde_json::Value::as_str),
            Some("2.0"),
            "unexpected frame on stdout: {line}"
        );
        frames += 1;
    }
    assert_eq!(frames, 1, "expected exactly one initialize response");

    // The logs still have to go somewhere, or the redirect is untested.
    assert!(
        stderr.contains("moltis starting"),
        "startup logging should be on stderr, got: {stderr}"
    );
}

#[test]
fn initialize_reports_moltis_as_the_agent() {
    let (stdout, _stderr) = run_acp(&["acp", "--echo"], INITIALIZE);
    let line = stdout
        .lines()
        .find(|line| !line.trim().is_empty())
        .expect("a response frame");
    let frame: serde_json::Value = serde_json::from_str(line).expect("valid JSON");
    assert_eq!(frame["result"]["agentInfo"]["name"], "moltis");
    assert_eq!(frame["result"]["protocolVersion"], 1);
}

#[test]
fn without_echo_the_command_fails_loudly_and_writes_nothing_to_stdout() {
    let (stdout, _stderr) = run_acp(&["acp"], INITIALIZE);
    assert!(
        stdout.trim().is_empty(),
        "a refused command must not emit protocol traffic: {stdout}"
    );
}
