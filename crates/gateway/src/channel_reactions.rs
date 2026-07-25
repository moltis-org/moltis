//! Per-turn channel acknowledgment reaction controller.
//!
//! A channel-dispatched agent turn is fire-and-forget: `chat.send` spawns the
//! run and returns immediately, so the ✅/❌ terminal cannot be applied from the
//! dispatch call site. Instead, a controller is created when the message is
//! received (adds 👀), driven through phase emojis (🛠️/🌐/💻/…) by the agent
//! loop, and finalized (✅/❌ or nothing on cancel) when the run completes.
//!
//! The controller owns a single reaction "slot" on the inbound message: each
//! transition adds the next emoji and then removes the previous one, so the
//! message is never momentarily bare and a failed add leaves the old marker
//! rather than clearing the acknowledgment entirely. All reaction API calls run
//! on one serialized worker task (no concurrent add/remove races), phase changes
//! are debounced to avoid flicker, and terminals win over late phases.
//!
//! Emoji come from the channel-neutral [`ack_emoji`] vocabulary; each channel
//! translates at its own boundary (Slack maps glyphs to shortcodes, Matrix uses
//! glyphs directly), so this controller stays platform-agnostic.
//!
//! Design ported from openclaw's `status-reactions.ts`.

use std::{collections::HashMap, sync::Arc, time::Duration};

use {
    moltis_channels::{ChannelAckOutcome, ChannelActivity, ChannelOutbound},
    tokio::sync::{Mutex, mpsc},
    tracing::debug,
};

/// Acknowledgment/phase emoji, from the shared channel-neutral vocabulary.
/// Channels translate these at their own boundary (Slack maps glyphs to
/// shortcodes; Matrix uses the glyph directly).
use moltis_channels::activity::ack_emoji;

const RECEIVED_EMOJI: &str = ack_emoji::RECEIVED;
const SUCCESS_EMOJI: &str = ack_emoji::SUCCESS;
const ERROR_EMOJI: &str = ack_emoji::ERROR;
const STALL_EMOJI: &str = ack_emoji::STALL;

/// Coalesce rapid phase changes so the reaction does not flicker.
const PHASE_DEBOUNCE: Duration = Duration::from_millis(700);
/// After this idle time with no activity, show a "still working" marker.
const STALL_AFTER: Duration = Duration::from_secs(20);
/// Hard cap on a worker's lifetime. Normally the run signals completion long
/// before this; the cap is a safety net so a run that never finalizes (e.g. a
/// panicked task that skips the terminal signal) can't leave the worker task —
/// and the 👀 reaction — alive forever.
const MAX_LIFETIME: Duration = Duration::from_secs(900);

/// Registry of active controllers, keyed by session key.
///
/// Agent runs serialize per session (via the send permit), so at most one run
/// is executing at a time. If a second message for the same session is received
/// while the first is still running, registering its controller replaces (and
/// finalizes) the first — see `register_channel_reaction_controller`.
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
        ack_emoji::WEB
    } else if has(&["bash", "exec", "shell", "process", "command", "run"]) {
        ack_emoji::SHELL
    } else if has(&[
        "edit",
        "write",
        "patch",
        "apply",
        "str_replace",
        "create_file",
        "code",
    ]) {
        ack_emoji::EDIT
    } else if has(&["deploy", "fly", "release", "publish"]) {
        ack_emoji::DEPLOY
    } else if has(&["build", "compile", "cargo", "npm", "make"]) {
        ack_emoji::BUILD
    } else {
        ack_emoji::TOOL
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
    let started = tokio::time::Instant::now();

    loop {
        // Safety net: never outlive the hard cap. If a run failed to signal
        // completion (e.g. it panicked), strip the in-progress marker and exit
        // so neither the task nor the 👀 reaction leaks.
        if started.elapsed() >= MAX_LIFETIME {
            if let Some(cur) = current.take() {
                remove_reaction(&outbound, &account_id, &chat_id, &message_id, &cur).await;
            }
            return;
        }

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
                match terminal_emoji(outcome) {
                    // Add the terminal marker *before* removing the in-progress
                    // one so the message is never momentarily bare, and a failed
                    // add still leaves the previous marker rather than nothing.
                    Some(term) => {
                        add_reaction(&outbound, &account_id, &chat_id, &message_id, term).await;
                        if let Some(cur) = current.take().filter(|cur| cur != term) {
                            remove_reaction(&outbound, &account_id, &chat_id, &message_id, &cur)
                                .await;
                        }
                    },
                    // Cancelled: strip the in-progress marker, leave nothing.
                    None => {
                        if let Some(cur) = current.take() {
                            remove_reaction(&outbound, &account_id, &chat_id, &message_id, &cur)
                                .await;
                        }
                    },
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
    // Add first, then remove the previous marker: the message always shows at
    // least one reaction, and a failed add leaves the old marker in place
    // instead of clearing the acknowledgment entirely.
    add_reaction(outbound, account_id, chat_id, message_id, emoji).await;
    if let Some(cur) = current.take() {
        remove_reaction(outbound, account_id, chat_id, message_id, &cur).await;
    }
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
    use std::sync::Mutex as StdMutex;

    use moltis_channels::{Result as ChannelResult, plugin::ChannelOutbound};

    use super::*;

    /// A mock outbound that records the reaction add/remove operations in order.
    struct RecordingOutbound {
        ops: Arc<StdMutex<Vec<String>>>,
    }

    #[async_trait::async_trait]
    impl ChannelOutbound for RecordingOutbound {
        async fn send_text(&self, _: &str, _: &str, _: &str, _: Option<&str>) -> ChannelResult<()> {
            Ok(())
        }

        async fn send_media(
            &self,
            _: &str,
            _: &str,
            _: &moltis_common::types::ReplyPayload,
            _: Option<&str>,
        ) -> ChannelResult<()> {
            Ok(())
        }

        async fn add_reaction(&self, _: &str, _: &str, _: &str, emoji: &str) -> ChannelResult<()> {
            self.ops
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(format!("+{emoji}"));
            Ok(())
        }

        async fn remove_reaction(
            &self,
            _: &str,
            _: &str,
            _: &str,
            emoji: &str,
        ) -> ChannelResult<()> {
            self.ops
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(format!("-{emoji}"));
            Ok(())
        }
    }

    async fn run_lifecycle(outcome: ChannelAckOutcome) -> Vec<String> {
        let ops = Arc::new(StdMutex::new(Vec::new()));
        let outbound = Arc::new(RecordingOutbound { ops: ops.clone() });
        let controller =
            ChannelReactionController::start(outbound, "a".into(), "c".into(), "m".into());
        // Let the worker apply the initial 👀 before signalling completion.
        tokio::time::sleep(Duration::from_millis(50)).await;
        controller.note(ChannelActivity::Finished(outcome)).await;
        tokio::time::sleep(Duration::from_millis(50)).await;
        ops.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    #[tokio::test]
    async fn lifecycle_success_swaps_eyes_for_check() {
        assert_eq!(run_lifecycle(ChannelAckOutcome::Success).await, vec![
            "+👀", "+✅", "-👀"
        ]);
    }

    #[tokio::test]
    async fn lifecycle_failure_swaps_eyes_for_x() {
        assert_eq!(run_lifecycle(ChannelAckOutcome::Failure).await, vec![
            "+👀", "+❌", "-👀"
        ]);
    }

    #[tokio::test]
    async fn lifecycle_cancelled_strips_eyes_with_no_terminal() {
        // Cancelled removes the in-progress marker and adds nothing.
        assert_eq!(run_lifecycle(ChannelAckOutcome::Cancelled).await, vec![
            "+👀", "-👀"
        ]);
    }

    #[tokio::test]
    async fn tool_phase_swaps_marker_then_terminal() {
        let ops = Arc::new(StdMutex::new(Vec::new()));
        let outbound = Arc::new(RecordingOutbound { ops: ops.clone() });
        let controller =
            ChannelReactionController::start(outbound, "a".into(), "c".into(), "m".into());
        tokio::time::sleep(Duration::from_millis(50)).await;
        controller
            .note(ChannelActivity::Tool("web_search".into()))
            .await;
        // Wait past the debounce window so the phase is applied.
        tokio::time::sleep(PHASE_DEBOUNCE + Duration::from_millis(100)).await;
        controller
            .note(ChannelActivity::Finished(ChannelAckOutcome::Success))
            .await;
        tokio::time::sleep(Duration::from_millis(50)).await;
        let recorded = ops.lock().unwrap_or_else(|e| e.into_inner()).clone();
        // Each transition adds the new marker before removing the old one, so
        // the message is never left without a reaction.
        assert_eq!(recorded, vec!["+👀", "+🌐", "-👀", "+✅", "-🌐"]);
    }

    #[test]
    fn classifies_web_tools() {
        assert_eq!(classify_tool_emoji("web_search"), ack_emoji::WEB);
        assert_eq!(classify_tool_emoji("web_fetch"), ack_emoji::WEB);
    }

    #[test]
    fn classifies_shell_and_edit_tools() {
        assert_eq!(classify_tool_emoji("exec"), ack_emoji::SHELL);
        assert_eq!(classify_tool_emoji("bash"), ack_emoji::SHELL);
        assert_eq!(classify_tool_emoji("str_replace_editor"), ack_emoji::EDIT);
    }

    #[test]
    fn unknown_tool_uses_generic_emoji() {
        assert_eq!(classify_tool_emoji("some_mcp_tool"), ack_emoji::TOOL);
    }

    #[test]
    fn terminal_emoji_maps_outcomes() {
        assert_eq!(
            terminal_emoji(ChannelAckOutcome::Success),
            Some(ack_emoji::SUCCESS)
        );
        assert_eq!(
            terminal_emoji(ChannelAckOutcome::Failure),
            Some(ack_emoji::ERROR)
        );
        assert_eq!(terminal_emoji(ChannelAckOutcome::Cancelled), None);
    }
}
