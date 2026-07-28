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
    io::{BufRead, BufReader, Read, Write},
    process::{Command, Stdio},
    sync::mpsc,
    time::Duration,
};

/// Reads `child`'s stdout to EOF after feeding it `input`, returning stdout and
/// stderr. Kills the child if it outlives the deadline.
fn run_acp_with_rust_log(args: &[&str], input: &str, rust_log: &str) -> (String, String) {
    let temp = tempfile::tempdir().expect("temp dir");
    std::fs::write(
        temp.path().join("moltis.toml"),
        "[providers]\noffered = [\"test-disabled\"]\n\n[code_index]\nenabled = false\n",
    )
    .expect("write isolated config");
    let mut child = Command::new(env!("CARGO_BIN_EXE_moltis"))
        .args(["--log-level", "trace"])
        // Keep the test off the developer's real ~/.moltis.
        .args(["--config-dir", &temp.path().to_string_lossy()])
        .args(["--data-dir", &temp.path().to_string_lossy()])
        .args(args)
        .env("HOME", temp.path())
        .env("XDG_CONFIG_HOME", temp.path())
        .env("RUST_LOG", rust_log)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn moltis acp");
    let child_stdout = child.stdout.take().expect("stdout");
    let (first_frame_tx, first_frame_rx) = mpsc::sync_channel(1);
    let stdout_reader = std::thread::spawn(move || {
        let mut reader = BufReader::new(child_stdout);
        let mut output = String::new();
        reader.read_line(&mut output).expect("read first frame");
        let _ = first_frame_tx.send(output.clone());
        reader.read_to_string(&mut output).expect("read stdout");
        output
    });
    let mut child_stderr = child.stderr.take().expect("stderr");
    let stderr_reader = std::thread::spawn(move || {
        let mut output = String::new();
        child_stderr
            .read_to_string(&mut output)
            .expect("read stderr");
        output
    });

    let mut stdin = child.stdin.take().expect("stdin");
    stdin.write_all(input.as_bytes()).expect("write request");
    stdin.flush().expect("flush");
    // A debug binary links the complete gateway and can be CPU-starved when
    // nextest runs the full workspace concurrently. This is a startup deadline,
    // not a sleep; focused runs normally answer in about ten seconds.
    match first_frame_rx.recv_timeout(Duration::from_secs(180)) {
        Ok(frame) if !frame.trim().is_empty() => {},
        result => {
            let _ = child.kill();
            let _ = child.wait();
            panic!("moltis acp did not return a response frame: {result:?}");
        },
    }
    // The client closes only after receiving its response; dropping stdin then
    // signals EOF and exercises graceful backend cleanup.
    drop(stdin);

    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    let status = loop {
        match child.try_wait().expect("wait") {
            Some(status) => break status,
            None if std::time::Instant::now() >= deadline => {
                let _ = child.kill();
                panic!("moltis acp did not exit after stdin closed");
            },
            None => std::thread::sleep(Duration::from_millis(50)),
        }
    };
    assert!(status.success(), "moltis acp exited with {status}");

    (
        stdout_reader.join().expect("stdout reader"),
        stderr_reader.join().expect("stderr reader"),
    )
}

fn run_acp(args: &[&str], input: &str) -> (String, String) {
    run_acp_with_rust_log(args, input, "trace")
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
fn protocol_payloads_are_not_logged_even_at_trace() {
    const SECRET: &str = "ACP-LOG-SENTINEL-DO-NOT-PRINT";
    let input = format!("not-json-{SECRET}\n{INITIALIZE}");
    let (_stdout, stderr) = run_acp_with_rust_log(
        &["acp", "--echo"],
        &input,
        "agent_client_protocol::rpc=trace",
    );
    assert!(
        !stderr.contains(SECRET),
        "malformed protocol payload leaked to stderr: {stderr}"
    );
}

#[test]
fn without_echo_boots_the_real_backend_and_serves_protocol() {
    let (stdout, _stderr) = run_acp(&["acp"], INITIALIZE);
    let frame: serde_json::Value = serde_json::from_str(stdout.trim()).expect("valid JSON-RPC");
    assert_eq!(frame["result"]["agentInfo"]["name"], "moltis");
    assert_eq!(frame["result"]["agentCapabilities"]["loadSession"], true);
}
