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
    },
    /// Check the status of an active call.
    Status {
        /// Call ID to check.
        call_id: String,
    },
    /// End an active call.
    End {
        /// Call ID to hang up.
        call_id: String,
    },
    /// Verify telephony setup (credentials, webhook reachability).
    Setup,
}

pub async fn handle_voicecall(action: VoiceCallAction) -> Result<()> {
    match action {
        VoiceCallAction::Call { to, message, mode } => {
            println!("Initiating {mode} call to {to}...");
            if let Some(msg) = &message {
                println!("  Message: {msg}");
            }
            // In a full implementation this would connect to the running gateway
            // via RPC. For now, print instructions.
            println!("\nTo make calls, start the gateway with telephony configured:");
            println!("  [channels.telephony.default]");
            println!("  provider = \"twilio\"");
            println!("  account_sid = \"AC...\"");
            println!("  auth_token = \"...\"");
            println!("  from_number = \"+15551234567\"");
            Ok(())
        },
        VoiceCallAction::Status { call_id } => {
            println!("Querying status for call {call_id}...");
            println!("(connect to gateway RPC for live status)");
            Ok(())
        },
        VoiceCallAction::End { call_id } => {
            println!("Ending call {call_id}...");
            println!("(connect to gateway RPC to end calls)");
            Ok(())
        },
        VoiceCallAction::Setup => {
            println!("Telephony setup check:");
            println!("  1. Configure [channels.telephony.<name>] in moltis.toml");
            println!("  2. Set provider credentials (account_sid, auth_token)");
            println!("  3. Set from_number to your Twilio phone number");
            println!("  4. Set webhook_url to a publicly reachable HTTPS URL");
            println!("  5. Start the gateway: moltis gateway");
            Ok(())
        },
    }
}
