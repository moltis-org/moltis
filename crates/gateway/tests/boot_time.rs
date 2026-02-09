//! Integration test: assert the gateway boots and responds within a time budget.
//!
//! This spawns a real `start_gateway` against a temp directory (in-memory-like
//! SQLite, no providers, no TLS) and measures wall-clock time until the `/health`
//! endpoint responds.

use std::time::{Duration, Instant};

/// Find a free TCP port by binding to :0 and reading the assigned port.
fn free_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap().port()
}

#[tokio::test]
async fn gateway_boots_under_five_seconds() {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().join("data");
    let config_dir = dir.path().join("config");
    std::fs::create_dir_all(&data_dir).unwrap();
    std::fs::create_dir_all(&config_dir).unwrap();

    // Write a minimal config so the gateway doesn't search standard locations.
    std::fs::write(
        config_dir.join("moltis.toml"),
        "[server]\nport = 0\n\n[auth]\ndisabled = true\n",
    )
    .unwrap();

    let port = free_port();
    let start = Instant::now();

    // Spawn the real gateway in a background task.
    let gateway_handle = tokio::spawn(async move {
        moltis_gateway::server::start_gateway(
            "127.0.0.1",
            port,
            true, // no_tls
            None,
            Some(config_dir),
            Some(data_dir),
            #[cfg(feature = "tailscale")]
            None,
        )
        .await
    });

    // Poll the health endpoint until it responds or we time out.
    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(200))
        .build()
        .unwrap();
    let url = format!("http://127.0.0.1:{port}/health");

    let deadline = start + Duration::from_secs(10);
    let mut healthy = false;
    while Instant::now() < deadline {
        if client.get(&url).send().await.is_ok() {
            healthy = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    let elapsed = start.elapsed();
    gateway_handle.abort();

    assert!(
        healthy,
        "gateway did not respond to /health within 10s (elapsed: {elapsed:?})"
    );
    assert!(
        elapsed < Duration::from_secs(5),
        "gateway boot took {elapsed:?}, expected < 5s"
    );

    eprintln!("gateway boot time: {elapsed:?}");
}
