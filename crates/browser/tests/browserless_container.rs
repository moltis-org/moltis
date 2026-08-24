//! Live compatibility tests for Browserless container images.
//!
//! These tests pull and run multi-gigabyte browser images, so they are ignored
//! during normal `cargo test` runs. Run them explicitly on a Docker host with:
//!
//! ```text
//! cargo test -p moltis-browser --test browserless_container -- --ignored --nocapture
//! ```

use std::{error::Error as StdError, path::Path, process::Command, time::Duration};

use {
    chromiumoxide::{Browser, handler::HandlerConfig},
    futures::StreamExt,
    moltis_browser::{
        container::{self, BrowserContainer, ContainerBackend},
        types::BrowserlessApiVersion,
    },
};

type TestResult<T = ()> = Result<T, Box<dyn StdError + Send + Sync>>;

const V1_IMAGE: &str = "docker.io/browserless/chrome:latest";
const V2_IMAGE: &str = "ghcr.io/browserless/chromium:v2.56.0";

fn container_host() -> String {
    std::env::var("MOLTIS_BROWSERLESS_TEST_HOST").unwrap_or_else(|_| "127.0.0.1".to_string())
}

async fn connect_browser(
    container: &BrowserContainer,
) -> TestResult<(Browser, tokio::task::JoinHandle<()>)> {
    let handler_config = HandlerConfig {
        request_timeout: Duration::from_secs(30),
        ..Default::default()
    };
    let (browser, mut handler) =
        match Browser::connect_with_config(container.websocket_url(), handler_config).await {
            Ok(connection) => connection,
            Err(error) => {
                let logs = docker_logs(container).unwrap_or_else(|log_error| {
                    format!("failed to read Browserless container logs: {log_error}")
                });
                return Err(std::io::Error::other(format!(
                    "failed to connect to Browserless: {error}; container logs:\n{logs}"
                ))
                .into());
            },
        };
    let handler_task = tokio::spawn(async move { while handler.next().await.is_some() {} });
    Ok((browser, handler_task))
}

async fn disconnect_browser(
    browser: Browser,
    mut handler_task: tokio::task::JoinHandle<()>,
) -> TestResult {
    handler_task.abort();
    let _ = (&mut handler_task).await;
    drop(browser);
    Ok(())
}

fn prepare_profile_dir(path: &Path) -> TestResult {
    std::fs::create_dir_all(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o777))?;
        if let Some(parent) = path.parent() {
            let permissions = std::fs::metadata(parent)?.permissions();
            if permissions.mode() & 0o001 == 0 {
                std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o777))?;
            }
        }
    }
    Ok(())
}

fn docker_logs(container: &BrowserContainer) -> TestResult<String> {
    let output = Command::new("docker")
        .args(["logs", container.id()])
        .output()?;
    if !output.status.success() {
        return Err(std::io::Error::other(format!(
            "docker logs failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ))
        .into());
    }
    Ok(format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    ))
}

#[tokio::test(flavor = "multi_thread")]
#[serial_test::serial]
#[ignore = "requires Docker and the Browserless v1 image"]
async fn browserless_v1_uses_legacy_connection_timeout() -> TestResult {
    container::ensure_image_with_backend(ContainerBackend::Docker, V1_IMAGE)?;
    let container_host = container_host();
    let container = BrowserContainer::start_with_backend(
        ContainerBackend::Docker,
        V1_IMAGE,
        "moltis-browserless-v1-test",
        1280,
        720,
        0,
        1_000,
        None,
        None,
        &container_host,
    )?;
    let (browser, handler_task) = connect_browser(&container).await?;
    let page = browser
        .new_page("data:text/html,<title>browserless-v1-ok</title>")
        .await?;

    assert_eq!(
        page.get_title().await?.as_deref(),
        Some("browserless-v1-ok")
    );
    tokio::time::sleep(Duration::from_secs(2)).await;
    let logs = docker_logs(&container)?;
    assert!(
        logs.contains("Job has timed-out, closing the WebSocket"),
        "Browserless v1 should apply CONNECTION_TIMEOUT and close the session"
    );

    handler_task.abort();
    drop(page);
    drop(browser);
    drop(container);
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
#[serial_test::serial]
#[ignore = "requires Docker and pulls the pinned Browserless v2 image"]
async fn browserless_v2_launches_and_reuses_profile_dir() -> TestResult {
    container::ensure_image_with_backend(ContainerBackend::Docker, V2_IMAGE)?;
    let container_host = container_host();
    let mut temp_dir = None;
    let profile_dir = if let Some(path) = std::env::var_os("MOLTIS_BROWSERLESS_TEST_PROFILE_DIR") {
        path.into()
    } else {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("profile");
        temp_dir = Some(dir);
        path
    };
    prepare_profile_dir(&profile_dir)?;

    let first_container = BrowserContainer::start_with_backend_and_api_version(
        ContainerBackend::Docker,
        V2_IMAGE,
        "moltis-browserless-v2-test",
        1280,
        720,
        0,
        120_000,
        Some(&profile_dir),
        None,
        &container_host,
        BrowserlessApiVersion::V2,
    )?;
    let (first_browser, first_handler) = connect_browser(&first_container).await?;
    let page = first_browser
        .new_page("data:text/html,<title>browserless-v2-ok</title>")
        .await?;
    assert_eq!(
        page.get_title().await?.as_deref(),
        Some("browserless-v2-ok")
    );

    let external_page = first_browser.new_page("about:blank").await?;
    external_page.goto("https://example.com").await?;
    assert_eq!(
        external_page.get_title().await?.as_deref(),
        Some("Example Domain")
    );
    drop(external_page);
    drop(page);
    disconnect_browser(first_browser, first_handler).await?;
    drop(first_container);
    assert!(profile_dir.join("Local State").is_file());

    let second_container = BrowserContainer::start_with_backend_and_api_version(
        ContainerBackend::Docker,
        V2_IMAGE,
        "moltis-browserless-v2-test",
        1280,
        720,
        0,
        120_000,
        Some(&profile_dir),
        None,
        &container_host,
        BrowserlessApiVersion::V2,
    )?;
    let (second_browser, second_handler) = connect_browser(&second_container).await?;
    let page = second_browser
        .new_page("data:text/html,<title>browserless-v2-restarted</title>")
        .await?;
    assert_eq!(
        page.get_title().await?.as_deref(),
        Some("browserless-v2-restarted")
    );

    drop(page);
    disconnect_browser(second_browser, second_handler).await?;
    drop(second_container);
    drop(temp_dir);
    Ok(())
}
