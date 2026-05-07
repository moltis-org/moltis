//! CLI commands for voice call management.

use {anyhow::Result, clap::Subcommand};

#[derive(Subcommand)]
pub enum VoiceCallAction {
    /// Initiate an outbound phone call.
    Call {
        /// Destination phone number (E.164 format, e.g. +15551234567).
        #[arg(long)]
        to: String,
        /// Message to speak when the call connects.
        #[arg(short, long)]
        message: Option<String>,
        /// Call mode: notify (one-way) or conversation (multi-turn).
        #[arg(long, default_value = "conversation")]
        mode: String,
        /// Gateway HTTP URL.
        #[arg(long, default_value = "http://localhost:9090")]
        host: String,
        /// API key for authenticating with the gateway.
        #[arg(long, env = "MOLTIS_API_KEY")]
        api_key: Option<String>,
    },
    /// Check the status of an active call.
    Status {
        /// Call ID to check.
        call_id: Option<String>,
        /// Gateway HTTP URL.
        #[arg(long, default_value = "http://localhost:9090")]
        host: String,
        /// API key for authenticating with the gateway.
        #[arg(long, env = "MOLTIS_API_KEY")]
        api_key: Option<String>,
    },
    /// End an active call.
    End {
        /// Call ID to hang up.
        call_id: String,
        /// Gateway HTTP URL.
        #[arg(long, default_value = "http://localhost:9090")]
        host: String,
        /// API key for authenticating with the gateway.
        #[arg(long, env = "MOLTIS_API_KEY")]
        api_key: Option<String>,
    },
    /// Verify telephony setup (credentials, webhook reachability).
    Setup,
}

pub async fn handle_voicecall(action: VoiceCallAction) -> Result<()> {
    match action {
        VoiceCallAction::Call {
            to,
            message,
            mode,
            host,
            api_key,
        } => {
            let result = gateway_rpc(
                &host,
                api_key.as_deref(),
                "voicecall.initiate",
                serde_json::json!({
                    "to": to,
                    "message": message,
                    "mode": mode,
                }),
            )
            .await?;

            println!(
                "Call initiated: {}",
                result
                    .get("call_id")
                    .and_then(|value| value.as_str())
                    .unwrap_or("(unknown)")
            );
            if let Some(from) = result.get("from").and_then(|value| value.as_str()) {
                println!("From: {from}");
            }
            if let Some(to) = result.get("to").and_then(|value| value.as_str()) {
                println!("To:   {to}");
            }
            Ok(())
        },
        VoiceCallAction::Status {
            call_id,
            host,
            api_key,
        } => {
            let result = gateway_rpc(
                &host,
                api_key.as_deref(),
                "voicecall.status",
                serde_json::json!({}),
            )
            .await?;
            if let Some(call_id) = call_id {
                for account in result
                    .get("accounts")
                    .and_then(|value| value.as_array())
                    .into_iter()
                    .flatten()
                {
                    for call in account
                        .get("active_calls")
                        .and_then(|value| value.as_array())
                        .into_iter()
                        .flatten()
                    {
                        if call.get("call_id").and_then(|value| value.as_str())
                            == Some(call_id.as_str())
                        {
                            println!("{}", serde_json::to_string_pretty(call)?);
                            return Ok(());
                        }
                    }
                }
                anyhow::bail!("call {call_id} not found");
            }
            println!("{}", serde_json::to_string_pretty(&result)?);
            Ok(())
        },
        VoiceCallAction::End {
            call_id,
            host,
            api_key,
        } => {
            let result = gateway_rpc(
                &host,
                api_key.as_deref(),
                "voicecall.end",
                serde_json::json!({ "call_id": call_id }),
            )
            .await?;
            println!("{}", serde_json::to_string_pretty(&result)?);
            Ok(())
        },
        VoiceCallAction::Setup => {
            println!("Telephony setup check:");
            println!("  1. Open Settings > Phone and choose a provider");
            println!("  2. Save provider credentials and a caller phone number");
            println!("  3. Set webhook_url or server.external_url to a public HTTPS URL");
            println!("  4. Start the gateway: moltis gateway");
            println!("  5. Use: moltis voice-call call --to +15559876543");
            Ok(())
        },
    }
}

async fn gateway_rpc(
    host: &str,
    api_key: Option<&str>,
    method: &str,
    params: serde_json::Value,
) -> Result<serde_json::Value> {
    let url = format!("{}/api/rpc", host.trim_end_matches('/'));
    let body = serde_json::json!({
        "method": method,
        "params": params,
    });

    let client = reqwest::Client::new();
    let mut req = client.post(&url).json(&body);
    if let Some(key) = api_key {
        req = req.header("Authorization", format!("Bearer {key}"));
    }

    let resp = req
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("cannot reach gateway at {host}: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("gateway returned {status}: {body}");
    }

    let result: serde_json::Value = resp.json().await?;
    if result.get("ok").and_then(|value| value.as_bool()) == Some(false) {
        let message = result
            .get("error")
            .and_then(|error| error.get("message"))
            .and_then(|value| value.as_str())
            .unwrap_or("unknown RPC error");
        anyhow::bail!("RPC error: {message}");
    }

    Ok(result.get("payload").cloned().unwrap_or(result))
}
