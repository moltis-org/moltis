// ── Chat event handler functions ──────────────────────────────

import {
	appendReasoningDisclosure,
	chatAddErrorCard,
	chatAddErrorMsg,
	chatAddMsg,
	removeThinking,
	smartScrollToBottom,
	updateTokenBar,
} from "../chat-ui";
import { highlightCodeBlocks } from "../code-highlight";
import { localizeStructuredError, renderAudioPlayer, renderMarkdown } from "../helpers";
import { t } from "../i18n";
import { maybeRefreshFullContext, renderCompactCard } from "../pages/ChatPage";
import { currentPrefix } from "../router";
import {
	appendLastMessageTimestamp,
	bumpSessionCount,
	cacheSessionHistoryMessage,
	clearSessionHistoryCache,
	fetchSessions,
	markSessionLocallyCleared,
	setSessionActiveRunId,
	setSessionReplying,
	setSessionUnread,
} from "../sessions";
import * as S from "../state";
import { sessionStore } from "../stores/session-store";
import type { AbortedPartialState, ChatPayload, CompactPayload, ToolCallPayload } from "../types/ws-events";
import {
	clearChatEmptyState,
	hasNonWhitespaceContent,
	isReasoningAlreadyShown,
	makeThinkingDots,
	makeThinkingStopBtn,
	moveFirstQueuedToChat,
	setSafeMarkdownHtml,
	updateSessionHistoryIndex,
	updateSessionRunId,
} from "./shared";
import {
	appendFinalFooter,
	clearPendingToolCallEndsForSession,
	clearStaleRunningToolCards,
	completeToolCard,
	handleToolCallStartDom,
	pendingToolCallEnds,
	renderAbortedPartialInDom,
	renderChannelUserMessage,
	resolveFinalMessageEl,
	toolCallCardId,
	toolCallEventKey,
} from "./tool-helpers";

export type ChatHandler = (p: ChatPayload, isActive: boolean, isChatPage: boolean, eventSession: string) => void;

// ── Individual chat event handlers ────────────────────────────

function handleChatThinking(p: ChatPayload, isActive: boolean, isChatPage: boolean, eventSession: string): void {
	updateSessionRunId(eventSession, p.runId);
	setSessionReplying(eventSession, true);
	if (!(isActive && isChatPage)) return;
	removeThinking();
	clearChatEmptyState();
	const thinkEl = document.createElement("div");
	thinkEl.className = "msg assistant thinking";
	thinkEl.id = "thinkingIndicator";
	thinkEl.appendChild(makeThinkingDots());
	thinkEl.appendChild(makeThinkingStopBtn(eventSession));
	S.chatMsgBox?.appendChild(thinkEl);
	smartScrollToBottom();
}

function handleChatThinkingText(p: ChatPayload, isActive: boolean, isChatPage: boolean, eventSession: string): void {
	updateSessionRunId(eventSession, p.runId);
	setSessionReplying(eventSession, true);
	if (!(isActive && isChatPage)) return;
	const indicator = document.getElementById("thinkingIndicator");
	if (indicator) {
		const existingBtn = indicator.querySelector(".thinking-stop-btn");
		while (indicator.firstChild) indicator.removeChild(indicator.firstChild);
		const textEl = document.createElement("span");
		textEl.className = "thinking-text";
		textEl.textContent = p.text || "";
		indicator.appendChild(textEl);
		indicator.appendChild(existingBtn || makeThinkingStopBtn(eventSession));
		smartScrollToBottom();
	}
}

function handleChatThinkingDone(_p: ChatPayload, isActive: boolean, isChatPage: boolean): void {
	// Don't remove the thinking indicator here. It will be removed by either:
	// - handleChatDelta (when text starts streaming)
	// - handleChatToolCallStart (which preserves thinking text as a disclosure)
	// - handleChatFinal / handleChatError (cleanup)
	// This keeps the thinking text visible until we know whether to preserve it.
	void (isActive && isChatPage);
}

function handleChatVoicePending(_p: ChatPayload, isActive: boolean, isChatPage: boolean, eventSession: string): void {
	// Update per-session signal
	const session = sessionStore.getByKey(eventSession);
	if (session) session.voicePending.value = true;
	if (!(isActive && isChatPage)) return;
	// Dual-write to global state for backward compat
	S.setVoicePending(true);
	// Keep the existing thinking dots visible -- no separate voice indicator.
}

function handleChatToolCallStart(p: ChatPayload, isActive: boolean, isChatPage: boolean, eventSession: string): void {
	updateSessionRunId(eventSession, p.runId);
	// Update per-session signal
	const session = sessionStore.getByKey(eventSession);
	if (session) session.streamText.value = "";
	if (!(isActive && isChatPage)) return;
	handleToolCallStartDom(p, eventSession);
}

function handleChatToolCallEnd(p: ChatPayload, isActive: boolean, isChatPage: boolean, eventSession: string): void {
	updateSessionRunId(eventSession, p.runId);
	// Always bump badge -- the server persists both the hidden assistant
	// tool-call frame and the visible tool_result for each completed call.
	bumpSessionCount(eventSession, 2);
	let toolHistoryIndex: number | undefined | null = p.messageIndex;
	if (toolHistoryIndex === undefined || toolHistoryIndex === null) {
		const toolSession = sessionStore.getByKey(eventSession);
		if (toolSession && typeof toolSession.messageCount === "number" && toolSession.messageCount > 0) {
			toolHistoryIndex = toolSession.messageCount - 1;
		}
	}
	cacheSessionHistoryMessage(
		eventSession,
		{
			role: "tool_result",
			tool_call_id: p.toolCallId || "",
			tool_name: p.toolName || "",
			success: p.success === true,
			result: p.result || null,
			error: p.error?.detail || p.error?.message || (typeof p.error === "string" ? String(p.error) : null),
			created_at: Date.now(),
		},
		toolHistoryIndex as number | undefined,
	);
	updateSessionHistoryIndex(eventSession, toolHistoryIndex as number | undefined);
	if (!(isActive && isChatPage)) return;
	const toolCard = document.getElementById(toolCallCardId(p));
	if (!toolCard) {
		pendingToolCallEnds.set(toolCallEventKey(eventSession, p), p as ToolCallPayload);
		return;
	}
	completeToolCard(toolCard, p, eventSession);
}

function handleChatChannelUser(p: ChatPayload, isActive: boolean, isChatPage: boolean, eventSession: string): void {
	// Always bump the badge so the total message count stays accurate,
	// even when the user is not on the chat page (e.g. Telegram messages).
	bumpSessionCount(eventSession, 1);
	const cachedAudio = p.channel?.audio_filename
		? `media/${eventSession.replaceAll(":", "_")}/${p.channel.audio_filename}`
		: undefined;
	cacheSessionHistoryMessage(
		eventSession,
		{
			role: "user",
			content: p.text || "",
			channel: p.channel || null,
			audio: cachedAudio,
			created_at: Date.now(),
		},
		p.messageIndex,
	);
	if (!isActive) {
		setSessionUnread(eventSession, true);
	}
	if (!(isChatPage && isActive)) {
		updateSessionHistoryIndex(eventSession, p.messageIndex);
		return;
	}
	renderChannelUserMessage(p, eventSession);
	updateSessionHistoryIndex(eventSession, p.messageIndex);
}

// Handle user messages broadcast by the backend after persisting a message
// sent via the GraphQL API, mobile app, or any non-web-UI client.
// The originating web client already rendered the message optimistically,
// so we skip rendering when the broadcast's seq matches a seq this client
// has already sent (seq <= S.chatSeq).
function handleChatUserMessage(p: ChatPayload, isActive: boolean, isChatPage: boolean, eventSession: string): void {
	// Suppress the echo for the originating client.
	if (p.seq !== undefined && p.seq !== null && p.seq <= S.chatSeq) return;

	bumpSessionCount(eventSession, 1);
	cacheSessionHistoryMessage(
		eventSession,
		{
			role: "user",
			content: p.text || "",
			created_at: Date.now(),
		},
		p.messageIndex,
	);
	if (!isActive) {
		setSessionUnread(eventSession, true);
	}
	if (!(isChatPage && isActive)) return;
	// Safe: renderMarkdown calls esc() first -- all user input is
	// HTML-escaped before formatting tags are applied.
	chatAddMsg("user", renderMarkdown(p.text || ""), true);
}

function handleChatDelta(p: ChatPayload, isActive: boolean, isChatPage: boolean, eventSession: string): void {
	updateSessionRunId(eventSession, p.runId);
	if (!p.text) return;
	// Update per-session signal
	const session = sessionStore.getByKey(eventSession);
	if (session) session.streamText.value += p.text;
	if (!(isActive && isChatPage)) return;
	// When voice is pending, accumulate text silently without rendering.
	if (S.voicePending) {
		S.setStreamText(S.streamText + p.text);
		return;
	}
	// Skip leading whitespace before any real content has been streamed.
	// Some providers emit newlines between thinking and content; rendering
	// them would create an empty assistant div that lingers if a tool call
	// follows immediately.  We must check this BEFORE removeThinking() so
	// the thinking text is still available for handleChatToolCallStart to
	// extract into a reasoning disclosure on the tool card.
	if (!(S.streamEl || p.text.trim())) return;
	removeThinking();
	if (!S.streamEl) {
		S.setStreamText("");
		S.setStreamEl(document.createElement("div"));
		S.streamEl!.className = "msg assistant";
		clearChatEmptyState();
		S.chatMsgBox?.appendChild(S.streamEl!);
	}
	S.setStreamText(S.streamText + p.text);
	setSafeMarkdownHtml(S.streamEl!, S.streamText);
	smartScrollToBottom();
}

// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: Final message handling with audio/voice branching
function handleChatFinal(p: ChatPayload, isActive: boolean, isChatPage: boolean, eventSession: string): void {
	clearPendingToolCallEndsForSession(eventSession);
	updateSessionRunId(eventSession, p.runId);
	// Always bump badge -- the server persists the final assistant message.
	bumpSessionCount(eventSession, 1);
	const finalText = String(p.text || "");
	const hasVisibleFinal =
		hasNonWhitespaceContent(finalText) ||
		hasNonWhitespaceContent(p.reasoning || "") ||
		hasNonWhitespaceContent(p.audio || "");
	if (hasVisibleFinal) {
		cacheSessionHistoryMessage(
			eventSession,
			{
				role: "assistant",
				content: finalText,
				model: p.model || "",
				provider: p.provider || "",
				inputTokens: p.inputTokens || 0,
				outputTokens: p.outputTokens || 0,
				cacheReadTokens: p.cacheReadTokens || 0,
				cacheWriteTokens: p.cacheWriteTokens || 0,
				durationMs: p.durationMs || 0,
				requestInputTokens: p.requestInputTokens,
				requestOutputTokens: p.requestOutputTokens,
				requestCacheReadTokens: p.requestCacheReadTokens,
				requestCacheWriteTokens: p.requestCacheWriteTokens,
				reasoning: p.reasoning || undefined,
				audio: p.audio || undefined,
				run_id: p.runId || undefined,
				created_at: Date.now(),
			},
			p.messageIndex,
		);
	}
	// Compare against the per-session history index so cross-session
	// events aren't wrongly skipped by another session's index.
	const evtSession = sessionStore.getByKey(eventSession);
	const lastIdx = evtSession ? evtSession.lastHistoryIndex.value : S.lastHistoryIndex;
	if (p.messageIndex !== undefined && p.messageIndex <= lastIdx) {
		setSessionReplying(eventSession, false);
		setSessionActiveRunId(eventSession, null);
		return;
	}
	updateSessionHistoryIndex(eventSession, p.messageIndex);
	setSessionReplying(eventSession, false);
	setSessionActiveRunId(eventSession, null);
	if (!isActive) {
		setSessionUnread(eventSession, true);
	}
	if (!(isActive && isChatPage)) {
		S.setVoicePending(false);
		return;
	}
	removeThinking();
	clearStaleRunningToolCards();

	if (S.voicePending && p.text && p.replyMedium === "voice") {
		// Voice pending path: we suppressed streaming, so render everything at once.
		console.debug("[audio] voice-pending path, audio:", !!p.audio, "text:", p.text.substring(0, 40));
		const msgEl = S.streamEl || document.createElement("div");
		msgEl.className = "msg assistant";
		msgEl.textContent = "";
		if (!msgEl.parentNode) {
			clearChatEmptyState();
			S.chatMsgBox?.appendChild(msgEl);
		}

		if (p.audio) {
			const filename = p.audio.split("/").pop() || "";
			const audioSrc = `/api/sessions/${encodeURIComponent(p.sessionKey || S.activeSessionKey)}/media/${encodeURIComponent(filename)}`;
			console.debug("[audio] rendering persisted audio:", filename);
			renderAudioPlayer(msgEl, audioSrc, true);
		}
		if (hasNonWhitespaceContent(p.text)) {
			// Safe: renderMarkdown calls esc() first -- all user input is HTML-escaped.
			const textWrap = document.createElement("div");
			textWrap.className = "mt-2";
			setSafeMarkdownHtml(textWrap, p.text);
			msgEl.appendChild(textWrap);
		}
		if (p.reasoning && !isReasoningAlreadyShown(p.reasoning)) {
			appendReasoningDisclosure(msgEl, p.reasoning);
		}
		appendFinalFooter(msgEl, p, eventSession);
		smartScrollToBottom();
	} else {
		let resolvedEl = resolveFinalMessageEl(p);
		const skipReasoning = p.reasoning && isReasoningAlreadyShown(p.reasoning);
		if (!resolvedEl && p.reasoning && !skipReasoning) {
			resolvedEl = chatAddMsg("assistant", "", false);
		}
		if (resolvedEl && p.reasoning && !skipReasoning) {
			appendReasoningDisclosure(resolvedEl, p.reasoning);
		}
		if (resolvedEl && p.text && p.replyMedium === "voice") {
			console.debug(
				"[audio] streamed path, audio:",
				!!p.audio,
				"voicePending:",
				S.voicePending,
				"text:",
				p.text.substring(0, 40),
			);
			if (p.audio) {
				const fn2 = p.audio.split("/").pop() || "";
				const src2 = `/api/sessions/${encodeURIComponent(p.sessionKey || S.activeSessionKey)}/media/${encodeURIComponent(fn2)}`;
				console.debug("[audio] rendering persisted audio (streamed):", fn2);
				resolvedEl.textContent = "";
				renderAudioPlayer(resolvedEl, src2, true);
				appendFinalFooter(resolvedEl, p, eventSession);
			} else {
				console.debug("[audio] no persisted audio, showing voice fallback action");
				appendFinalFooter(resolvedEl, p, eventSession);
			}
		} else {
			// Silent reply -- attach footer to the last visible assistant element
			// (e.g. exec card). Never attach to a user message.
			let target = resolvedEl;
			if (!target) {
				const last = S.chatMsgBox?.lastElementChild as HTMLElement | null;
				if (last && !last.classList.contains("user")) target = last;
			}
			appendFinalFooter(target, p, eventSession);
		}
	}
	if (p.inputTokens || p.outputTokens) {
		S.sessionTokens.input += p.inputTokens || 0;
		S.sessionTokens.output += p.outputTokens || 0;
	}
	if (p.requestInputTokens !== undefined && p.requestInputTokens !== null) {
		S.setSessionCurrentInputTokens(p.requestInputTokens || 0);
	} else if (p.inputTokens || p.outputTokens) {
		S.setSessionCurrentInputTokens(p.inputTokens || 0);
	}
	updateTokenBar();
	appendLastMessageTimestamp(Date.now());
	// Reset per-session stream state
	const finalSession = sessionStore.getByKey(eventSession);
	if (finalSession) finalSession.resetStreamState();
	// Dual-write to global state for backward compat
	S.setStreamEl(null);
	S.setStreamText("");
	S.setLastToolOutput("");
	S.setVoicePending(false);
	maybeRefreshFullContext();
	// Syntax-highlight any code blocks in the completed message.
	if (S.chatMsgBox?.lastElementChild) {
		highlightCodeBlocks(S.chatMsgBox.lastElementChild as HTMLElement);
	}
	// Move the next queued message from the tray AFTER the response is
	// fully rendered. This ensures correct ordering: user-msg -> response ->
	// next-user-msg -> next-response (never next-user-msg before response).
	moveFirstQueuedToChat();
}

// ── Compact handling ──────────────────────────────────────────

// Shared debounce so the auto-compact path (which broadcasts both
// `chat.compact done` from within ChatService::compact AND a wrapping
// `auto_compact done` from the send() caller) renders the card exactly
// once. Whichever event arrives first claims the render; the other is
// a no-op within the debounce window.
const COMPACT_CARD_DEBOUNCE_MS = 500;
const lastCompactCardAt: Map<string, number> = new Map();

// Per-session reference to the "Compacting conversation..." status message
// appended on `auto_compact start`. Tracked explicitly (not via
// `lastChild`) because `send()`'s pre-emptive auto-compact path
// interleaves `chat.compact done` between `auto_compact start` and
// `auto_compact done`, which means the old "remove lastChild" pattern
// would remove the compact card instead of the status message.
// Greptile P1 on commit 0531913b.
const compactingStatusElements: Map<string, HTMLElement> = new Map();

function shouldRenderCompactCard(p: CompactPayload): boolean {
	const key = p.sessionKey || "__active__";
	const now = Date.now();
	const previous = lastCompactCardAt.get(key) || 0;
	if (now - previous < COMPACT_CARD_DEBOUNCE_MS) {
		return false;
	}
	lastCompactCardAt.set(key, now);
	return true;
}

// Drop the "Compacting conversation..." status message the auto-compact
// start phase appended for this session, if one exists. Called by both
// compact-done handlers before rendering the card so the status message
// never outlives its purpose, regardless of which event arrives first.
function removeCompactingStatus(p: CompactPayload): void {
	const key = p.sessionKey || "__active__";
	const el = compactingStatusElements.get(key);
	compactingStatusElements.delete(key);
	if (el && el.parentNode === S.chatMsgBox) {
		S.chatMsgBox?.removeChild(el);
	}
}

function resetTokensAfterCompaction(): void {
	S.setSessionTokens({ input: 0, output: 0 });
	S.setSessionCurrentInputTokens(0);
	updateTokenBar();
}

function handleChatAutoCompact(p: ChatPayload, isActive: boolean, isChatPage: boolean): void {
	if (!(isActive && isChatPage)) return;
	if (p.phase === "start") {
		const statusEl = chatAddMsg("system", "Compacting conversation (context limit reached)\u2026");
		const key = p.sessionKey || "__active__";
		if (statusEl) {
			compactingStatusElements.set(key, statusEl);
		}
	} else if (p.phase === "done") {
		// Always drop the status message -- even when the card was
		// already rendered by an earlier `chat.compact done` event.
		removeCompactingStatus(p as CompactPayload);
		if (shouldRenderCompactCard(p as CompactPayload)) {
			renderCompactCard(p);
		}
		resetTokensAfterCompaction();
	} else if (p.phase === "error") {
		removeCompactingStatus(p as CompactPayload);
		chatAddMsg("error", `Auto-compact failed: ${p.error?.message || p.error?.detail || "unknown error"}`);
	}
}

// `chat.compact done` is emitted by ChatService::compact on every
// compaction run (manual `/compact` RPCs AND the pre-emptive auto-
// compact path). It carries the mode/tokens/settings metadata from
// CompactionOutcome::broadcast_metadata() so the same card renders.
function handleChatCompact(p: ChatPayload, isActive: boolean, isChatPage: boolean): void {
	if (!(isActive && isChatPage)) return;
	if (p.phase !== "done") return;
	// Drop the auto-compact status message if one exists. For the
	// manual `/compact` RPC path there is no status message, so this
	// is a no-op. For `send()`'s pre-emptive auto-compact path,
	// `chat.compact done` arrives BEFORE `auto_compact done`, so we
	// clear the status message here; the subsequent `auto_compact done`
	// handler will find the slot already empty.
	removeCompactingStatus(p as CompactPayload);
	if (!shouldRenderCompactCard(p as CompactPayload)) return;
	renderCompactCard(p);
	resetTokensAfterCompaction();
}

// ── Retry handling ────────────────────────────────────────────

function retryDelayMsFromPayload(p: ChatPayload): number {
	if (p.retryAfterMs !== undefined && p.retryAfterMs !== null) return Number(p.retryAfterMs) || 0;
	if (p.error?.retryAfterMs !== undefined && p.error?.retryAfterMs !== null) return Number(p.error.retryAfterMs) || 0;
	return 0;
}

function retryStatusText(p: ChatPayload): string {
	const retryMs = retryDelayMsFromPayload(p);
	const retrySecs = Math.max(1, Math.ceil(retryMs / 1000));
	const rateLimited = p.error?.type === "rate_limit_exceeded";
	return rateLimited
		? `Rate limited by provider, retrying in ${retrySecs}s\u2026`
		: `Temporary provider issue, retrying in ${retrySecs}s\u2026`;
}

function handleChatRetrying(p: ChatPayload, isActive: boolean, isChatPage: boolean, eventSession: string): void {
	updateSessionRunId(eventSession, p.runId);
	setSessionReplying(eventSession, true);
	if (!(isActive && isChatPage)) return;

	let indicator = document.getElementById("thinkingIndicator");
	if (!indicator) {
		removeThinking();
		indicator = document.createElement("div");
		indicator.className = "msg assistant thinking";
		indicator.id = "thinkingIndicator";
		indicator.appendChild(makeThinkingDots());
		clearChatEmptyState();
		S.chatMsgBox?.appendChild(indicator);
	}

	while (indicator.firstChild) indicator.removeChild(indicator.firstChild);
	const textEl = document.createElement("span");
	textEl.className = "thinking-text";
	textEl.textContent = retryStatusText(p);
	indicator.appendChild(textEl);
	smartScrollToBottom();
}

// ── Error / abort / notice / clear ────────────────────────────

function handleChatError(p: ChatPayload, isActive: boolean, isChatPage: boolean, eventSession: string): void {
	clearPendingToolCallEndsForSession(eventSession);
	setSessionReplying(eventSession, false);
	setSessionActiveRunId(eventSession, null);
	// Reset per-session stream state
	const errSession = sessionStore.getByKey(eventSession);
	if (errSession) errSession.resetStreamState();
	if (!(isActive && isChatPage)) {
		S.setVoicePending(false);
		return;
	}
	removeThinking();
	clearStaleRunningToolCards();
	if (p.error?.title) {
		chatAddErrorCard(localizeStructuredError(p.error) as Parameters<typeof chatAddErrorCard>[0]);
	} else {
		chatAddErrorMsg(p.message || "unknown");
	}
	// Add continue button for max_iterations_reached errors.
	if (p.error?.canContinue) {
		const lastCard = S.chatMsgBox?.querySelector(".error-card:last-child") as HTMLElement | null;
		if (lastCard) {
			const btn = document.createElement("button");
			btn.className = "provider-btn error-continue-btn";
			btn.textContent = t("errors:chat.continue", "Continue");
			btn.onclick = () => {
				btn.disabled = true;
				btn.textContent = t("errors:chat.continuing", "Continuing...");
				(S.chatInput as HTMLInputElement).value = t(
					"errors:chat.continueMessage",
					"Please continue where you left off.",
				);
				// Trigger send by clicking the chat send button (sendChat is local to ChatPage)
				S.chatSendBtn?.click();
			};
			const body = lastCard.querySelector(".error-body");
			if (body) body.appendChild(btn);
		}
	}
	S.setStreamEl(null);
	S.setStreamText("");
	S.setVoicePending(false);
	moveFirstQueuedToChat();
}

function getAbortedPartialState(p: ChatPayload): AbortedPartialState {
	const partial = p.partialMessage && typeof p.partialMessage === "object" ? p.partialMessage : null;
	const partialText = String(partial?.content || "");
	const partialReasoning = String(partial?.reasoning || "");
	return {
		partial,
		partialText,
		partialReasoning,
		hasVisiblePartial: hasNonWhitespaceContent(partialText) || hasNonWhitespaceContent(partialReasoning),
	};
}

function cacheAbortedPartial(
	eventSession: string,
	p: ChatPayload,
	abortSession: ReturnType<typeof sessionStore.getByKey>,
	partialState: AbortedPartialState,
): void {
	if (!partialState.hasVisiblePartial) return;
	const partial = partialState.partial;
	const lastIdx = abortSession ? abortSession.lastHistoryIndex.value : S.lastHistoryIndex;
	if (p.messageIndex === undefined || p.messageIndex === null || p.messageIndex > lastIdx) {
		bumpSessionCount(eventSession, 1);
	}
	cacheSessionHistoryMessage(
		eventSession,
		{
			role: "assistant",
			content: partialState.partialText,
			model: partial?.model || "",
			provider: partial?.provider || "",
			inputTokens: partial?.inputTokens || 0,
			outputTokens: partial?.outputTokens || 0,
			durationMs: partial?.durationMs || 0,
			requestInputTokens: partial?.requestInputTokens,
			requestOutputTokens: partial?.requestOutputTokens,
			reasoning: partial?.reasoning || undefined,
			audio: partial?.audio || undefined,
			run_id: partial?.run_id || p.runId || undefined,
			created_at: partial?.created_at || Date.now(),
		},
		p.messageIndex,
	);
	updateSessionHistoryIndex(eventSession, p.messageIndex);
}

function handleChatAborted(p: ChatPayload, isActive: boolean, isChatPage: boolean, eventSession: string): void {
	clearPendingToolCallEndsForSession(eventSession);
	setSessionReplying(eventSession, false);
	setSessionActiveRunId(eventSession, null);
	const partialState = getAbortedPartialState(p);
	const abortSession = sessionStore.getByKey(eventSession);
	cacheAbortedPartial(eventSession, p, abortSession, partialState);
	if (abortSession) abortSession.resetStreamState();
	if (partialState.hasVisiblePartial && !isActive) {
		setSessionUnread(eventSession, true);
	}
	if (!(isActive && isChatPage)) {
		S.setVoicePending(false);
		return;
	}
	removeThinking();
	clearStaleRunningToolCards();
	renderAbortedPartialInDom(eventSession, p, partialState);
	S.setStreamEl(null);
	S.setStreamText("");
	S.setVoicePending(false);
	moveFirstQueuedToChat();
}

function handleChatNotice(p: ChatPayload, isActive: boolean, isChatPage: boolean): void {
	if (!(isActive && isChatPage)) return;
	// Render titled notices as markdown so emphasis is visible.
	const msg = p.title ? `**${p.title}:** ${p.message}` : p.message || "";
	const noticeEl = p.title ? chatAddMsg("system", renderMarkdown(msg), true) : chatAddMsg("system", msg);
	if (!(noticeEl && p.title)) return;
	noticeEl.classList.add("system-notice");
	if (String(p.title).toLowerCase() !== "sandbox") return;
	noticeEl.classList.add("system-notice-sandbox");
	const normalizedMessage = String(p.message || "").toLowerCase();
	if (normalizedMessage.indexOf("enabled") !== -1) {
		noticeEl.classList.add("is-enabled");
	} else if (normalizedMessage.indexOf("disabled") !== -1) {
		noticeEl.classList.add("is-disabled");
	}
}

function handleChatQueueCleared(_p: ChatPayload, isActive: boolean, isChatPage: boolean): void {
	if (!(isActive && isChatPage)) return;
	const tray = document.getElementById("queuedMessages");
	if (tray) {
		const count = tray.querySelectorAll(".msg").length;
		console.debug("[queued] queue_cleared: removing all from tray", { count });
		while (tray.firstChild) tray.removeChild(tray.firstChild);
		tray.classList.add("hidden");
	}
}

function handleChatSessionCleared(_p: ChatPayload, isActive: boolean, isChatPage: boolean, eventSession: string): void {
	clearPendingToolCallEndsForSession(eventSession);
	setSessionActiveRunId(eventSession, null);
	clearSessionHistoryCache(eventSession);
	// Reset badge, unread state, and history index for every client.
	markSessionLocallyCleared(eventSession);
	if (isActive) {
		S.setLastHistoryIndex(-1);
		S.setChatSeq(0);
	}
	if (!(isActive && isChatPage)) return;
	// Active viewer: clear the chat box and token bar.
	if (S.chatMsgBox) S.chatMsgBox.textContent = "";
	S.setSessionTokens({ input: 0, output: 0 });
	S.setSessionCurrentInputTokens(0);
	updateTokenBar();
}

// ── Handler map and dispatcher ────────────────────────────────

export const chatHandlers: Record<string, ChatHandler> = {
	thinking: handleChatThinking,
	thinking_text: handleChatThinkingText,
	thinking_done: handleChatThinkingDone,
	voice_pending: handleChatVoicePending,
	tool_call_start: handleChatToolCallStart,
	tool_call_end: handleChatToolCallEnd,
	channel_user: handleChatChannelUser,
	user_message: handleChatUserMessage,
	delta: handleChatDelta,
	final: handleChatFinal,
	auto_compact: handleChatAutoCompact,
	compact: handleChatCompact,
	retrying: handleChatRetrying,
	error: handleChatError,
	aborted: handleChatAborted,
	notice: handleChatNotice,
	queue_cleared: handleChatQueueCleared,
	session_cleared: handleChatSessionCleared,
};

export function handleChatEvent(p: ChatPayload): void {
	const eventSession = p.sessionKey || sessionStore.activeSessionKey.value;
	const isActive = eventSession === sessionStore.activeSessionKey.value;
	const isChatPage = currentPrefix === "/chats";

	if (isActive && sessionStore.switchInProgress.value) {
		// If session switching got stuck (e.g. lost RPC response), do not drop
		// terminal frames. Unstick and process final/error so replies still show
		// without requiring a full page reload.
		const allowDuringSwitch =
			p.state === "final" ||
			p.state === "error" ||
			p.state === "aborted" ||
			p.state === "notice" ||
			p.state === "session_cleared" ||
			p.state === "queue_cleared";
		if (!allowDuringSwitch) {
			return;
		}
		if (p.state === "final" || p.state === "error" || p.state === "aborted") {
			sessionStore.switchInProgress.value = false;
			S.setSessionSwitchInProgress(false);
		}
	}

	if (p.sessionKey && !sessionStore.getByKey(p.sessionKey)) {
		fetchSessions();
	}

	const handler = chatHandlers[p.state || ""];
	if (handler) handler(p, isActive, isChatPage, eventSession);
}
