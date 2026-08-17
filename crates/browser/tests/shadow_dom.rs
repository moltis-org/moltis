//! Real-browser end-to-end tests for shadow-DOM piercing element lookups.
//!
//! Launches an installed Chromium-family browser in headless mode against a
//! local test page containing nested open shadow roots. When no suitable
//! browser is installed (e.g. bare CI runners) the test skips itself with a
//! message instead of failing.

use std::time::{Duration, Instant};

use tokio::io::{AsyncReadExt, AsyncWriteExt};

use moltis_browser::{
    BrowserAction, BrowserConfig, BrowserManager, BrowserRequest, detect::detect_browser,
};

/// Test page: one light-DOM button, one button inside an open shadow root, and
/// one input inside a shadow root nested one level deeper.
const SHADOW_PAGE: &str = r#"<!doctype html>
<html><body>
<button id="light">Light button</button>
<div id="host"></div>
<script>
  const host = document.getElementById('host');
  const root = host.attachShadow({ mode: 'open' });
  root.innerHTML = '<button id="shadow-btn">Shadow button</button><div id="nested"></div>';
  root.getElementById('shadow-btn').addEventListener('click', () => {
    window.__moltisClicked = true;
  });
  const nested = root.getElementById('nested').attachShadow({ mode: 'open' });
  nested.innerHTML = '<input type="text" placeholder="deep-input">';
</script>
</body></html>"#;

/// Serve `SHADOW_PAGE` on an ephemeral localhost port and return its URL.
async fn serve_shadow_page() -> String {
    let listener = match tokio::net::TcpListener::bind("127.0.0.1:0").await {
        Ok(listener) => listener,
        Err(e) => panic!("failed to bind test HTTP server: {e}"),
    };
    let addr = match listener.local_addr() {
        Ok(addr) => addr,
        Err(e) => panic!("failed to read test HTTP server address: {e}"),
    };

    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            tokio::spawn(async move {
                let mut buf = [0u8; 4096];
                let _ = stream.read(&mut buf).await;
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: \
                     {}\r\nConnection: close\r\n\r\n{SHADOW_PAGE}",
                    SHADOW_PAGE.len()
                );
                let _ = stream.write_all(response.as_bytes()).await;
                let _ = stream.shutdown().await;
            });
        }
    });

    format!("http://127.0.0.1:{}/", addr.port())
}

fn req(session_id: Option<String>, action: BrowserAction) -> BrowserRequest {
    BrowserRequest {
        session_id,
        action,
        timeout_ms: 30_000,
        sandbox: Some(false),
        browser: None,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn shadow_dom_lookups_pierce_open_roots() {
    let detection = detect_browser(None);
    // Renderless CDP browsers (Obscura, Lightpanda) don't implement enough of
    // the DOM for this test; require a pixel-rendering Chromium-family engine.
    let Some(chromium) = detection
        .browsers
        .iter()
        .find(|b| b.kind.supports_screenshots())
    else {
        eprintln!("skipping shadow_dom e2e: no Chromium-family browser installed");
        return;
    };

    let url = serve_shadow_page().await;

    let config = BrowserConfig {
        enabled: true,
        headless: true,
        persist_profile: false,
        chrome_path: Some(chromium.path.to_string_lossy().into_owned()),
        ..BrowserConfig::default()
    };
    let manager = BrowserManager::new(config);

    let nav = manager
        .handle_request(req(None, BrowserAction::Navigate { url }))
        .await;
    assert!(nav.success, "navigate failed: {:?}", nav.error);
    let sid = Some(nav.session_id.clone());

    // Snapshot must surface elements from the light DOM, an open shadow root,
    // and a shadow root nested inside another shadow root.
    let snap = manager
        .handle_request(req(sid.clone(), BrowserAction::Snapshot))
        .await;
    assert!(snap.success, "snapshot failed: {:?}", snap.error);
    let Some(snapshot) = snap.snapshot else {
        panic!("snapshot response missing DOM snapshot");
    };
    assert!(
        snapshot
            .elements
            .iter()
            .any(|e| e.text.as_deref() == Some("Light button")),
        "light-DOM button missing from snapshot: {:?}",
        snapshot.elements
    );
    let Some(shadow_btn) = snapshot
        .elements
        .iter()
        .find(|e| e.text.as_deref() == Some("Shadow button"))
    else {
        panic!(
            "open-shadow-root button missing from snapshot: {:?}",
            snapshot.elements
        );
    };
    let Some(deep_input) = snapshot
        .elements
        .iter()
        .find(|e| e.placeholder.as_deref() == Some("deep-input"))
    else {
        panic!(
            "nested-shadow-root input missing from snapshot: {:?}",
            snapshot.elements
        );
    };

    // Click resolves the shadow element's coordinates through the deep finder.
    let click = manager
        .handle_request(req(sid.clone(), BrowserAction::Click {
            ref_: shadow_btn.ref_,
        }))
        .await;
    assert!(click.success, "click failed: {:?}", click.error);
    let clicked = manager
        .handle_request(req(sid.clone(), BrowserAction::Evaluate {
            code: "window.__moltisClicked === true".into(),
        }))
        .await;
    assert_eq!(
        clicked.result,
        Some(serde_json::Value::Bool(true)),
        "click on shadow-root button did not fire its handler"
    );

    // Type focuses the nested shadow input through the deep finder.
    let typed = manager
        .handle_request(req(sid.clone(), BrowserAction::Type {
            ref_: deep_input.ref_,
            text: "hi".into(),
        }))
        .await;
    assert!(typed.success, "type failed: {:?}", typed.error);
    let value = manager
        .handle_request(req(sid.clone(), BrowserAction::Evaluate {
            code: "document.getElementById('host').shadowRoot.getElementById('nested')\
                   .shadowRoot.querySelector('input').value"
                .into(),
        }))
        .await;
    assert_eq!(
        value.result,
        Some(serde_json::Value::String("hi".into())),
        "typed text did not reach the nested shadow input"
    );

    // Wait with a selector that only matches inside the nested shadow root.
    let ok_wait = manager
        .handle_request(req(sid.clone(), BrowserAction::Wait {
            selector: Some(r#"[placeholder="deep-input"]"#.into()),
            ref_: None,
            timeout_ms: 5_000,
        }))
        .await;
    assert!(
        ok_wait.success,
        "wait on shadow selector failed: {:?}",
        ok_wait.error
    );

    // An invalid selector must fail fast, not poll until the wait timeout.
    let start = Instant::now();
    let bad_wait = manager
        .handle_request(req(sid.clone(), BrowserAction::Wait {
            selector: Some("[[not-a-selector".into()),
            ref_: None,
            timeout_ms: 10_000,
        }))
        .await;
    let elapsed = start.elapsed();
    assert!(!bad_wait.success, "invalid selector unexpectedly matched");
    let bad_err = bad_wait.error.unwrap_or_default();
    assert!(
        !bad_err.contains("element not found"),
        "invalid selector polled to timeout instead of failing fast: {bad_err}"
    );
    assert!(
        elapsed < Duration::from_secs(3),
        "invalid selector took {elapsed:?}; expected an immediate error"
    );

    // Stale refs from a previous snapshot must be cleared: hide the light
    // button, re-snapshot, and its old ref attribute must be gone.
    let hide = manager
        .handle_request(req(sid.clone(), BrowserAction::Evaluate {
            code: "document.getElementById('light').style.display = 'none'; true".into(),
        }))
        .await;
    assert!(hide.success, "hide failed: {:?}", hide.error);
    let resnap = manager
        .handle_request(req(sid.clone(), BrowserAction::Snapshot))
        .await;
    assert!(resnap.success, "re-snapshot failed: {:?}", resnap.error);
    let stale = manager
        .handle_request(req(sid.clone(), BrowserAction::Evaluate {
            code: "document.getElementById('light').hasAttribute('data-moltis-ref')".into(),
        }))
        .await;
    assert_eq!(
        stale.result,
        Some(serde_json::Value::Bool(false)),
        "hidden element kept its stale ref after re-snapshot"
    );

    let close = manager.handle_request(req(sid, BrowserAction::Close)).await;
    assert!(close.success, "close failed: {:?}", close.error);
}
