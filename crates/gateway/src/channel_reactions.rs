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

/// Opaque per-message acknowledgment identity: `account:chat:message_id`.
///
/// One inbound message == one ack key == one reaction slot, independent of
/// which agent run eventually handles it.
#[must_use]
pub fn ack_key(account_id: &str, chat_id: &str, message_id: &str) -> String {
    format!("{account_id}:{chat_id}:{message_id}")
}

/// Upper bound on parked (received but not yet running) acknowledgments, so a
/// message that is never claimed by a run cannot grow the map without bound.
/// Controllers additionally self-expire via [`MAX_LIFETIME`].
const MAX_PENDING_ACKS: usize = 512;

/// The acknowledgments a currently-executing run owns.
///
/// `id` distinguishes successive turns on the same session so a slow terminal
/// from turn N can never clear turn N+1's state.
struct ActiveTurn {
    id: u64,
    keys: Vec<String>,
}

#[derive(Default)]
struct RegistryInner {
    /// Received messages with a live reaction, not yet claimed by a run.
    pending: HashMap<String, Arc<ChannelReactionController>>,
    /// Insertion order of `pending`, for oldest-first eviction.
    pending_order: Vec<String>,
    /// Which acknowledgments the running turn owns, per session.
    active: HashMap<String, ActiveTurn>,
}

/// Routes acknowledgment-reaction activity to the right inbound message(s).
///
/// A message's controller is created when it is *received* (so 👀 appears
/// immediately, even while queued) and keyed by its own ack key. A run claims
/// its acknowledgments when it actually starts executing — which is also when
/// the queue decision is known — so activity is never misrouted to a message
/// that merely happens to share the session. In `collect` queue mode a single
/// run legitimately owns several inbound messages, and every one of them
/// receives the phase and terminal reactions.
#[derive(Default)]
pub struct ReactionRegistry {
    inner: Mutex<RegistryInner>,
    next_turn_id: std::sync::atomic::AtomicU64,
}

impl ReactionRegistry {
    /// Park a freshly received message's controller until a run claims it.
    pub async fn register_pending(&self, key: String, controller: Arc<ChannelReactionController>) {
        let mut inner = self.inner.lock().await;
        if inner.pending.insert(key.clone(), controller).is_none() {
            inner.pending_order.push(key);
        }
        while inner.pending_order.len() > MAX_PENDING_ACKS {
            let oldest = inner.pending_order.remove(0);
            inner.pending.remove(&oldest);
        }
    }

    /// Bind the given acknowledgments to this session's now-executing run.
    ///
    /// Any previously active turn for the session is dropped from routing (its
    /// controllers keep their own lifetime cap), so a superseded run cannot
    /// keep driving reactions.
    pub async fn activate(&self, session_key: &str, keys: Vec<String>) {
        if keys.is_empty() {
            return;
        }
        let id = self
            .next_turn_id
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let mut inner = self.inner.lock().await;
        inner
            .active
            .insert(session_key.to_string(), ActiveTurn { id, keys });
    }

    /// Forward an activity to every acknowledgment the session's run owns.
    ///
    /// Terminal activities finalize and release those acknowledgments; the
    /// release is identity-checked against the turn id so a late terminal from
    /// a superseded run cannot clear the current one.
    pub async fn note(&self, session_key: &str, activity: ChannelActivity) {
        let is_terminal = matches!(activity, ChannelActivity::Finished(_));
        let (turn_id, controllers) = {
            let inner = self.inner.lock().await;
            let Some(turn) = inner.active.get(session_key) else {
                return;
            };
            let controllers: Vec<_> = turn
                .keys
                .iter()
                .filter_map(|k| inner.pending.get(k).cloned())
                .collect();
            (turn.id, controllers)
        };

        for controller in &controllers {
            controller.note(activity.clone()).await;
        }

        if is_terminal {
            let mut inner = self.inner.lock().await;
            // Only release if this session's active turn is still ours.
            let still_ours = inner
                .active
                .get(session_key)
                .is_some_and(|turn| turn.id == turn_id);
            if still_ours && let Some(turn) = inner.active.remove(session_key) {
                for key in turn.keys {
                    inner.pending.remove(&key);
                    inner.pending_order.retain(|k| k != &key);
                }
            }
        }
    }

    /// Finalize specific acknowledgments directly, for paths where no run ever
    /// executes (a rejected hook, an early error) and nothing will signal a
    /// terminal for them.
    pub async fn finalize_keys(&self, keys: &[String], outcome: ChannelAckOutcome) {
        let controllers = {
            let mut inner = self.inner.lock().await;
            keys.iter()
                .filter_map(|k| {
                    inner.pending_order.retain(|pk| pk != k);
                    inner.pending.remove(k)
                })
                .collect::<Vec<_>>()
        };
        for controller in controllers {
            controller.note(ChannelActivity::Finished(outcome)).await;
        }
    }
}

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
    // Initial acknowledgment: 👀. Only track it if it actually landed.
    let acked = add_reaction(
        &outbound,
        &account_id,
        &chat_id,
        &message_id,
        RECEIVED_EMOJI,
    )
    .await;
    let mut current: Option<String> = acked.then(|| RECEIVED_EMOJI.to_string());
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
                        let landed =
                            add_reaction(&outbound, &account_id, &chat_id, &message_id, term).await;
                        let stale = current.take().filter(|cur| cur != term);
                        if let Some(cur) = stale.filter(|_| landed) {
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
    if !add_reaction(outbound, account_id, chat_id, message_id, emoji).await {
        // The new marker never landed — keep tracking the old one so it is
        // still cleaned up at the terminal instead of being orphaned.
        return;
    }
    if let Some(cur) = current.take() {
        remove_reaction(outbound, account_id, chat_id, message_id, &cur).await;
    }
    *current = Some(emoji.to_string());
}

/// Returns whether the reaction is believed to be on the message afterwards, so
/// callers only advance their tracked state when the call actually landed.
async fn add_reaction(
    outbound: &Arc<dyn ChannelOutbound>,
    account_id: &str,
    chat_id: &str,
    message_id: &str,
    emoji: &str,
) -> bool {
    match outbound
        .add_reaction(account_id, chat_id, message_id, emoji)
        .await
    {
        Ok(()) => true,
        Err(e) => {
            debug!(
                account_id,
                chat_id, emoji, "channel add_reaction failed: {e}"
            );
            false
        },
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

    /// Build a registry with one parked acknowledgment; returns its ops log.
    async fn park(registry: &ReactionRegistry, key: &str) -> Arc<StdMutex<Vec<String>>> {
        let ops = Arc::new(StdMutex::new(Vec::new()));
        let outbound = Arc::new(RecordingOutbound { ops: ops.clone() });
        let controller =
            ChannelReactionController::start(outbound, "a".into(), "c".into(), key.into());
        registry.register_pending(key.to_string(), controller).await;
        tokio::time::sleep(Duration::from_millis(40)).await;
        ops
    }

    fn ops_of(ops: &Arc<StdMutex<Vec<String>>>) -> Vec<String> {
        ops.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    #[tokio::test]
    async fn queued_message_keeps_its_own_ack_and_is_not_driven_by_the_active_run() {
        // A is running; B arrives and queues behind it. B must keep its own 👀
        // and must not receive A's terminal.
        let registry = ReactionRegistry::default();
        let a = park(&registry, "msg-a").await;
        let b = park(&registry, "msg-b").await;
        registry.activate("sess", vec!["msg-a".into()]).await;

        registry
            .note(
                "sess",
                ChannelActivity::Finished(ChannelAckOutcome::Success),
            )
            .await;
        tokio::time::sleep(Duration::from_millis(60)).await;

        assert_eq!(ops_of(&a), vec!["+👀", "+✅", "-👀"], "A should complete");
        assert_eq!(
            ops_of(&b),
            vec!["+👀"],
            "B must still be waiting, untouched"
        );
    }

    #[tokio::test]
    async fn collect_mode_fans_terminal_out_to_every_combined_message() {
        let registry = ReactionRegistry::default();
        let a = park(&registry, "msg-a").await;
        let b = park(&registry, "msg-b").await;
        // One run answers both messages.
        registry
            .activate("sess", vec!["msg-a".into(), "msg-b".into()])
            .await;

        registry
            .note(
                "sess",
                ChannelActivity::Finished(ChannelAckOutcome::Success),
            )
            .await;
        tokio::time::sleep(Duration::from_millis(60)).await;

        for ops in [&a, &b] {
            assert_eq!(ops_of(ops), vec!["+👀", "+✅", "-👀"]);
        }
    }

    #[tokio::test]
    async fn superseded_turn_terminal_does_not_clear_the_current_turn() {
        // Turn 1 activates, is superseded by turn 2, then turn 1's terminal
        // arrives late. It must not release turn 2's acknowledgment.
        let registry = ReactionRegistry::default();
        let b = park(&registry, "msg-b").await;
        registry.activate("sess", vec!["msg-b".into()]).await;
        registry.activate("sess", vec!["msg-b".into()]).await; // turn 2

        registry
            .note(
                "sess",
                ChannelActivity::Finished(ChannelAckOutcome::Success),
            )
            .await;
        tokio::time::sleep(Duration::from_millis(60)).await;

        // The still-current turn resolved exactly once.
        assert_eq!(ops_of(&b), vec!["+👀", "+✅", "-👀"]);
    }

    #[tokio::test]
    async fn finalize_keys_resolves_acks_that_never_ran() {
        // Hook rejection / early error: no run ever claims the message.
        let registry = ReactionRegistry::default();
        let a = park(&registry, "msg-a").await;
        registry
            .finalize_keys(&["msg-a".to_string()], ChannelAckOutcome::Failure)
            .await;
        tokio::time::sleep(Duration::from_millis(60)).await;
        assert_eq!(ops_of(&a), vec!["+👀", "+❌", "-👀"]);
    }

    #[tokio::test]
    async fn activity_for_unknown_session_is_a_noop() {
        let registry = ReactionRegistry::default();
        let a = park(&registry, "msg-a").await;
        // Never activated — a stray activity must not touch the parked ack.
        registry
            .note(
                "other-session",
                ChannelActivity::Finished(ChannelAckOutcome::Success),
            )
            .await;
        tokio::time::sleep(Duration::from_millis(40)).await;
        assert_eq!(ops_of(&a), vec!["+👀"]);
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
