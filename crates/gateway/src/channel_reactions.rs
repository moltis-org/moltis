//! Per-turn channel acknowledgment reaction controller.
//!
//! A channel-dispatched agent turn is fire-and-forget: `chat.send` spawns the
//! run and returns immediately, so the ✅/❌ terminal cannot be applied from the
//! dispatch call site. Instead, a controller is created when the message is
//! received (adds 👀), driven through phase emojis (🛠️/🌐/💻/…) by the agent
//! loop, and finalized (✅/❌ or nothing on cancel) when the run completes.
//!
//! The controller owns a single reaction "slot" on the inbound message: each
//! transition removes the current emoji and adds the next, so at most one
//! reaction from the bot is visible at a time. All Slack API calls run on one
//! serialized worker task (no concurrent add/remove races), phase changes are
//! debounced to avoid flicker, and terminal transitions win over late phases.
//!
//! Design ported from openclaw's `status-reactions.ts`.

use std::{collections::HashMap, sync::Arc, time::Duration};

use {
    moltis_channels::{ChannelAckOutcome, ChannelActivity, ChannelOutbound},
    tokio::sync::{Mutex, mpsc},
    tracing::debug,
};

/// Emoji shortcodes for acknowledgment/phase reactions (Slack-style names).
const RECEIVED_EMOJI: &str = "eyes";
const SUCCESS_EMOJI: &str = "white_check_mark";
const ERROR_EMOJI: &str = "x";
const STALL_EMOJI: &str = "hourglass_flowing_sand";

/// Coalesce rapid phase changes so the reaction does not flicker.
const PHASE_DEBOUNCE: Duration = Duration::from_millis(700);
/// After this idle time with no activity, show a "still working" marker.
const STALL_AFTER: Duration = Duration::from_secs(20);

/// Registry of active controllers, keyed by session key (runs serialize per
/// session via the send permit, so at most one is active per session).
pub type ReactionControllerRegistry = Arc<Mutex<HashMap<String, Arc<ChannelReactionController>>>>;

/// Classify a tool name into a phase emoji shortcode.
///
/// Mirrors openclaw's token lists so the single reaction slot communicates what
/// kind of work is happening. Unknown tools fall back to the generic tool emoji.
#[must_use]
pub fn classify_tool_emoji(tool_name: &str) -> &'static str {
    let name = tool_name.to_ascii_lowercase();
    let has = |tokens: &[&str]| tokens.iter().any(|t| name.contains(*t));

    if has(&[
        "web_search",
        "search",
        "web_fetch",
        "fetch",
        "browse",
        "navigate",
        "http",
        "url",
    ]) {
        "globe_with_meridians"
    } else if has(&["bash", "exec", "shell", "process", "command", "run"]) {
        "computer"
    } else if has(&[
        "edit",
        "write",
        "patch",
        "apply",
        "str_replace",
        "create_file",
        "code",
    ]) {
        "pencil2"
    } else if has(&["deploy", "fly", "release", "publish"]) {
        "airplane_departure"
    } else if has(&["build", "compile", "cargo", "npm", "make"]) {
        "building_construction"
    } else {
        "hammer_and_wrench"
    }
}

/// Terminal emoji for an outcome, or `None` when no marker should be left
/// (cancelled turns strip the in-progress reaction and add nothing).
#[must_use]
fn terminal_emoji(outcome: ChannelAckOutcome) -> Option<&'static str> {
    match outcome {
        ChannelAckOutcome::Success => Some(SUCCESS_EMOJI),
        ChannelAckOutcome::Failure => Some(ERROR_EMOJI),
        ChannelAckOutcome::Cancelled => None,
    }
}

/// Command sent to the controller's serialized worker.
#[derive(Debug)]
enum Command {
    Phase(String),
    Finish(ChannelAckOutcome),
}

/// A per-turn reaction controller. Cheap handle around an mpsc sender to the
/// worker task that owns the reaction state machine.
pub struct ChannelReactionController {
    tx: mpsc::Sender<Command>,
}

impl ChannelReactionController {
    /// Start a controller: spawns the worker, which immediately adds 👀 to the
    /// target message. `message_id` is the exact inbound message ts.
    #[must_use]
    pub fn start(
        outbound: Arc<dyn ChannelOutbound>,
        account_id: String,
        chat_id: String,
        message_id: String,
    ) -> Arc<Self> {
        let (tx, rx) = mpsc::channel(32);
        tokio::spawn(run_worker(rx, outbound, account_id, chat_id, message_id));
        Arc::new(Self { tx })
    }

    /// Forward an activity signal from the agent run.
    pub async fn note(&self, activity: ChannelActivity) {
        let cmd = match activity {
            ChannelActivity::Tool(name) => Command::Phase(classify_tool_emoji(&name).to_string()),
            // Thinking maps back to the base received/working marker.
            ChannelActivity::Thinking => Command::Phase(RECEIVED_EMOJI.to_string()),
            ChannelActivity::Finished(outcome) => Command::Finish(outcome),
        };
        // A closed channel means the worker already finalized; drop silently.
        let _ = self.tx.send(cmd).await;
    }
}

/// The serialized reaction state machine.
async fn run_worker(
    mut rx: mpsc::Receiver<Command>,
    outbound: Arc<dyn ChannelOutbound>,
    account_id: String,
    chat_id: String,
    message_id: String,
) {
    // Initial acknowledgment: 👀.
    add_reaction(
        &outbound,
        &account_id,
        &chat_id,
        &message_id,
        RECEIVED_EMOJI,
    )
    .await;
    let mut current: Option<String> = Some(RECEIVED_EMOJI.to_string());
    let mut pending: Option<String> = None;
    let mut stalled = false;

    loop {
        // Wait for the next command. If a phase change is pending, apply it once
        // the debounce window elapses; otherwise fall back to the stall timer.
        let wait = if pending.is_some() {
            PHASE_DEBOUNCE
        } else {
            STALL_AFTER
        };

        match tokio::time::timeout(wait, rx.recv()).await {
            Ok(Some(Command::Phase(emoji))) => {
                // Coalesce: remember the latest requested phase, apply after debounce.
                if current.as_deref() != Some(emoji.as_str()) {
                    pending = Some(emoji);
                }
            },
            Ok(Some(Command::Finish(outcome))) => {
                if let Some(cur) = current.take() {
                    remove_reaction(&outbound, &account_id, &chat_id, &message_id, &cur).await;
                }
                if let Some(term) = terminal_emoji(outcome) {
                    add_reaction(&outbound, &account_id, &chat_id, &message_id, term).await;
                }
                return;
            },
            Ok(None) => {
                // Sender dropped without a terminal — leave the current marker.
                return;
            },
            Err(_) => {
                // Timeout elapsed.
                if let Some(emoji) = pending.take() {
                    swap(
                        &outbound,
                        &account_id,
                        &chat_id,
                        &message_id,
                        &mut current,
                        &emoji,
                    )
                    .await;
                    stalled = false;
                } else if !stalled {
                    // Idle too long: show a "still working" marker.
                    swap(
                        &outbound,
                        &account_id,
                        &chat_id,
                        &message_id,
                        &mut current,
                        STALL_EMOJI,
                    )
                    .await;
                    stalled = true;
                }
            },
        }
    }
}

/// Remove the current emoji (if any) and add the new one, updating `current`.
async fn swap(
    outbound: &Arc<dyn ChannelOutbound>,
    account_id: &str,
    chat_id: &str,
    message_id: &str,
    current: &mut Option<String>,
    emoji: &str,
) {
    if current.as_deref() == Some(emoji) {
        return;
    }
    if let Some(cur) = current.take() {
        remove_reaction(outbound, account_id, chat_id, message_id, &cur).await;
    }
    add_reaction(outbound, account_id, chat_id, message_id, emoji).await;
    *current = Some(emoji.to_string());
}

async fn add_reaction(
    outbound: &Arc<dyn ChannelOutbound>,
    account_id: &str,
    chat_id: &str,
    message_id: &str,
    emoji: &str,
) {
    if let Err(e) = outbound
        .add_reaction(account_id, chat_id, message_id, emoji)
        .await
    {
        debug!(
            account_id,
            chat_id, emoji, "channel add_reaction failed: {e}"
        );
    }
}

async fn remove_reaction(
    outbound: &Arc<dyn ChannelOutbound>,
    account_id: &str,
    chat_id: &str,
    message_id: &str,
    emoji: &str,
) {
    if let Err(e) = outbound
        .remove_reaction(account_id, chat_id, message_id, emoji)
        .await
    {
        debug!(
            account_id,
            chat_id, emoji, "channel remove_reaction failed: {e}"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_web_tools() {
        assert_eq!(classify_tool_emoji("web_search"), "globe_with_meridians");
        assert_eq!(classify_tool_emoji("web_fetch"), "globe_with_meridians");
    }

    #[test]
    fn classifies_shell_and_edit_tools() {
        assert_eq!(classify_tool_emoji("exec"), "computer");
        assert_eq!(classify_tool_emoji("bash"), "computer");
        assert_eq!(classify_tool_emoji("str_replace_editor"), "pencil2");
    }

    #[test]
    fn unknown_tool_uses_generic_emoji() {
        assert_eq!(classify_tool_emoji("some_mcp_tool"), "hammer_and_wrench");
    }

    #[test]
    fn terminal_emoji_maps_outcomes() {
        assert_eq!(
            terminal_emoji(ChannelAckOutcome::Success),
            Some("white_check_mark")
        );
        assert_eq!(terminal_emoji(ChannelAckOutcome::Failure), Some("x"));
        assert_eq!(terminal_emoji(ChannelAckOutcome::Cancelled), None);
    }
}
