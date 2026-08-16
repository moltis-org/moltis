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

use std::{
    collections::{HashMap, VecDeque},
    sync::Arc,
    time::{Duration, Instant},
};

use {
    moltis_channels::{ChannelAckOutcome, ChannelActivity, ChannelOutbound},
    tokio::sync::{Mutex, mpsc},
    tracing::{debug, warn},
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
/// Pause before the single terminal-reaction retry. The terminal is the last
/// thing the worker does, so losing it leaves a permanently stale marker.
const TERMINAL_RETRY_DELAY: Duration = Duration::from_millis(400);

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
/// Threshold for cleaning up turns that have exceeded [`MAX_LIFETIME`]. Healthy
/// concurrent turns may temporarily exceed it.
const MAX_ACTIVE_TURNS: usize = 512;

struct ActiveAck {
    key: String,
    controller: Arc<ChannelReactionController>,
}

/// The acknowledgments a currently-executing run owns.
struct ActiveTurn {
    session_key: String,
    acknowledgments: Vec<ActiveAck>,
    activated_at: Instant,
}

#[derive(Default)]
struct RegistryInner {
    /// Received messages with a live reaction, not yet claimed by a run.
    pending: HashMap<String, Arc<ChannelReactionController>>,
    /// Insertion order of `pending`, for oldest-first eviction.
    pending_order: VecDeque<String>,
    /// Controllers owned by a running turn, keyed by the run's identity.
    active: HashMap<String, ActiveTurn>,
    /// Current activity identity per session, used to settle a displaced turn.
    active_by_session: HashMap<String, String>,
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
pub struct ReactionRegistry {
    inner: Mutex<RegistryInner>,
    pending_limit: usize,
    active_limit: usize,
    active_lifetime: Duration,
}

impl Default for ReactionRegistry {
    fn default() -> Self {
        Self {
            inner: Mutex::new(RegistryInner::default()),
            pending_limit: MAX_PENDING_ACKS,
            active_limit: MAX_ACTIVE_TURNS,
            active_lifetime: MAX_LIFETIME,
        }
    }
}

impl ReactionRegistry {
    /// Park a freshly received message's controller until a run claims it.
    ///
    /// A controller displaced by a duplicate registration or by the pending cap
    /// is finalized rather than dropped: dropping its handle would close the
    /// worker's channel, which exits without touching the message and would
    /// leave 👀 on it until the lifetime cap.
    pub async fn register_pending(&self, key: String, controller: Arc<ChannelReactionController>) {
        let displaced = {
            let mut inner = self.inner.lock().await;
            let mut displaced = Vec::new();
            if let Some(previous) = inner.pending.insert(key.clone(), controller) {
                displaced.push(previous);
            } else {
                inner.pending_order.push_back(key.clone());
            }
            while inner.pending_order.len() > self.pending_limit {
                let Some(oldest) = inner.pending_order.pop_front() else {
                    break;
                };
                if let Some(evicted) = inner.pending.remove(&oldest) {
                    displaced.push(evicted);
                }
            }
            displaced
        };
        for controller in displaced {
            controller
                .note(ChannelActivity::Finished(ChannelAckOutcome::Cancelled))
                .await;
        }
    }

    /// Transfer the given acknowledgments to this now-executing run.
    pub async fn activate(&self, activity_id: &str, session_key: &str, keys: Vec<String>) {
        if keys.is_empty() {
            return;
        }
        let displaced = {
            let mut inner = self.inner.lock().await;
            let acknowledgments = keys
                .into_iter()
                .filter_map(|key| {
                    inner.pending_order.retain(|pending| pending != &key);
                    inner
                        .pending
                        .remove(&key)
                        .map(|controller| ActiveAck { key, controller })
                })
                .collect::<Vec<_>>();
            if acknowledgments.is_empty() {
                return;
            }

            let mut displaced = Vec::new();
            // This is a cleanup threshold rather than a concurrency cap: live
            // workers may overlap, so only turns beyond their worker lifetime
            // are eligible for eviction.
            if inner.active.len() >= self.active_limit {
                let now = Instant::now();
                let stale_ids = inner
                    .active
                    .iter()
                    .filter(|(_, turn)| {
                        now.duration_since(turn.activated_at) >= self.active_lifetime
                    })
                    .map(|(activity_id, _)| activity_id.clone())
                    .collect::<Vec<_>>();
                for stale_id in stale_ids {
                    if let Some(stale) = remove_active(&mut inner, &stale_id) {
                        displaced.extend(stale.acknowledgments);
                    }
                }
            }
            if let Some(previous_id) = inner
                .active_by_session
                .get(session_key)
                .filter(|previous_id| previous_id.as_str() != activity_id)
                .cloned()
                && let Some(previous) = remove_active(&mut inner, &previous_id)
            {
                displaced.extend(previous.acknowledgments);
            }
            if let Some(previous) = remove_active(&mut inner, activity_id) {
                displaced.extend(previous.acknowledgments);
            }

            inner.active.insert(activity_id.to_string(), ActiveTurn {
                session_key: session_key.to_string(),
                acknowledgments,
                activated_at: Instant::now(),
            });
            inner
                .active_by_session
                .insert(session_key.to_string(), activity_id.to_string());
            displaced
        };
        finish_acks(displaced, ChannelAckOutcome::Cancelled).await;
    }

    /// Forward an activity to every acknowledgment this exact run owns.
    ///
    /// A terminal detaches the turn before any controller I/O, so later activity
    /// cannot race behind it and a delayed terminal cannot address a newer run.
    pub async fn note(&self, activity_id: &str, activity: ChannelActivity) {
        let is_terminal = matches!(activity, ChannelActivity::Finished(_));
        let controllers = {
            let mut inner = self.inner.lock().await;
            if is_terminal {
                remove_active(&mut inner, activity_id)
                    .map(|turn| turn.acknowledgments)
                    .unwrap_or_default()
            } else {
                inner
                    .active
                    .get(activity_id)
                    .map(|turn| {
                        turn.acknowledgments
                            .iter()
                            .map(|ack| ActiveAck {
                                key: ack.key.clone(),
                                controller: Arc::clone(&ack.controller),
                            })
                            .collect()
                    })
                    .unwrap_or_default()
            }
        };

        for ack in controllers {
            ack.controller.note(activity.clone()).await;
        }
    }

    /// Finalize the exact active turn identified by `activity_id`.
    ///
    /// Used by abort, where the run future is killed and cannot signal its own
    /// terminal. Taking the turn under the lock ensures a later turn in the
    /// same session can never be caught by this call.
    pub async fn finalize_active(&self, activity_id: &str, outcome: ChannelAckOutcome) {
        let acknowledgments = {
            let mut inner = self.inner.lock().await;
            remove_active(&mut inner, activity_id)
                .map(|turn| turn.acknowledgments)
                .unwrap_or_default()
        };
        finish_acks(acknowledgments, outcome).await;
    }

    /// Finalize specific acknowledgments directly, for paths where no run ever
    /// executes (a rejected hook, an early error) and nothing will signal a
    /// terminal for them.
    pub async fn finalize_keys(&self, keys: &[String], outcome: ChannelAckOutcome) {
        let controllers = {
            let mut inner = self.inner.lock().await;
            let mut controllers = keys
                .iter()
                .filter_map(|k| {
                    inner.pending_order.retain(|pk| pk != k);
                    inner.pending.remove(k)
                })
                .collect::<Vec<_>>();
            let activity_ids = inner.active.keys().cloned().collect::<Vec<_>>();
            for activity_id in activity_ids {
                let Some(mut turn) = remove_active(&mut inner, &activity_id) else {
                    continue;
                };
                let mut kept = Vec::new();
                for ack in turn.acknowledgments {
                    if keys.contains(&ack.key) {
                        controllers.push(ack.controller);
                    } else {
                        kept.push(ack);
                    }
                }
                if !kept.is_empty() {
                    turn.acknowledgments = kept;
                    inner
                        .active_by_session
                        .insert(turn.session_key.clone(), activity_id.clone());
                    inner.active.insert(activity_id, turn);
                }
            }
            controllers
        };
        for controller in controllers {
            controller.note(ChannelActivity::Finished(outcome)).await;
        }
    }
}

fn remove_active(inner: &mut RegistryInner, activity_id: &str) -> Option<ActiveTurn> {
    let turn = inner.active.remove(activity_id)?;
    if inner
        .active_by_session
        .get(&turn.session_key)
        .map(String::as_str)
        == Some(activity_id)
    {
        inner.active_by_session.remove(&turn.session_key);
    }
    Some(turn)
}

async fn finish_acks(acknowledgments: Vec<ActiveAck>, outcome: ChannelAckOutcome) {
    for ack in acknowledgments {
        ack.controller
            .note(ChannelActivity::Finished(outcome))
            .await;
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
                        // The worker exits immediately after this, so a failed
                        // terminal is permanent — unlike an intermediate phase,
                        // which the next transition would supersede. Retry once.
                        let mut landed =
                            add_reaction(&outbound, &account_id, &chat_id, &message_id, term).await;
                        if !landed {
                            tokio::time::sleep(TERMINAL_RETRY_DELAY).await;
                            landed =
                                add_reaction(&outbound, &account_id, &chat_id, &message_id, term)
                                    .await;
                        }
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
                // The controller owner disappeared without a terminal. Strip
                // the in-progress marker rather than leaking it indefinitely.
                if let Some(cur) = current.take() {
                    remove_reaction(&outbound, &account_id, &chat_id, &message_id, &cur).await;
                }
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

/// Retract a reaction, tolerating failure.
///
/// Every call here sits on a path that returns immediately afterwards — the
/// terminal swap, cancellation, the lifetime safety net — so a channel that
/// reports failure has already exhausted whatever retries it has (Nostr
/// publishes a NIP-09 deletion and retries the transient cases itself). There is
/// no later attempt to schedule, which is exactly why the failure is worth
/// surfacing: the acknowledgment on the user's message is now wrong, visibly and
/// permanently, and nothing else is coming to fix it.
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
        warn!(
            account_id,
            chat_id, emoji, "channel reaction could not be retracted, it may stay visible: {e}"
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
        /// Report every retraction as failed, like a channel whose own retries
        /// are exhausted (Nostr, once a relay keeps refusing the deletion).
        retraction_fails: bool,
    }

    impl RecordingOutbound {
        fn new(ops: Arc<StdMutex<Vec<String>>>) -> Self {
            Self {
                ops,
                retraction_fails: false,
            }
        }

        fn failing_retractions(ops: Arc<StdMutex<Vec<String>>>) -> Self {
            Self {
                ops,
                retraction_fails: true,
            }
        }
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
            if self.retraction_fails {
                return Err(moltis_channels::Error::unavailable("retraction refused"));
            }
            Ok(())
        }
    }

    async fn run_lifecycle(outcome: ChannelAckOutcome) -> Vec<String> {
        let ops = Arc::new(StdMutex::new(Vec::new()));
        let outbound = Arc::new(RecordingOutbound::new(ops.clone()));
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

    /// A channel that cannot retract must not take the turn down with it.
    ///
    /// By the time a retraction is reported as failed the channel has exhausted
    /// its own retries, so there is nothing left to attempt here — the worker
    /// logs it and finishes. What matters is that the terminal ✅ still lands
    /// (the run's outcome is reported even though the 👀 is stuck beside it) and
    /// that the worker exits rather than hanging on the failure.
    #[tokio::test]
    async fn a_retraction_the_channel_refuses_still_leaves_the_terminal() {
        let ops = Arc::new(StdMutex::new(Vec::new()));
        let outbound = Arc::new(RecordingOutbound::failing_retractions(ops.clone()));
        let controller =
            ChannelReactionController::start(outbound, "a".into(), "c".into(), "m".into());
        tokio::time::sleep(Duration::from_millis(50)).await;
        controller
            .note(ChannelActivity::Finished(ChannelAckOutcome::Success))
            .await;
        tokio::time::sleep(Duration::from_millis(50)).await;

        // The retraction was attempted and refused; 👀 stays on the message,
        // which is why the failure is logged at `warn!` rather than swallowed.
        assert_eq!(ops_of(&ops), vec!["+👀", "+✅", "-👀"]);
    }

    #[tokio::test]
    async fn tool_phase_swaps_marker_then_terminal() {
        let ops = Arc::new(StdMutex::new(Vec::new()));
        let outbound = Arc::new(RecordingOutbound::new(ops.clone()));
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
        let outbound = Arc::new(RecordingOutbound::new(ops.clone()));
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
        registry
            .activate("activity-a", "sess", vec!["msg-a".into()])
            .await;

        registry
            .note(
                "activity-a",
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
            .activate("activity-a", "sess", vec!["msg-a".into(), "msg-b".into()])
            .await;

        registry
            .note(
                "activity-a",
                ChannelActivity::Finished(ChannelAckOutcome::Success),
            )
            .await;
        tokio::time::sleep(Duration::from_millis(60)).await;

        for ops in [&a, &b] {
            assert_eq!(ops_of(ops), vec!["+👀", "+✅", "-👀"]);
        }
    }

    #[tokio::test]
    async fn pending_limit_evicts_only_after_crossing_the_boundary() {
        let registry = ReactionRegistry {
            pending_limit: 1,
            ..ReactionRegistry::default()
        };
        let oldest = park(&registry, "msg-oldest").await;
        assert_eq!(ops_of(&oldest), vec!["+👀"]);

        let newest = park(&registry, "msg-newest").await;
        tokio::time::sleep(Duration::from_millis(60)).await;

        assert_eq!(ops_of(&oldest), vec!["+👀", "-👀"]);
        assert_eq!(ops_of(&newest), vec!["+👀"]);
    }

    #[tokio::test]
    async fn superseded_turn_terminal_does_not_clear_the_current_turn() {
        // Turn 1 activates, is superseded by turn 2, then turn 1's terminal
        // arrives late. It must not release turn 2's acknowledgment.
        let registry = ReactionRegistry::default();
        let a = park(&registry, "msg-a").await;
        registry
            .activate("activity-1", "sess", vec!["msg-a".into()])
            .await;
        let b = park(&registry, "msg-b").await;
        registry
            .activate("activity-2", "sess", vec!["msg-b".into()])
            .await;

        registry
            .note(
                "activity-1",
                ChannelActivity::Finished(ChannelAckOutcome::Success),
            )
            .await;
        tokio::time::sleep(Duration::from_millis(60)).await;
        assert_eq!(ops_of(&a), vec!["+👀", "-👀"]);
        assert_eq!(ops_of(&b), vec!["+👀"], "late terminal touched turn 2");

        registry
            .note(
                "activity-2",
                ChannelActivity::Finished(ChannelAckOutcome::Success),
            )
            .await;
        tokio::time::sleep(Duration::from_millis(60)).await;

        assert_eq!(ops_of(&b), vec!["+👀", "+✅", "-👀"]);
    }

    #[tokio::test]
    async fn active_cleanup_threshold_does_not_cancel_healthy_live_turns() {
        let registry = ReactionRegistry {
            active_limit: 1,
            ..ReactionRegistry::default()
        };
        let a = park(&registry, "msg-a").await;
        let b = park(&registry, "msg-b").await;
        registry
            .activate("activity-a", "sess-a", vec!["msg-a".into()])
            .await;
        registry
            .activate("activity-b", "sess-b", vec!["msg-b".into()])
            .await;
        tokio::time::sleep(Duration::from_millis(60)).await;

        assert_eq!(ops_of(&a), vec!["+👀"], "healthy turn was evicted");
        assert_eq!(ops_of(&b), vec!["+👀"]);

        registry
            .note(
                "activity-a",
                ChannelActivity::Finished(ChannelAckOutcome::Success),
            )
            .await;
        registry
            .note(
                "activity-b",
                ChannelActivity::Finished(ChannelAckOutcome::Success),
            )
            .await;
        tokio::time::sleep(Duration::from_millis(60)).await;
        assert_eq!(ops_of(&a), vec!["+👀", "+✅", "-👀"]);
        assert_eq!(ops_of(&b), vec!["+👀", "+✅", "-👀"]);
    }

    #[tokio::test]
    async fn active_cleanup_threshold_cancels_only_expired_turns() {
        let registry = ReactionRegistry {
            active_limit: 1,
            active_lifetime: Duration::ZERO,
            ..ReactionRegistry::default()
        };
        let stale = park(&registry, "msg-stale").await;
        let current = park(&registry, "msg-current").await;
        registry
            .activate("activity-stale", "sess-stale", vec!["msg-stale".into()])
            .await;
        registry
            .activate("activity-current", "sess-current", vec![
                "msg-current".into(),
            ])
            .await;
        tokio::time::sleep(Duration::from_millis(60)).await;

        assert_eq!(ops_of(&stale), vec!["+👀", "-👀"]);
        assert_eq!(ops_of(&current), vec!["+👀"]);
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
    async fn duplicate_registration_resolves_the_displaced_controller() {
        // A retry of the same message must not strand the first 👀.
        let registry = ReactionRegistry::default();
        let first = park(&registry, "msg-a").await;
        let second_ops = Arc::new(StdMutex::new(Vec::new()));
        let outbound = Arc::new(RecordingOutbound::new(second_ops.clone()));
        let replacement =
            ChannelReactionController::start(outbound, "a".into(), "c".into(), "msg-a".into());
        registry
            .register_pending("msg-a".to_string(), replacement)
            .await;
        tokio::time::sleep(Duration::from_millis(60)).await;
        // Displaced controller cleared its marker instead of leaking it.
        assert_eq!(ops_of(&first), vec!["+👀", "-👀"]);
        assert_eq!(ops_of(&second_ops), vec!["+👀"]);
    }

    #[tokio::test]
    async fn activity_for_unknown_id_is_a_noop() {
        let registry = ReactionRegistry::default();
        let a = park(&registry, "msg-a").await;
        // Never activated — a stray activity must not touch the parked ack.
        registry
            .note(
                "other-activity",
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
