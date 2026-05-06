use {
    futures::StreamExt,
    matrix_sdk::{
        Client, Room,
        encryption::verification::{SasState, Verification, VerificationRequestState},
        ruma::{
            OwnedRoomId, OwnedUserId,
            events::{
                ToDeviceEvent,
                key::verification::request::ToDeviceKeyVerificationRequestEventContent,
            },
        },
    },
    tracing::{info, warn},
};

use crate::{
    handler::send_text,
    state::{AccountStateMap, VerificationPrompt},
};

enum VerificationCommandAction {
    Confirm,
    Reject,
    Show,
    Cancel,
}

pub(crate) async fn maybe_handle_confirmation_message(
    body: &str,
    room: &Room,
    account_id: &str,
    sender_id: &str,
    accounts: &AccountStateMap,
) -> bool {
    let Some(action) = parse_verification_command(body) else {
        return false;
    };

    let Some((prompt, client)) = find_prompt_for_sender(account_id, sender_id, room, accounts)
    else {
        return false;
    };

    match action {
        VerificationCommandAction::Show => {
            notify_room(room, &format_prompt_message(&prompt)).await;
            return true;
        },
        VerificationCommandAction::Confirm
        | VerificationCommandAction::Reject
        | VerificationCommandAction::Cancel => {},
    }

    let Ok(other_user_id) = prompt.other_user_id.parse::<OwnedUserId>() else {
        clear_prompt(account_id, &prompt.flow_id, accounts);
        notify_room(
            room,
            "Matrix verification state became invalid, please start verification again.",
        )
        .await;
        return true;
    };

    let Some(Verification::SasV1(sas)) = client
        .encryption()
        .get_verification(&other_user_id, &prompt.flow_id)
        .await
    else {
        clear_prompt(account_id, &prompt.flow_id, accounts);
        notify_room(
            room,
            "No active Matrix verification flow is ready anymore, please start verification again.",
        )
        .await;
        return true;
    };

    let result = match action {
        VerificationCommandAction::Confirm => sas.confirm().await,
        VerificationCommandAction::Reject => sas.mismatch().await,
        VerificationCommandAction::Cancel => sas.cancel().await,
        VerificationCommandAction::Show => Ok(()),
    };

    if let Err(error) = result {
        warn!(
            account_id,
            flow_id = prompt.flow_id,
            other_user_id = prompt.other_user_id,
            "failed to update Matrix verification flow: {error}"
        );
        notify_room(
            room,
            "Failed to update the Matrix verification flow, please try again.",
        )
        .await;
        return true;
    }

    let status = match action {
        VerificationCommandAction::Confirm => {
            "Sent Matrix verification confirmation. Element should finish shortly."
        },
        VerificationCommandAction::Reject => {
            "Sent Matrix verification mismatch. Start a new verification if you want to retry."
        },
        VerificationCommandAction::Cancel => "Cancelled the Matrix verification flow.",
        VerificationCommandAction::Show => unreachable!(),
    };
    notify_room(room, status).await;
    true
}

pub(crate) async fn handle_room_verification_request(
    room: Room,
    account_id: String,
    sender_id: String,
    flow_id: String,
    accounts: AccountStateMap,
) {
    track_verification_flow(
        account_id,
        sender_id,
        flow_id,
        Some(room.room_id().to_string()),
        accounts,
    )
    .await;
}

pub(crate) async fn handle_to_device_verification_request(
    event: ToDeviceEvent<ToDeviceKeyVerificationRequestEventContent>,
    account_id: String,
    accounts: AccountStateMap,
) {
    track_verification_flow(
        account_id,
        event.sender.to_string(),
        event.content.transaction_id.to_string(),
        None,
        accounts,
    )
    .await;
}

async fn track_verification_flow(
    account_id: String,
    other_user_id: String,
    flow_id: String,
    room_id: Option<String>,
    accounts: AccountStateMap,
) {
    let Some((client, should_track)) =
        account_client_and_insert_flow(&account_id, &flow_id, &accounts)
    else {
        return;
    };

    if !should_track {
        return;
    }

    let request = match fetch_verification_request(&client, &other_user_id, &flow_id).await {
        Some(request) => request,
        None => {
            clear_flow(&account_id, &flow_id, &accounts);
            warn!(
                account_id,
                %flow_id,
                %other_user_id,
                "Matrix verification request was not available after receiving the event"
            );
            return;
        },
    };

    if let Err(error) = request.accept().await {
        clear_flow(&account_id, &flow_id, &accounts);
        warn!(
            account_id,
            %flow_id,
            %other_user_id,
            "failed to accept Matrix verification request: {error}"
        );
        return;
    }

    info!(
        account_id,
        %flow_id,
        %other_user_id,
        room_id = room_id.as_deref(),
        "accepted Matrix verification request"
    );

    tokio::spawn(async move {
        watch_verification_request(
            request,
            client,
            account_id,
            other_user_id,
            flow_id,
            room_id,
            accounts,
        )
        .await;
    });
}

async fn watch_verification_request(
    request: matrix_sdk::encryption::verification::VerificationRequest,
    client: Client,
    account_id: String,
    other_user_id: String,
    flow_id: String,
    room_id: Option<String>,
    accounts: AccountStateMap,
) {
    let mut changes = request.changes();

    while let Some(state) = changes.next().await {
        match state {
            VerificationRequestState::Requested { .. } => {
                if let Err(error) = request.accept().await {
                    warn!(account_id, %flow_id, %other_user_id, "failed to re-accept Matrix verification request: {error}");
                }
            },
            VerificationRequestState::Ready { .. } => match request.start_sas().await {
                Ok(Some(sas)) => {
                    spawn_sas_watcher(
                        sas,
                        client.clone(),
                        account_id.clone(),
                        other_user_id.clone(),
                        flow_id.clone(),
                        room_id.clone(),
                        accounts.clone(),
                    );
                    return;
                },
                Ok(None) => {},
                Err(error) => {
                    clear_flow(&account_id, &flow_id, &accounts);
                    warn!(account_id, %flow_id, %other_user_id, "failed to start Matrix SAS verification: {error}");
                    return;
                },
            },
            VerificationRequestState::Transitioned { verification } => {
                if let Some(sas) = verification.sas() {
                    spawn_sas_watcher(
                        sas,
                        client.clone(),
                        account_id.clone(),
                        other_user_id.clone(),
                        flow_id.clone(),
                        room_id.clone(),
                        accounts.clone(),
                    );
                    return;
                }
            },
            VerificationRequestState::Done => {
                clear_flow(&account_id, &flow_id, &accounts);
                return;
            },
            VerificationRequestState::Cancelled(cancel_info) => {
                clear_flow(&account_id, &flow_id, &accounts);
                warn!(
                    account_id,
                    %flow_id,
                    %other_user_id,
                    reason = cancel_info.reason(),
                    "Matrix verification request was cancelled before SAS started"
                );
                return;
            },
            VerificationRequestState::Created { .. } => {},
        }
    }

    clear_flow(&account_id, &flow_id, &accounts);
}

fn spawn_sas_watcher(
    sas: matrix_sdk::encryption::verification::SasVerification,
    client: Client,
    account_id: String,
    other_user_id: String,
    flow_id: String,
    room_id: Option<String>,
    accounts: AccountStateMap,
) {
    tokio::spawn(async move {
        watch_sas_verification(
            sas,
            client,
            account_id,
            other_user_id,
            flow_id,
            room_id,
            accounts,
        )
        .await;
    });
}

async fn watch_sas_verification(
    sas: matrix_sdk::encryption::verification::SasVerification,
    client: Client,
    account_id: String,
    other_user_id: String,
    flow_id: String,
    room_id: Option<String>,
    accounts: AccountStateMap,
) {
    if !sas.we_started()
        && let Err(error) = sas.accept().await
    {
        clear_flow(&account_id, &flow_id, &accounts);
        warn!(account_id, %flow_id, %other_user_id, "failed to accept Matrix SAS verification: {error}");
        return;
    }

    let mut changes = sas.changes();
    while let Some(state) = changes.next().await {
        match state {
            SasState::KeysExchanged {
                emojis,
                decimals: _,
            } => {
                let Some(emojis) = emojis else {
                    continue;
                };

                let room_id =
                    resolve_verification_room_id(&client, room_id.clone(), &other_user_id).await;
                let prompt = VerificationPrompt {
                    flow_id: flow_id.clone(),
                    other_user_id: other_user_id.clone(),
                    room_id: room_id.clone(),
                    emoji_lines: emojis
                        .emojis
                        .into_iter()
                        .map(|emoji| format!("{} {}", emoji.symbol, emoji.description))
                        .collect(),
                };
                store_prompt(&account_id, prompt.clone(), &accounts);

                if let Some(room) = room_for_notice(&client, room_id.as_deref()) {
                    notify_room(&room, &format_prompt_message(&prompt)).await;
                } else {
                    info!(
                        account_id,
                        %flow_id,
                        %other_user_id,
                        emojis = ?prompt.emoji_lines,
                        "Matrix verification is waiting for user confirmation but no room was found to post instructions"
                    );
                }
            },
            SasState::Done { .. } => {
                let room_id =
                    prompt_room_id(&account_id, &flow_id, &accounts).or_else(|| room_id.clone());
                clear_flow(&account_id, &flow_id, &accounts);
                if let Some(room) = room_for_notice(&client, room_id.as_deref()) {
                    notify_room(&room, "Matrix verification completed successfully.").await;
                }
                return;
            },
            SasState::Cancelled(cancel_info) => {
                let room_id =
                    prompt_room_id(&account_id, &flow_id, &accounts).or_else(|| room_id.clone());
                clear_flow(&account_id, &flow_id, &accounts);
                if let Some(room) = room_for_notice(&client, room_id.as_deref()) {
                    let message = format!(
                        "Matrix verification was cancelled: {}",
                        cancel_info.reason()
                    );
                    notify_room(&room, &message).await;
                }
                return;
            },
            SasState::Created { .. }
            | SasState::Started { .. }
            | SasState::Accepted { .. }
            | SasState::Confirmed => {},
        }
    }

    clear_flow(&account_id, &flow_id, &accounts);
}

async fn fetch_verification_request(
    client: &Client,
    other_user_id: &str,
    flow_id: &str,
) -> Option<matrix_sdk::encryption::verification::VerificationRequest> {
    let user_id = other_user_id.parse::<OwnedUserId>().ok()?;
    for _ in 0..5 {
        if let Some(request) = client
            .encryption()
            .get_verification_request(&user_id, flow_id)
            .await
        {
            return Some(request);
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
    None
}

fn account_client_and_insert_flow(
    account_id: &str,
    flow_id: &str,
    accounts: &AccountStateMap,
) -> Option<(Client, bool)> {
    let guard = accounts.read().unwrap_or_else(|error| error.into_inner());
    let state = guard.get(account_id)?;
    let should_track = {
        let mut verification = state
            .verification
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        verification.watched_flows.insert(flow_id.to_string())
    };
    Some((state.client.clone(), should_track))
}

fn store_prompt(account_id: &str, prompt: VerificationPrompt, accounts: &AccountStateMap) {
    let guard = accounts.read().unwrap_or_else(|error| error.into_inner());
    let Some(state) = guard.get(account_id) else {
        return;
    };
    let mut verification = state
        .verification
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    verification.prompts.insert(prompt.flow_id.clone(), prompt);
}

fn prompt_room_id(account_id: &str, flow_id: &str, accounts: &AccountStateMap) -> Option<String> {
    let guard = accounts.read().unwrap_or_else(|error| error.into_inner());
    let state = guard.get(account_id)?;
    let verification = state
        .verification
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    verification
        .prompts
        .get(flow_id)
        .and_then(|prompt| prompt.room_id.clone())
}

fn clear_prompt(account_id: &str, flow_id: &str, accounts: &AccountStateMap) {
    let guard = accounts.read().unwrap_or_else(|error| error.into_inner());
    let Some(state) = guard.get(account_id) else {
        return;
    };
    let mut verification = state
        .verification
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    verification.prompts.remove(flow_id);
}

fn clear_flow(account_id: &str, flow_id: &str, accounts: &AccountStateMap) {
    let guard = accounts.read().unwrap_or_else(|error| error.into_inner());
    let Some(state) = guard.get(account_id) else {
        return;
    };
    let mut verification = state
        .verification
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    verification.watched_flows.remove(flow_id);
    verification.prompts.remove(flow_id);
}

fn find_prompt_for_sender(
    account_id: &str,
    sender_id: &str,
    room: &Room,
    accounts: &AccountStateMap,
) -> Option<(VerificationPrompt, Client)> {
    let guard = accounts.read().unwrap_or_else(|error| error.into_inner());
    let state = guard.get(account_id)?;
    let prompt = {
        let verification = state
            .verification
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        verification
            .prompts
            .values()
            .find(|prompt| {
                prompt.other_user_id == sender_id
                    && prompt.room_id.as_deref() == Some(room.room_id().as_str())
            })
            .cloned()
    }?;
    Some((prompt, state.client.clone()))
}

async fn resolve_verification_room_id(
    client: &Client,
    preferred_room_id: Option<String>,
    other_user_id: &str,
) -> Option<String> {
    if preferred_room_id.is_some() {
        return preferred_room_id;
    }

    let user_id = other_user_id.parse::<OwnedUserId>().ok()?;
    for room in client.joined_rooms() {
        let direct_flag = room.is_direct().await.unwrap_or(false);
        let looks_like_dm =
            direct_flag || room.active_members_count() == 2 || room.joined_members_count() == 2;
        if !looks_like_dm {
            continue;
        }

        let Ok(Some(_)) = room.get_member_no_sync(&user_id).await else {
            continue;
        };
        return Some(room.room_id().to_string());
    }

    None
}

fn room_for_notice(client: &Client, room_id: Option<&str>) -> Option<Room> {
    let room_id = room_id?.parse::<OwnedRoomId>().ok()?;
    client.get_room(&room_id)
}

async fn notify_room(room: &Room, message: &str) {
    if let Err(error) = send_text(room, message).await {
        warn!(room = %room.room_id(), "failed to send Matrix verification notice: {error}");
    }
}

fn parse_verification_command(body: &str) -> Option<VerificationCommandAction> {
    match body.trim().to_ascii_lowercase().as_str() {
        "verify yes" | "verify accept" | "verify confirm" => {
            Some(VerificationCommandAction::Confirm)
        },
        "verify no" | "verify reject" | "verify mismatch" => {
            Some(VerificationCommandAction::Reject)
        },
        "verify show" | "verify emojis" => Some(VerificationCommandAction::Show),
        "verify cancel" => Some(VerificationCommandAction::Cancel),
        _ => None,
    }
}

fn format_prompt_message(prompt: &VerificationPrompt) -> String {
    format!(
        "Matrix verification for {} is waiting.\n\
Compare these emojis with Element:\n\
{}\n\n\
Send one of these exact messages as a normal message in this same Matrix chat:\n\
verify yes\n\
verify no\n\
verify show\n\
verify cancel",
        prompt.other_user_id,
        prompt.emoji_lines.join("\n"),
    )
}

#[cfg(test)]
mod tests {
    use super::{VerificationPrompt, format_prompt_message, parse_verification_command};

    #[test]
    fn parse_verification_commands_accepts_expected_variants() {
        assert!(parse_verification_command("verify yes").is_some());
        assert!(parse_verification_command("verify no").is_some());
        assert!(parse_verification_command("verify show").is_some());
        assert!(parse_verification_command("verify cancel").is_some());
        assert!(parse_verification_command("hey bot").is_none());
    }

    #[test]
    fn prompt_message_contains_emojis_and_instructions() {
        let prompt = VerificationPrompt {
            flow_id: "flow".into(),
            other_user_id: "@alice:matrix.org".into(),
            room_id: Some("!room:matrix.org".into()),
            emoji_lines: vec!["🐶 Dog".into(), "🔥 Fire".into()],
        };

        let message = format_prompt_message(&prompt);
        assert!(message.contains("@alice:matrix.org"));
        assert!(message.contains("🐶 Dog"));
        assert!(message.contains("verify yes"));
        assert!(message.contains("verify no"));
        assert!(message.contains("same Matrix chat"));
    }
}
