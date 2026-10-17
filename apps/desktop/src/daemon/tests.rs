#![allow(clippy::unwrap_used, clippy::expect_used)]

use gpui::{AppContext, TestAppContext};

use super::*;

fn profile() -> ServerProfile {
    ServerProfile::direct(
        "test",
        Endpoint::parse("http://localhost:1", TlsTrust::System).unwrap(),
    )
}

#[gpui::test]
fn offline_send_does_not_start_an_rpc(cx: &mut TestAppContext) {
    let daemon = cx.new(|cx| Daemon::disconnected_for_test(profile(), cx));
    daemon.update(cx, |daemon, cx| {
        daemon.sessions.insert_draft("s");
        daemon.phase = Phase::Reconnecting {
            reason: "offline".into(),
            retry_in: Duration::from_secs(1),
        };
        assert!(daemon.send("draft".into(), cx).is_none());
        assert_eq!(daemon.seq, 0);
    });
}

#[gpui::test]
fn connected_update_invalidates_cached_history_and_inflight_loads(cx: &mut TestAppContext) {
    use moltis_protocol::{Extensions, Features, Policy, ServerInfo};

    let daemon = cx.new(|cx| Daemon::disconnected_for_test(profile(), cx));
    daemon.update(cx, |daemon, cx| {
        daemon.transcripts.insert_draft("s");
        let old = daemon.transcripts.begin_load("other").unwrap();
        daemon.connected_once = true;
        daemon.on_update(
            ClientUpdate::Connected {
                hello: Box::new(HelloOk {
                    r#type: "hello-ok".into(),
                    protocol: 4,
                    server: ServerInfo {
                        version: "test".into(),
                        commit: None,
                        host: None,
                        conn_id: "test".into(),
                    },
                    features: Features {
                        methods: vec![],
                        events: vec![],
                    },
                    snapshot: serde_json::json!({}),
                    canvas_host_url: None,
                    auth: None,
                    policy: Policy::default(),
                    extensions: Extensions::new(),
                }),
            },
            cx,
        );
        assert!(daemon.transcripts.needs_load("s"));
        assert!(daemon.transcripts.needs_load("other"));
        assert!(!daemon.transcripts.finish_load("other", old, &[]));
    });
}

#[gpui::test]
async fn disconnect_during_submission_reports_failure_to_composer(cx: &mut TestAppContext) {
    cx.background_executor.allow_parking();
    cx.update(|cx| runtime::Tokio::install(cx).unwrap());
    let daemon = cx.new(|cx| Daemon::disconnected_for_test(profile(), cx));
    let sent = daemon.update(cx, |daemon, cx| {
        let _guard = runtime::Tokio::handle(cx).enter();
        let (client, _updates) = ManagedClient::spawn(
            ConnectOptions::new(
                Endpoint::parse("http://localhost:1", TlsTrust::System).unwrap(),
                identity(),
            ),
            vec![],
        );
        // The UI has not received the disconnect update yet, but the client's
        // live connection is already gone when chat.send tries to use it.
        daemon.client = Some(client);
        daemon.phase = Phase::Connected;
        daemon.sessions.insert_draft("s");
        daemon.send("draft".into(), cx).unwrap()
    });
    assert!(!sent.await);
    daemon.update(cx, |daemon, _| daemon.stop());
}

#[cfg(feature = "ssh")]
fn ssh_profile() -> ServerProfile {
    ServerProfile {
        kind: ServerKind::Ssh {
            target: moltis_client::ssh::SshTarget {
                destination: "test".into(),
                remote_port: 18789,
                ssh_port: None,
                identity_file: None,
                tls: false,
            },
            tls: TlsTrust::System,
        },
        ..profile()
    }
}

#[cfg(feature = "ssh")]
#[gpui::test]
fn manual_retry_recreates_a_missing_ssh_transport(cx: &mut TestAppContext) {
    cx.update(|cx| runtime::Tokio::install(cx).unwrap());
    let daemon = cx.new(|cx| Daemon::disconnected_for_test(ssh_profile(), cx));
    daemon.update(cx, |daemon, cx| {
        let _guard = runtime::Tokio::handle(cx).enter();
        let (client, _updates) = ManagedClient::spawn(
            ConnectOptions::new(
                Endpoint::parse("http://localhost:1", TlsTrust::System).unwrap(),
                identity(),
            ),
            vec![],
        );
        daemon.client = Some(client);
        let generation = daemon.generation;
        daemon.retry_now(cx);
        assert_eq!(daemon.generation, generation + 1);
        assert!(
            daemon.client.is_none(),
            "old forwarded port must not be retried"
        );
        daemon.stop();
    });
}

#[cfg(feature = "ssh")]
#[gpui::test]
async fn automatic_retry_requests_a_new_ssh_transport(cx: &mut TestAppContext) {
    // Updates come from the real Tokio runtime, outside GPUI's test scheduler.
    cx.background_executor.allow_parking();
    cx.update(|cx| runtime::Tokio::install(cx).unwrap());
    let daemon = cx.new(|cx| Daemon::disconnected_for_test(ssh_profile(), cx));
    let task = daemon.update(cx, |_, cx| {
        cx.spawn(async move |this, cx| {
            Daemon::run_client(
                &this,
                Endpoint::parse("http://localhost:1", TlsTrust::System).unwrap(),
                None,
                1,
                cx,
            )
            .await
            .unwrap()
        })
    });
    assert!(
        task.await,
        "a dead forward must escape the WebSocket retry loop"
    );
    daemon.update(cx, |daemon, _| daemon.stop());
}
