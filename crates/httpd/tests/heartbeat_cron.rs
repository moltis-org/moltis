#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Integration tests for heartbeat cron job creation.

use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{Arc, Mutex},
};

use {
    async_trait::async_trait,
    futures::{SinkExt, StreamExt},
    serde_json::Value,
    tokio::net::TcpListener,
};

use {
    moltis_gateway::{
        auth,
        methods::MethodRegistry,
        services::{CronService, GatewayServices, ServiceResult},
        state::GatewayState,
    },
    moltis_httpd::server::{build_gateway_base, finalize_gateway_app},
};

use moltis_cron::types::{CronJob, CronJobCreate, CronJobPatch};

/// A mock cron service that stores jobs in memory for testing.
struct MockCronService {
    jobs: Arc<Mutex<HashMap<String, CronJob>>>,
}

impl MockCronService {
    fn new() -> Self {
        Self {
            jobs: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn get_job(&self, name: &str) -> Option<CronJob> {
        self.jobs
            .lock()
            .unwrap()
            .values()
            .find(|j| j.name == name)
            .cloned()
    }

    #[allow(dead_code)]
    fn job_count(&self) -> usize {
        self.jobs.lock().unwrap().len()
    }
}

#[async_trait]
impl CronService for MockCronService {
    async fn list(&self) -> ServiceResult {
        let jobs: Vec<CronJob> = self.jobs.lock().unwrap().values().cloned().collect();
        Ok(serde_json::to_value(jobs)?)
    }

    async fn status(&self) -> ServiceResult {
        Ok(serde_json::json!({ "running": true }))
    }

    async fn add(&self, params: Value) -> ServiceResult {
        let create: CronJobCreate =
            serde_json::from_value(params).map_err(|e| format!("invalid job spec: {e}"))?;

        let job = CronJob {
            id: create
                .id
                .clone()
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
            name: create.name.clone(),
            schedule: create.schedule.clone(),
            payload: create.payload.clone(),
            session_target: create.session_target.clone(),
            delete_after_run: create.delete_after_run,
            enabled: create.enabled,
            system: create.system,
            sandbox: create.sandbox.clone(),
            wake_mode: create.wake_mode,
            state: moltis_cron::types::CronJobState::default(),
            created_at_ms: 0,
            updated_at_ms: 0,
        };

        let id = job.id.clone();
        self.jobs.lock().unwrap().insert(id.clone(), job);

        // Return the created job
        let created = self.jobs.lock().unwrap().get(&id).cloned().unwrap();
        Ok(serde_json::to_value(created)?)
    }

    async fn update(&self, params: Value) -> ServiceResult {
        let id = params
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'id'".to_string())?;
        let patch: CronJobPatch = params
            .get("patch")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .ok_or_else(|| "missing or invalid 'patch'".to_string())?;

        let mut jobs = self.jobs.lock().unwrap();
        if let Some(job) = jobs.get_mut(id) {
            // Apply patches to the job
            if let Some(name) = patch.name {
                job.name = name;
            }
            if let Some(schedule) = patch.schedule {
                job.schedule = schedule;
            }
            if let Some(payload) = patch.payload {
                job.payload = payload;
            }
            if let Some(session_target) = patch.session_target {
                job.session_target = session_target;
            }
            if let Some(enabled) = patch.enabled {
                job.enabled = enabled;
            }
            if let Some(delete_after_run) = patch.delete_after_run {
                job.delete_after_run = delete_after_run;
            }
            if let Some(sandbox) = patch.sandbox {
                job.sandbox = sandbox;
            }
            if let Some(wake_mode) = patch.wake_mode {
                job.wake_mode = wake_mode;
            }
            Ok(serde_json::json!({ "updated": id }))
        } else {
            Err("job not found".into())
        }
    }

    async fn remove(&self, params: Value) -> ServiceResult {
        let id = params
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'id'".to_string())?;
        self.jobs.lock().unwrap().remove(id);
        Ok(serde_json::json!({ "removed": id }))
    }

    async fn run(&self, params: Value) -> ServiceResult {
        let id = params
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'id'".to_string())?;
        Ok(serde_json::json!({ "ran": id }))
    }

    async fn runs(&self, params: Value) -> ServiceResult {
        let _id = params
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'id'".to_string())?;
        let _limit = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(20) as usize;
        Ok(serde_json::json!([]))
    }
}

/// Start a test server with a mock cron service.
async fn start_test_server_with_mock_cron() -> (SocketAddr, Arc<MockCronService>) {
    let mock_cron = Arc::new(MockCronService::new());
    let resolved_auth = auth::resolve_auth(None, None);

    let services = GatewayServices::noop().with_cron(mock_cron.clone());

    let state = GatewayState::new(resolved_auth, services);
    let methods = Arc::new(MethodRegistry::new());

    #[cfg(feature = "push-notifications")]
    let (router, app_state) = build_gateway_base(state, methods, None, None);
    #[cfg(not(feature = "push-notifications"))]
    let (router, app_state) = build_gateway_base(state, methods, None);

    let router = router.merge(moltis_web::web_routes());
    let app = finalize_gateway_app(router, app_state, false);

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });

    (addr, mock_cron)
}

#[tokio::test]
async fn heartbeat_update_creates_cron_job_when_prompt_configured() {
    let (addr, mock_cron) = start_test_server_with_mock_cron().await;

    // Initially, no heartbeat job should exist
    assert!(mock_cron.get_job("__heartbeat__").is_none());

    // Connect via WebSocket
    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/ws/chat"))
        .await
        .expect("ws connect failed");

    // Send connect handshake
    let connect_frame = serde_json::json!({
        "type": "req",
        "id": "test-connect",
        "method": "connect",
        "params": {
            "minProtocol": 3,
            "maxProtocol": 4,
            "client": {
                "id": "test-client",
                "version": "0.0.1",
                "platform": "test",
                "mode": "operator"
            }
        }
    });
    ws.send(tokio_tungstenite::tungstenite::Message::Text(
        connect_frame.to_string().into(),
    ))
    .await
    .unwrap();

    // Consume hello-ok
    let _ = ws.next().await.unwrap().unwrap();

    // Call heartbeat.update with a custom prompt
    let heartbeat_update = serde_json::json!({
        "type": "req",
        "id": "hb-update-1",
        "method": "heartbeat.update",
        "params": {
            "enabled": true,
            "every": "30m",
            "prompt": "Test heartbeat prompt",
            "model": null,
            "ackMaxChars": 500,
            "activeHours": {
                "start": "00:00",
                "end": "23:59"
            },
            "deliver": false,
            "channel": null,
            "to": null,
            "sandboxEnabled": false,
            "sandboxImage": null
        }
    });
    ws.send(tokio_tungstenite::tungstenite::Message::Text(
        heartbeat_update.to_string().into(),
    ))
    .await
    .unwrap();

    // Read the response
    let msg = ws.next().await.unwrap().unwrap();
    let frame: Value = serde_json::from_str(msg.to_text().unwrap()).unwrap();

    // Should succeed
    assert_eq!(frame["type"], "res");
    assert_eq!(frame["id"], "hb-update-1");
    assert_eq!(
        frame["ok"],
        true,
        "heartbeat.update failed: {:?}",
        frame.get("payload")
    );

    // Now the heartbeat job should exist
    let job = mock_cron.get_job("__heartbeat__");
    assert!(job.is_some(), "heartbeat cron job should have been created");

    let job = job.unwrap();
    assert_eq!(job.name, "__heartbeat__");
    assert!(job.system);

    ws.close(None).await.ok();
}

#[tokio::test]
async fn heartbeat_update_does_not_create_job_without_prompt() {
    let (addr, mock_cron) = start_test_server_with_mock_cron().await;

    // Initially, no heartbeat job should exist
    assert!(mock_cron.get_job("__heartbeat__").is_none());

    // Connect via WebSocket
    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/ws/chat"))
        .await
        .expect("ws connect failed");

    // Send connect handshake
    let connect_frame = serde_json::json!({
        "type": "req",
        "id": "test-connect",
        "method": "connect",
        "params": {
            "minProtocol": 3,
            "maxProtocol": 4,
            "client": {
                "id": "test-client",
                "version": "0.0.1",
                "platform": "test",
                "mode": "operator"
            }
        }
    });
    ws.send(tokio_tungstenite::tungstenite::Message::Text(
        connect_frame.to_string().into(),
    ))
    .await
    .unwrap();

    // Consume hello-ok
    let _ = ws.next().await.unwrap().unwrap();

    // Call heartbeat.update WITHOUT a custom prompt (using default)
    let heartbeat_update = serde_json::json!({
        "type": "req",
        "id": "hb-update-1",
        "method": "heartbeat.update",
        "params": {
            "enabled": true,
            "every": "30m",
            "prompt": null,
            "model": null,
            "ackMaxChars": 500,
            "activeHours": {
                "start": "00:00",
                "end": "23:59"
            },
            "deliver": false,
            "channel": null,
            "to": null,
            "sandboxEnabled": false,
            "sandboxImage": null
        }
    });
    ws.send(tokio_tungstenite::tungstenite::Message::Text(
        heartbeat_update.to_string().into(),
    ))
    .await
    .unwrap();

    // Read the response
    let msg = ws.next().await.unwrap().unwrap();
    let frame: Value = serde_json::from_str(msg.to_text().unwrap()).unwrap();

    // Should succeed but not create job (no meaningful prompt)
    assert_eq!(frame["type"], "res");
    assert_eq!(frame["id"], "hb-update-1");
    assert_eq!(frame["ok"], true);

    // No job should have been created (no prompt configured)
    assert!(
        mock_cron.get_job("__heartbeat__").is_none(),
        "heartbeat cron job should NOT have been created without a prompt"
    );

    ws.close(None).await.ok();
}

#[tokio::test]
async fn heartbeat_update_updates_existing_job() {
    let (addr, mock_cron) = start_test_server_with_mock_cron().await;

    // Pre-create a heartbeat job using the Rust struct directly
    let create = CronJobCreate {
        id: Some("__heartbeat__".into()),
        name: "__heartbeat__".into(),
        schedule: moltis_cron::types::CronSchedule::Every {
            every_ms: 1800000,
            anchor_ms: None,
        },
        payload: moltis_cron::types::CronPayload::AgentTurn {
            message: "Old prompt".into(),
            model: None,
            timeout_secs: None,
            deliver: false,
            channel: None,
            to: None,
        },
        session_target: moltis_cron::types::SessionTarget::Named("heartbeat".into()),
        delete_after_run: false,
        enabled: true,
        system: true,
        sandbox: moltis_cron::types::CronSandboxConfig {
            enabled: false,
            image: None,
        },
        wake_mode: moltis_cron::types::CronWakeMode::NextHeartbeat,
    };
    let create_json = serde_json::to_value(create).unwrap();
    mock_cron.add(create_json).await.unwrap();

    // Verify initial state
    let initial_job = mock_cron.get_job("__heartbeat__").unwrap();
    assert!(initial_job.enabled);
    // Check that the job has the old prompt
    match &initial_job.payload {
        moltis_cron::types::CronPayload::AgentTurn { message, .. } => {
            assert_eq!(message, "Old prompt");
        },
        _ => panic!("Expected AgentTurn payload"),
    }

    // Connect via WebSocket
    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/ws/chat"))
        .await
        .expect("ws connect failed");

    // Send connect handshake
    let connect_frame = serde_json::json!({
        "type": "req",
        "id": "test-connect",
        "method": "connect",
        "params": {
            "minProtocol": 3,
            "maxProtocol": 4,
            "client": {
                "id": "test-client",
                "version": "0.0.1",
                "platform": "test",
                "mode": "operator"
            }
        }
    });
    ws.send(tokio_tungstenite::tungstenite::Message::Text(
        connect_frame.to_string().into(),
    ))
    .await
    .unwrap();

    // Consume hello-ok
    let _ = ws.next().await.unwrap().unwrap();

    // Call heartbeat.update to update the existing job
    let heartbeat_update = serde_json::json!({
        "type": "req",
        "id": "hb-update-1",
        "method": "heartbeat.update",
        "params": {
            "enabled": false,
            "every": "60m",
            "prompt": "Updated prompt",
            "model": null,
            "ackMaxChars": 500,
            "activeHours": {
                "start": "09:00",
                "end": "17:00"
            },
            "deliver": false,
            "channel": null,
            "to": null,
            "sandboxEnabled": false,
            "sandboxImage": null
        }
    });
    ws.send(tokio_tungstenite::tungstenite::Message::Text(
        heartbeat_update.to_string().into(),
    ))
    .await
    .unwrap();

    // Read the response
    let msg = ws.next().await.unwrap().unwrap();
    let frame: Value = serde_json::from_str(msg.to_text().unwrap()).unwrap();

    // Should succeed
    assert_eq!(frame["type"], "res");
    assert_eq!(frame["id"], "hb-update-1");
    assert_eq!(
        frame["ok"],
        true,
        "heartbeat.update failed: {:?}",
        frame.get("payload")
    );

    // Verify the job was updated (not created anew)
    let updated_job = mock_cron.get_job("__heartbeat__").unwrap();
    assert!(!updated_job.enabled);
    // Check that the job has the updated prompt
    match &updated_job.payload {
        moltis_cron::types::CronPayload::AgentTurn { message, .. } => {
            assert_eq!(message, "Updated prompt");
        },
        _ => panic!("Expected AgentTurn payload"),
    }

    ws.close(None).await.ok();
}

#[tokio::test]
async fn heartbeat_update_disabled_with_prompt_does_not_create_job() {
    let (addr, mock_cron) = start_test_server_with_mock_cron().await;

    assert!(mock_cron.get_job("__heartbeat__").is_none());

    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/ws/chat"))
        .await
        .expect("ws connect failed");

    let connect_frame = serde_json::json!({
        "type": "req",
        "id": "test-connect",
        "method": "connect",
        "params": {
            "minProtocol": 3,
            "maxProtocol": 4,
            "client": {
                "id": "test-client",
                "version": "0.0.1",
                "platform": "test",
                "mode": "operator"
            }
        }
    });
    ws.send(tokio_tungstenite::tungstenite::Message::Text(
        connect_frame.to_string().into(),
    ))
    .await
    .unwrap();

    let _ = ws.next().await.unwrap().unwrap();

    // Send heartbeat.update with enabled=false but a valid prompt
    let heartbeat_update = serde_json::json!({
        "type": "req",
        "id": "hb-update-1",
        "method": "heartbeat.update",
        "params": {
            "enabled": false,
            "every": "30m",
            "prompt": "Test heartbeat prompt",
            "model": null,
            "ackMaxChars": 500,
            "activeHours": {
                "start": "00:00",
                "end": "23:59"
            },
            "deliver": false,
            "channel": null,
            "to": null,
            "sandboxEnabled": false,
            "sandboxImage": null
        }
    });
    ws.send(tokio_tungstenite::tungstenite::Message::Text(
        heartbeat_update.to_string().into(),
    ))
    .await
    .unwrap();

    let msg = ws.next().await.unwrap().unwrap();
    let frame: Value = serde_json::from_str(msg.to_text().unwrap()).unwrap();

    assert_eq!(frame["type"], "res");
    assert_eq!(frame["id"], "hb-update-1");
    assert_eq!(frame["ok"], true);

    // No job should be created when disabled, even with a prompt
    assert!(
        mock_cron.get_job("__heartbeat__").is_none(),
        "heartbeat cron job should NOT be created when disabled"
    );

    ws.close(None).await.ok();
}
