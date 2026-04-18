// ── WebSocket ─────────────────────────────────────────────────

import {
	appendChannelFooter,
	appendReasoningDisclosure,
	chatAddErrorCard,
	chatAddErrorMsg,
	chatAddMsg,
	removeThinking,
	renderApprovalCard,
	stripChannelPrefix,
	updateTokenBar,
} from "./chat-ui";
import { highlightCodeBlocks } from "./code-highlight";
import { eventListeners } from "./events";
import {
	formatAssistantTokenUsage,
	formatTokenSpeed,
	localizeStructuredError,
	renderAudioPlayer,
	renderDocument,
	renderMapLinks,
	renderMapPointGroups,
	renderMarkdown,
	renderScreenshot,
	sendRpc,
	tokenSpeedTone,
	toolCallSummary,
} from "./helpers";
import { t } from "./i18n";
import { clearLogsAlert, updateLogsAlert } from "./logs-alert";
import { attachMessageVoiceControl } from "./message-voice";
import { fetchModels } from "./models";
import { prefetchChannels } from "./pages/ChannelsPage";
import { maybeRefreshFullContext, renderCompactCard } from "./pages/ChatPage";
import { fetchProjects } from "./projects";
import { currentPage, currentPrefix, mount, navigate } from "./router";
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
} from "./sessions";
import * as S from "./state";
import { sessionStore } from "./stores/session-store";
import type { ConnectOptions } from "./ws-connect";
import { connectWs, forceReconnect, subscribeEvents } from "./ws-connect";

// ── Payload / event interfaces ──────────────────────────────

interface ToolCallPayload {
	runId?: string;
	toolCallId?: string;
	toolName?: string;
	arguments?: Record<string, unknown>;
	executionMode?: string;
	messageIndex?: number;
	sessionKey?: string;
	success?: boolean;
	result?: ToolResult;
	error?: ToolError;
}

interface ToolResult {
	stdout?: string;
	stderr?: string;
	exit_code?: number;
	screenshot?: string;
	screenshot_scale?: number;
	document_ref?: string;
	filename?: string;
	mime_type?: string;
	size_bytes?: number;
	points?: MapPoint[];
	label?: string;
	map_links?: MapLinks;
}

interface ToolError {
	detail?: string;
	message?: string;
	retryAfterMs?: number;
	type?: string;
}

interface MapLinks {
	url?: string;
	google_maps?: string;
	apple_maps?: string;
	openstreetmap?: string;
	[key: string]: unknown;
}

interface MapPoint {
	label?: string;
	latitude?: number;
	longitude?: number;
	map_links?: MapLinks;
}

interface ChatPayload {
	state?: string;
	sessionKey?: string;
	runId?: string;
	text?: string;
	model?: string;
	provider?: string;
	inputTokens?: number;
	outputTokens?: number;
	cacheReadTokens?: number;
	cacheWriteTokens?: number;
	durationMs?: number;
	requestInputTokens?: number;
	requestOutputTokens?: number;
	requestCacheReadTokens?: number;
	requestCacheWriteTokens?: number;
	reasoning?: string;
	audio?: string;
	audioWarning?: string | null;
	replyMedium?: string;
	messageIndex?: number;
	toolCallId?: string;
	toolName?: string;
	arguments?: Record<string, unknown>;
	executionMode?: string;
	success?: boolean;
	result?: ToolResult;
	error?: ChatError;
	message?: string;
	channel?: ChannelInfo;
	title?: string;
	phase?: string;
	mode?: string;
	seq?: number;
	retryAfterMs?: number;
	partialMessage?: PartialMessage;
	canContinue?: boolean;
}

interface ChatError {
	title?: string;
	detail?: string;
	message?: string;
	type?: string;
	retryAfterMs?: number;
	canContinue?: boolean;
}

interface ChannelInfo {
	audio_filename?: string;
	[key: string]: unknown;
}

interface PartialMessage {
	content?: string;
	reasoning?: string;
	model?: string;
	provider?: string;
	inputTokens?: number;
	outputTokens?: number;
	durationMs?: number;
	requestInputTokens?: number;
	requestOutputTokens?: number;
	requestCacheReadTokens?: number;
	requestCacheWriteTokens?: number;
	audio?: string;
	run_id?: string;
	created_at?: number;
}

interface CompactPayload {
	sessionKey?: string;
	phase?: string;
	error?: string;
	mode?: string;
	[key: string]: unknown;
}

interface ApprovalPayload {
	requestId: string;
	command: string;
}

interface LogEntryPayload {
	level?: string;
	[key: string]: unknown;
}

interface SandboxPhasePayload {
	phase?: string;
	error?: string;
	tag?: string;
	built?: boolean;
	count?: number;
	installed?: number;
	skipped?: number;
	image?: string;
}

interface LocalLlmDownloadPayload {
	displayName?: string;
	modelId?: string;
	error?: string;
	complete?: boolean;
	progress?: number;
	downloaded?: number;
	total?: number;
}

interface ModelsUpdatedPayload {
	phase?: string;
	[key: string]: unknown;
}

interface WsErrorPayload {
	message?: string;
}

interface LocationRequestPayload {
	requestId?: string;
	precision?: string;
}

interface AuthCredentialsPayload {
	reason?: string;
}

interface WsFrame {
	type: string;
	event?: string;
	payload?: Record<string, unknown>;
	stream?: unknown;
	done?: unknown;
	channel?: unknown;
}

interface StreamMeta {
	stream: unknown;
	done: unknown;
	channel: unknown;
}

interface AbortedPartialState {
	partial: PartialMessage | null;
	partialText: string;
	partialReasoning: string;
	hasVisiblePartial: boolean;
}

type ChatHandler = (p: ChatPayload, isActive: boolean, isChatPage: boolean, eventSession: string) => void;

// Extend Window for moltis-specific global
declare global {
	interface Window {
		__moltisSuppressNextPasswordChangedRedirect?: boolean;
	}
}

// ── Chat event handlers ──────────────────────────────────────

const pendingToolCallEnds: Map<string, ToolCallPayload> = new Map();
let hasConnectedOnce = false;

function clearChatEmptyState(): void {
	if (!S.chatMsgBox) return;
	const welcome = S.chatMsgBox.querySelector("#welcomeCard");
	if (welcome) welcome.remove();
	const noProviders = S.chatMsgBox.querySelector("#noProvidersCard");
	if (noProviders) noProviders.remove();
	S.chatMsgBox.classList.remove("chat-messages-empty");
}

function toolCallLogicalId(payload: ToolCallPayload | null | undefined): string {
	if (!payload) return "";
	if (payload.runId) return `${payload.runId}:${payload.toolCallId}`;
	return String(payload.toolCallId || "");
}

function toolCallCardId(payload: ToolCallPayload | ChatPayload | null | undefined): string {
	if ((payload as ToolCallPayload)?.runId) {
		return `tool-${(payload as ToolCallPayload).runId}-${(payload as ToolCallPayload).toolCallId}`;
	}
	return `tool-${(payload as ToolCallPayload)?.toolCallId}`;
}

function toolCallEventKey(eventSession: string, payload: ToolCallPayload | ChatPayload | null | undefined): string {
	return `${eventSession}:${toolCallLogicalId(payload as ToolCallPayload)}`;
}

function clearPendingToolCallEndsForSession(sessionKey: string): void {
	const prefix = `${sessionKey}:`;
	for (const key of pendingToolCallEnds.keys()) {
		if (key.startsWith(prefix)) {
			pendingToolCallEnds.delete(key);
		}
	}
}

function makeThinkingDots(): Element {
	const tpl = S.$<HTMLTemplateElement>("tpl-thinking-dots")!;
	return (tpl.content.cloneNode(true) as DocumentFragment).firstElementChild!;
}

function updateSessionRunId(sessionKey: string, runId: string | undefined): void {
	if (!runId) return;
	setSessionActiveRunId(sessionKey, runId);
}

function updateSessionHistoryIndex(sessionKey: string, messageIndex: number | undefined): void {
	const idx = Number(messageIndex);
	if (!Number.isInteger(idx) || idx < 0) return;
	const session = sessionStore.getByKey(sessionKey);
	if (session && idx > session.lastHistoryIndex.value) {
		session.lastHistoryIndex.value = idx;
	}
	if (sessionKey === sessionStore.activeSessionKey.value && idx > S.lastHistoryIndex) {
		S.setLastHistoryIndex(idx);
	}
}

function moveFirstQueuedToChat(): void {
	const tray = document.getElementById("queuedMessages");
	if (!tray) return;
	const firstQueued = tray.querySelector(".msg.user.queued");
	if (!firstQueued) return;
	console.debug("[queued] moving queued message from tray to chat", {
		remaining: tray.querySelectorAll(".msg").length - 1,
	});
	firstQueued.classList.remove("queued");
	const badge = firstQueued.querySelector(".queued-badge");
	if (badge) badge.remove();
	clearChatEmptyState();
	S.chatMsgBox?.appendChild(firstQueued);
	if (!tray.querySelector(".msg")) tray.classList.add("hidden");
}

function makeThinkingStopBtn(sessionKey: string): HTMLButtonElement {
	const btn = document.createElement("button");
	btn.className = "thinking-stop-btn";
	btn.type = "button";
	btn.title = "Stop generation";
	btn.textContent = "Stop";
	btn.addEventListener("click", () => {
		btn.disabled = true;
		btn.textContent = "Stopping\u2026";
		sendRpc("chat.abort", { sessionKey }).catch(() => undefined);
	});
	return btn;
}

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
	if (S.chatMsgBox) S.chatMsgBox.scrollTop = S.chatMsgBox.scrollHeight;
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
		if (S.chatMsgBox) S.chatMsgBox.scrollTop = S.chatMsgBox.scrollHeight;
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
	// Keep the existing thinking dots visible — no separate voice indicator.
}

/** Check whether a reasoning disclosure with the given text already exists in
 * the chat box (from a previous preserveThinkingAsDisclosure call). */
function isReasoningAlreadyShown(text: string | null | undefined): boolean {
	if (!(S.chatMsgBox && text)) return false;
	const normalized = text.trim();
	for (const el of S.chatMsgBox.querySelectorAll(".msg-reasoning-body")) {
		if (el.textContent?.trim() === normalized) return true;
	}
	return false;
}

/** Extract thinking text from the indicator before it is removed. Returns the
 * trimmed text or null if the indicator has no thinking content. */
function extractThinkingText(): string | null {
	const indicator = document.getElementById("thinkingIndicator");
	if (!indicator) return null;
	const textEl = indicator.querySelector(".thinking-text");
	const text = textEl?.textContent?.trim();
	return text || null;
}

function handleChatToolCallStart(p: ChatPayload, isActive: boolean, isChatPage: boolean, eventSession: string): void {
	updateSessionRunId(eventSession, p.runId);
	// Update per-session signal
	const session = sessionStore.getByKey(eventSession);
	if (session) session.streamText.value = "";
	if (!(isActive && isChatPage)) return;
	const thinkingText = extractThinkingText();
	removeThinking();
	// Close the current streaming element so new text deltas after this tool
	// call will create a fresh element positioned after the tool card
	if (S.streamEl) {
		// Remove the element if it's empty (e.g. only whitespace from a
		// pre-tool-call delta) to avoid leaving an orphaned empty div.
		if (!S.streamEl.textContent?.trim()) {
			S.streamEl.remove();
		}
		S.setStreamEl(null);
		S.setStreamText("");
	}
	const cardId = toolCallCardId(p);
	if (document.getElementById(cardId)) return;
	const tpl = S.$<HTMLTemplateElement>("tpl-exec-card")!;
	const frag = tpl.content.cloneNode(true) as DocumentFragment;
	const card = frag.firstElementChild as HTMLElement;
	card.id = cardId;
	const cmd = toolCallSummary(p.toolName, p.arguments, p.executionMode);
	const cmdEl = card.querySelector("[data-cmd]");
	if (cmdEl) cmdEl.textContent = ` ${cmd}`;
	// Preserve thinking text as a reasoning disclosure inside the tool card
	if (thinkingText) appendReasoningDisclosure(card, thinkingText);
	clearChatEmptyState();
	S.chatMsgBox?.appendChild(card);
	const endKey = toolCallEventKey(eventSession, p);
	const pendingEnd = pendingToolCallEnds.get(endKey);
	if (pendingEnd) {
		pendingToolCallEnds.delete(endKey);
		completeToolCard(card, pendingEnd as ChatPayload, eventSession);
	}
	if (S.chatMsgBox) S.chatMsgBox.scrollTop = S.chatMsgBox.scrollHeight;
}

function appendToolResult(toolCard: HTMLElement, result: ToolResult, eventSession: string): void {
	const out = (result.stdout || "").replace(/\n+$/, "");
	// Update per-session signal
	const toolSession = sessionStore.getByKey(eventSession);
	if (toolSession) toolSession.lastToolOutput.value = out;
	// Dual-write to global state for backward compat
	S.setLastToolOutput(out);
	if (out) {
		const outEl = document.createElement("pre");
		outEl.className = "exec-output";
		outEl.textContent = out;
		toolCard.appendChild(outEl);
	}
	const stderrText = (result.stderr || "").replace(/\n+$/, "");
	if (stderrText) {
		const errEl = document.createElement("pre");
		errEl.className = "exec-output exec-stderr";
		errEl.textContent = stderrText;
		toolCard.appendChild(errEl);
	}
	if (result.exit_code !== undefined && result.exit_code !== 0) {
		const codeEl = document.createElement("div");
		codeEl.className = "exec-exit";
		codeEl.textContent = `exit ${result.exit_code}`;
		toolCard.appendChild(codeEl);
	}
	// Browser screenshot support - display as thumbnail with lightbox and download
	if (result.screenshot) {
		const imgSrc = result.screenshot.startsWith("data:")
			? result.screenshot
			: `data:image/png;base64,${result.screenshot}`;
		renderScreenshot(toolCard, imgSrc, result.screenshot_scale || 1);
	}
	// Document card (send_document tool)
	if (result.document_ref) {
		const docStoredName = result.document_ref.split("/").pop() || "";
		const docDisplayName = result.filename || docStoredName;
		const docSessionKey = eventSession || S.activeSessionKey || "main";
		const docMediaSrc = `/api/sessions/${encodeURIComponent(docSessionKey)}/media/${encodeURIComponent(docStoredName)}`;
		renderDocument(toolCard, docMediaSrc, docDisplayName, result.mime_type, result.size_bytes);
	}
	// Map link buttons (show_map tool)
	const renderedPointGroups = renderMapPointGroups(toolCard, result.points, result.label);
	if (!renderedPointGroups && result.map_links) {
		renderMapLinks(toolCard, result.map_links, result.label);
	}
}

function isToolValidationErrorPayload(p: ChatPayload): boolean {
	if (!(p && !p.success && p.error && p.error.detail)) return false;
	const errDetail = p.error.detail.toLowerCase();
	return (
		errDetail.includes("missing field") ||
		errDetail.includes("missing required") ||
		errDetail.includes("missing 'action'") ||
		errDetail.includes("missing 'url'")
	);
}

function completeToolCard(toolCard: HTMLElement, p: ChatPayload, eventSession: string): void {
	// Use muted "retry" style for validation errors, normal styles otherwise.
	if (isToolValidationErrorPayload(p)) {
		toolCard.className = "msg exec-card exec-retry";
	} else {
		toolCard.className = `msg exec-card ${p.success ? "exec-ok" : "exec-err"}`;
	}

	const toolSpin = toolCard.querySelector(".exec-status");
	if (toolSpin) toolSpin.remove();

	if (p.success && p.result) {
		appendToolResult(toolCard, p.result, eventSession);
		return;
	}
	if (!p.success && p.error && p.error.detail) {
		const errMsg = document.createElement("div");
		errMsg.className = isToolValidationErrorPayload(p) ? "exec-retry-detail" : "exec-error-detail";
		errMsg.textContent = p.error.detail;
		toolCard.appendChild(errMsg);
	}
	// Show a hint below the card when a skill is created or updated.
	if (p.success && (p.toolName === "create_skill" || p.toolName === "update_skill")) {
		const hint = document.createElement("div");
		hint.className = "skill-hint";
		const verb = p.toolName === "create_skill" ? "created" : "updated";
		const link = document.createElement("a");
		link.href = "/skills";
		link.textContent = "personal skills";
		link.addEventListener("click", (e: MouseEvent) => {
			e.preventDefault();
			navigate("/skills");
		});
		hint.append(`Skill ${verb} \u2014 available in your `, link);
		toolCard.appendChild(hint);
	}
}

function clearStaleRunningToolCards(): void {
	if (!S.chatMsgBox) return;
	const statusEls = S.chatMsgBox.querySelectorAll(".msg.exec-card .exec-status");
	for (const statusEl of statusEls) {
		const card = statusEl.closest(".msg.exec-card") as HTMLElement | null;
		statusEl.remove();
		if (!card) continue;
		if (!(card.classList.contains("exec-ok") || card.classList.contains("exec-err"))) {
			card.className = "msg exec-card exec-ok";
		}
	}
}

function handleChatToolCallEnd(p: ChatPayload, isActive: boolean, isChatPage: boolean, eventSession: string): void {
	updateSessionRunId(eventSession, p.runId);
	// Always bump badge — the server persists both the hidden assistant
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
			error:
				p.error?.detail || p.error?.message || (typeof p.error === "string" ? (p.error as unknown as string) : null),
			created_at: Date.now(),
		},
		toolHistoryIndex as number | undefined,
	);
	updateSessionHistoryIndex(eventSession, toolHistoryIndex as number | undefined);
	if (!(isActive && isChatPage)) return;
	const toolCard = document.getElementById(toolCallCardId(p));
	if (!toolCard) {
		pendingToolCallEnds.set(toolCallEventKey(eventSession, p), p as unknown as ToolCallPayload);
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
	// Compare against the per-session history index, not the global one,
	// to avoid skipping events when viewing a different session.
	const chanSession = sessionStore.getByKey(p.sessionKey || S.activeSessionKey);
	const chanLastIdx = chanSession ? chanSession.lastHistoryIndex.value : S.lastHistoryIndex;
	if (p.messageIndex !== undefined && p.messageIndex <= chanLastIdx) return;
	updateSessionHistoryIndex(eventSession, p.messageIndex);
	const cleanText = stripChannelPrefix(p.text || "");
	const sessionKey = p.sessionKey || S.activeSessionKey;
	const audioFilename = p.channel?.audio_filename;
	let el: HTMLElement | null;
	if (audioFilename) {
		el = chatAddMsg("user", "", true);
		if (el) {
			const audioSrc = `/api/sessions/${encodeURIComponent(sessionKey)}/media/${encodeURIComponent(audioFilename)}`;
			renderAudioPlayer(el, audioSrc);
			if (cleanText) {
				const textWrap = document.createElement("div");
				textWrap.className = "mt-2";
				// Safe: renderMarkdown calls esc() first — all user input is
				// HTML-escaped before formatting tags are applied.
				textWrap.innerHTML = renderMarkdown(cleanText); // eslint-disable-line no-unsanitized/property
				el.appendChild(textWrap);
			}
		}
	} else {
		el = chatAddMsg("user", renderMarkdown(cleanText), true);
	}
	if (el && p.channel) {
		appendChannelFooter(el, p.channel as unknown as Parameters<typeof appendChannelFooter>[1]);
	}
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
	// Safe: renderMarkdown calls esc() first — all user input is
	// HTML-escaped before formatting tags are applied.
	chatAddMsg("user", renderMarkdown(p.text || ""), true);
}

// Safe: renderMarkdown calls esc() first — all user input is HTML-escaped before
// being passed to innerHTML. This is the standard rendering path for chat messages.
function setSafeMarkdownHtml(el: HTMLElement, text: string): void {
	el.innerHTML = renderMarkdown(text); // eslint-disable-line no-unsanitized/property
}

function hasNonWhitespaceContent(text: string | null | undefined): boolean {
	return String(text || "").trim().length > 0;
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
	if (S.chatMsgBox) S.chatMsgBox.scrollTop = S.chatMsgBox.scrollHeight;
}

function normalizeEchoComparable(text: string | null | undefined): string {
	if (!text) return "";
	return text
		.replace(/```[a-zA-Z0-9_-]*\n?/g, "")
		.replace(/```/g, "")
		.replace(/[`\s]/g, "");
}

function isPureToolOutputEcho(finalText: string, toolOutput: string): boolean {
	const finalComparable = normalizeEchoComparable(finalText);
	const toolComparable = normalizeEchoComparable(toolOutput);
	if (!(finalComparable && toolComparable)) return false;
	return finalComparable === toolComparable;
}

function resolveFinalMessageEl(p: ChatPayload): HTMLElement | null {
	const finalText = String(p.text || "");
	const hasFinalText = hasNonWhitespaceContent(finalText);
	const isEcho = hasFinalText && isPureToolOutputEcho(finalText, S.lastToolOutput);
	if (!isEcho) {
		if (hasFinalText && S.streamEl) {
			setSafeMarkdownHtml(S.streamEl, finalText);
			return S.streamEl;
		}
		if (hasFinalText) return chatAddMsg("assistant", renderMarkdown(finalText), true);
		// No text (silent reply) — remove any leftover stream element.
		if (S.streamEl) S.streamEl.remove();
		return null;
	}
	if (S.streamEl) S.streamEl.remove();
	return null;
}

function appendFinalFooter(msgEl: HTMLElement | null, p: ChatPayload, eventSession: string): void {
	if (!(msgEl && p.model)) return;
	const footer = document.createElement("div");
	footer.className = "msg-model-footer";
	let footerText = p.provider ? `${p.provider} / ${p.model}` : p.model;
	if (p.inputTokens || p.outputTokens) {
		footerText += ` \u00b7 ${formatAssistantTokenUsage(p.inputTokens, p.outputTokens, p.cacheReadTokens)}`;
	}
	const textSpan = document.createElement("span");
	textSpan.textContent = footerText;
	footer.appendChild(textSpan);

	const speedLabel = formatTokenSpeed(p.outputTokens || 0, p.durationMs || 0);
	if (speedLabel) {
		const speed = document.createElement("span");
		speed.className = "msg-token-speed";
		const tone = tokenSpeedTone(p.outputTokens || 0, p.durationMs || 0);
		if (tone) speed.classList.add(`msg-token-speed-${tone}`);
		speed.textContent = ` \u00b7 ${speedLabel}`;
		footer.appendChild(speed);
	}

	if (p.replyMedium === "voice" || p.replyMedium === "text") {
		const badge = document.createElement("span");
		badge.className = "reply-medium-badge";
		badge.textContent = p.replyMedium;
		footer.appendChild(badge);
	}
	msgEl.appendChild(footer);

	void attachMessageVoiceControl({
		messageEl: msgEl,
		footerEl: footer,
		sessionKey: p.sessionKey || eventSession || S.activeSessionKey,
		text: p.text || "",
		runId: p.runId,
		messageIndex: p.messageIndex,
		audioPath: p.audio || undefined,
		audioWarning: p.audioWarning || undefined,
		forceAction: p.replyMedium === "voice" && !p.audio,
		autoplayOnGenerate: true,
	});
}

// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: Final message handling with audio/voice branching
function handleChatFinal(p: ChatPayload, isActive: boolean, isChatPage: boolean, eventSession: string): void {
	clearPendingToolCallEndsForSession(eventSession);
	updateSessionRunId(eventSession, p.runId);
	// Always bump badge — the server persists the final assistant message.
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
			// Safe: renderMarkdown calls esc() first — all user input is HTML-escaped.
			const textWrap = document.createElement("div");
			textWrap.className = "mt-2";
			setSafeMarkdownHtml(textWrap, p.text);
			msgEl.appendChild(textWrap);
		}
		if (p.reasoning && !isReasoningAlreadyShown(p.reasoning)) {
			appendReasoningDisclosure(msgEl, p.reasoning);
		}
		appendFinalFooter(msgEl, p, eventSession);
		if (S.chatMsgBox) S.chatMsgBox.scrollTop = S.chatMsgBox.scrollHeight;
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
			// Silent reply — attach footer to the last visible assistant element
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
		// Always drop the status message — even when the card was
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
	if (S.chatMsgBox) S.chatMsgBox.scrollTop = S.chatMsgBox.scrollHeight;
}

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

function renderAbortedPartialInDom(eventSession: string, p: ChatPayload, partialState: AbortedPartialState): void {
	if (!partialState.hasVisiblePartial) return;
	const partial = partialState.partial;
	let partialEl: HTMLElement | null = null;
	if (hasNonWhitespaceContent(partialState.partialText) && S.streamEl) {
		setSafeMarkdownHtml(S.streamEl, partialState.partialText);
		partialEl = S.streamEl;
	} else if (hasNonWhitespaceContent(partialState.partialText)) {
		partialEl = chatAddMsg("assistant", renderMarkdown(partialState.partialText), true);
	} else if (hasNonWhitespaceContent(partialState.partialReasoning)) {
		partialEl = chatAddMsg("assistant", "", false);
	}
	if (partialEl && partialState.partialReasoning && !isReasoningAlreadyShown(partialState.partialReasoning)) {
		appendReasoningDisclosure(partialEl, partialState.partialReasoning);
	}
	if (!partialEl) return;
	appendFinalFooter(
		partialEl,
		{
			model: partial?.model || "",
			provider: partial?.provider || "",
			inputTokens: partial?.inputTokens || 0,
			outputTokens: partial?.outputTokens || 0,
			durationMs: partial?.durationMs || 0,
			replyMedium: p.replyMedium || "text",
			text: partialState.partialText,
			audio: partial?.audio || undefined,
			audioWarning: undefined,
			runId: p.runId,
			messageIndex: p.messageIndex,
			sessionKey: eventSession,
		},
		eventSession,
	);
	if (S.chatMsgBox) S.chatMsgBox.scrollTop = S.chatMsgBox.scrollHeight;
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

const chatHandlers: Record<string, ChatHandler> = {
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

function handleChatEvent(p: ChatPayload): void {
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

function handleApprovalEvent(payload: ApprovalPayload): void {
	renderApprovalCard(payload.requestId, payload.command);
}

function handleLogEntry(payload: LogEntryPayload): void {
	if (S.logsEventHandler) S.logsEventHandler(payload);
	if (currentPage !== "/logs") {
		const ll = (payload.level || "").toUpperCase();
		if (ll === "ERROR") {
			S.setUnseenErrors(S.unseenErrors + 1);
			updateLogsAlert();
		} else if (ll === "WARN") {
			S.setUnseenWarns(S.unseenWarns + 1);
			updateLogsAlert();
		}
	}
}

function updateSandboxBuildingFlag(building: boolean): void {
	const info = S.sandboxInfo as Record<string, unknown> | null;
	if (info) S.setSandboxInfo({ ...info, image_building: building });
}

let sandboxPrepareIndicatorEl: HTMLElement | null = null;
function handleSandboxPrepare(payload: SandboxPhasePayload): void {
	const isChatPage = currentPrefix === "/chats";
	if (!isChatPage) return;

	if (payload.phase === "start") {
		if (sandboxPrepareIndicatorEl) {
			sandboxPrepareIndicatorEl.remove();
			sandboxPrepareIndicatorEl = null;
		}
		sandboxPrepareIndicatorEl = chatAddMsg(
			"system",
			"Preparing sandbox environment (first run may take a minute)\u2026",
		);
		return;
	}

	if (sandboxPrepareIndicatorEl) {
		sandboxPrepareIndicatorEl.remove();
		sandboxPrepareIndicatorEl = null;
	}

	if (payload.phase === "error") {
		chatAddMsg("error", `Sandbox setup failed: ${payload.error || "unknown"}`);
	}
}

function handleSandboxImageBuild(payload: SandboxPhasePayload): void {
	const phase = payload.phase;
	// Update the sandboxInfo signal so all pages (chat, settings) reflect the build state.
	updateSandboxBuildingFlag(phase === "start");

	const isChatPage = currentPrefix === "/chats";
	if (!isChatPage) return;
	if (phase === "start") {
		chatAddMsg("system", "Building sandbox image (installing packages)\u2026");
	} else if (phase === "done") {
		if (S.chatMsgBox?.lastChild) S.chatMsgBox.removeChild(S.chatMsgBox.lastChild);
		const msg = payload.built ? `Sandbox image ready: ${payload.tag}` : `Sandbox image already cached: ${payload.tag}`;
		chatAddMsg("system", msg);
	} else if (phase === "error") {
		if (S.chatMsgBox?.lastChild) S.chatMsgBox.removeChild(S.chatMsgBox.lastChild);
		chatAddMsg("error", `Sandbox image build failed: ${payload.error || "unknown"}`);
	}
}

function handleSandboxImageProvision(payload: SandboxPhasePayload): void {
	const isChatPage = currentPrefix === "/chats";
	if (!isChatPage) return;
	if (payload.phase === "start") {
		chatAddMsg("system", "Provisioning sandbox packages\u2026");
	} else if (payload.phase === "done") {
		if (S.chatMsgBox?.lastChild) S.chatMsgBox.removeChild(S.chatMsgBox.lastChild);
		chatAddMsg("system", "Sandbox packages provisioned");
	} else if (payload.phase === "error") {
		if (S.chatMsgBox?.lastChild) S.chatMsgBox.removeChild(S.chatMsgBox.lastChild);
		chatAddMsg("error", `Sandbox provisioning failed: ${payload.error || "unknown"}`);
	}
}

// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: Provisioning UI with multiple phases
function handleSandboxHostProvision(payload: SandboxPhasePayload): void {
	const isChatPage = currentPrefix === "/chats";
	if (!isChatPage) return;
	if (payload.phase === "start") {
		const msg = `Installing ${payload.count || ""} package${payload.count === 1 ? "" : "s"} on host\u2026`;
		chatAddMsg("system", msg);
	} else if (payload.phase === "done") {
		if (S.chatMsgBox?.lastChild) S.chatMsgBox.removeChild(S.chatMsgBox.lastChild);
		const parts: string[] = [];
		if ((payload.installed || 0) > 0) parts.push(`${payload.installed} installed`);
		if ((payload.skipped || 0) > 0) parts.push(`${payload.skipped} already present`);
		chatAddMsg("system", `Host packages ready (${parts.join(", ") || "done"})`);
	} else if (payload.phase === "error") {
		if (S.chatMsgBox?.lastChild) S.chatMsgBox.removeChild(S.chatMsgBox.lastChild);
		chatAddMsg("error", `Host package install failed: ${payload.error || "unknown"}`);
	}
}

function handleBrowserImagePull(payload: SandboxPhasePayload): void {
	const isChatPage = currentPrefix === "/chats";
	if (!isChatPage) return;
	const image = payload.image || "browser container";
	if (payload.phase === "start") {
		chatAddMsg("system", `Pulling browser container image (${image})\u2026 This may take a few minutes on first run.`);
	} else if (payload.phase === "done") {
		if (S.chatMsgBox?.lastChild) S.chatMsgBox.removeChild(S.chatMsgBox.lastChild);
		chatAddMsg("system", `Browser container image ready: ${image}`);
	} else if (payload.phase === "error") {
		if (S.chatMsgBox?.lastChild) S.chatMsgBox.removeChild(S.chatMsgBox.lastChild);
		chatAddMsg("error", `Browser container image pull failed: ${payload.error || "unknown"}`);
	}
}

// Track download indicator element
let downloadIndicatorEl: HTMLElement | null = null;

// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: Download progress UI with multiple states
function handleLocalLlmDownload(payload: LocalLlmDownloadPayload): void {
	const isChatPage = currentPrefix === "/chats";
	if (!isChatPage) return;

	const modelName = payload.displayName || payload.modelId || "model";

	if (payload.error) {
		// Download error
		if (downloadIndicatorEl) {
			downloadIndicatorEl.remove();
			downloadIndicatorEl = null;
		}
		chatAddMsg("error", `Failed to download ${modelName}: ${payload.error}`);
		return;
	}

	if (payload.complete) {
		// Download complete
		if (downloadIndicatorEl) {
			downloadIndicatorEl.remove();
			downloadIndicatorEl = null;
		}
		chatAddMsg("system", `${modelName} ready`);
		return;
	}

	// Download in progress - show/update progress indicator
	if (!downloadIndicatorEl) {
		downloadIndicatorEl = document.createElement("div");
		downloadIndicatorEl.className = "msg system download-indicator";

		const status = document.createElement("div");
		status.className = "download-status";
		status.textContent = `Downloading ${modelName}\u2026`;
		downloadIndicatorEl.appendChild(status);

		const progressContainer = document.createElement("div");
		progressContainer.className = "download-progress";
		const progressBar = document.createElement("div");
		progressBar.className = "download-progress-bar";
		progressContainer.appendChild(progressBar);
		downloadIndicatorEl.appendChild(progressContainer);

		const progressText = document.createElement("div");
		progressText.className = "download-progress-text";
		downloadIndicatorEl.appendChild(progressText);

		if (S.chatMsgBox) {
			clearChatEmptyState();
			S.chatMsgBox.appendChild(downloadIndicatorEl);
			S.chatMsgBox.scrollTop = S.chatMsgBox.scrollHeight;
		}
	}

	// Update progress bar
	const barEl = downloadIndicatorEl.querySelector(".download-progress-bar") as HTMLElement | null;
	const textEl = downloadIndicatorEl.querySelector(".download-progress-text") as HTMLElement | null;
	const containerEl = downloadIndicatorEl.querySelector(".download-progress") as HTMLElement | null;

	if (barEl && containerEl) {
		if (payload.progress != null) {
			// Determinate progress - show actual percentage
			containerEl.classList.remove("indeterminate");
			barEl.style.width = `${payload.progress.toFixed(1)}%`;
		} else if (payload.total == null && payload.downloaded != null) {
			// Indeterminate progress - CSS handles the animation
			containerEl.classList.add("indeterminate");
			barEl.style.width = ""; // Let CSS control width
		}
	}

	if (payload.downloaded != null && textEl) {
		const downloadedMb = (payload.downloaded / (1024 * 1024)).toFixed(1);
		if (payload.total != null) {
			const totalMb = (payload.total / (1024 * 1024)).toFixed(1);
			textEl.textContent = `${downloadedMb} / ${totalMb} MB`;
		} else {
			textEl.textContent = `${downloadedMb} MB`;
		}
	}
}

let modelsUpdatedTimer: ReturnType<typeof setTimeout> | null = null;
function handleModelsUpdated(payload: ModelsUpdatedPayload): void {
	// Progress/status frames are consumed directly by the Providers page.
	// Avoid spamming model refresh requests while a probe is running.
	if (payload?.phase === "start" || payload?.phase === "progress") return;
	if (modelsUpdatedTimer) return;
	modelsUpdatedTimer = setTimeout(() => {
		modelsUpdatedTimer = null;
		// fetchModels() delegates to modelStore.fetch() internally
		fetchModels();
		if (S.refreshProvidersPage) S.refreshProvidersPage();
	}, 150);
}

// ── Location request handler ─────────────────────────────────

function handleWsError(payload: WsErrorPayload): void {
	const isChatPage = currentPrefix === "/chats";
	if (!isChatPage) return;
	chatAddErrorMsg(payload.message || "Unknown error");
}

function handleLocationRequest(payload: LocationRequestPayload): void {
	const requestId = payload.requestId;
	if (!requestId) return;

	if (!navigator.geolocation) {
		sendRpc("location.result", {
			requestId,
			error: { code: 0, message: "Geolocation not supported" },
		});
		return;
	}

	// Coarse: city-level, fast, longer cache. Precise: GPS-level, fresh.
	const coarse = payload.precision === "coarse";
	const geoOpts: PositionOptions = coarse
		? { enableHighAccuracy: false, timeout: 10000, maximumAge: 1800000 }
		: { enableHighAccuracy: true, timeout: 15000, maximumAge: 60000 };

	navigator.geolocation.getCurrentPosition(
		(pos: GeolocationPosition) => {
			sendRpc("location.result", {
				requestId,
				location: {
					latitude: pos.coords.latitude,
					longitude: pos.coords.longitude,
					accuracy: pos.coords.accuracy,
				},
			});
		},
		(err: GeolocationPositionError) => {
			sendRpc("location.result", {
				requestId,
				error: { code: err.code, message: err.message },
			});
		},
		geoOpts,
	);
}

function handleNetworkAuditEntry(payload: Record<string, unknown>): void {
	if (S.networkAuditEventHandler) S.networkAuditEventHandler(payload);
}

function handleAuthCredentialsChanged(payload: AuthCredentialsPayload): void {
	if (payload?.reason === "password_changed" && window.__moltisSuppressNextPasswordChangedRedirect === true) {
		window.__moltisSuppressNextPasswordChangedRedirect = false;
		console.info("Deferring redirect for password_changed to show recovery key first");
		return;
	}
	console.warn("Auth credentials changed:", payload.reason);
	window.location.href = "/login";
}

const eventHandlers: Record<string, (payload: Record<string, unknown>, streamMeta?: StreamMeta | null) => void> = {
	chat: handleChatEvent as (payload: Record<string, unknown>) => void,
	error: handleWsError as (payload: Record<string, unknown>) => void,
	"auth.credentials_changed": handleAuthCredentialsChanged as (payload: Record<string, unknown>) => void,
	"exec.approval.requested": handleApprovalEvent as unknown as (payload: Record<string, unknown>) => void,
	"logs.entry": handleLogEntry as (payload: Record<string, unknown>) => void,
	"sandbox.prepare": handleSandboxPrepare as (payload: Record<string, unknown>) => void,
	"sandbox.image.build": handleSandboxImageBuild as (payload: Record<string, unknown>) => void,
	"sandbox.image.provision": handleSandboxImageProvision as (payload: Record<string, unknown>) => void,
	"sandbox.host.provision": handleSandboxHostProvision as (payload: Record<string, unknown>) => void,
	"browser.image.pull": handleBrowserImagePull as (payload: Record<string, unknown>) => void,
	"local-llm.download": handleLocalLlmDownload as (payload: Record<string, unknown>) => void,
	"models.updated": handleModelsUpdated as (payload: Record<string, unknown>) => void,
	"location.request": handleLocationRequest as (payload: Record<string, unknown>) => void,
	"network.audit.entry": handleNetworkAuditEntry as (payload: Record<string, unknown>) => void,
};

function dispatchFrame(frame: WsFrame): void {
	if (frame.type !== "event") return;
	const streamMeta: StreamMeta | null =
		frame.stream != null || frame.done != null
			? { stream: frame.stream, done: frame.done, channel: frame.channel }
			: null;
	const listeners = eventListeners[frame.event || ""] || [];
	listeners.forEach((h) => {
		h(frame.payload || {});
	});
	const handler = eventHandlers[frame.event || ""];
	if (handler) handler(frame.payload || {}, streamMeta);
}

const connectOpts: ConnectOptions = {
	onFrame: dispatchFrame as ConnectOptions["onFrame"],
	onConnected: async (hello) => {
		const isReconnect = hasConnectedOnce;
		hasConnectedOnce = true;
		setStatus("connected", "");
		const now = new Date();
		const ts = now.toLocaleTimeString([], {
			hour: "2-digit",
			minute: "2-digit",
			second: "2-digit",
		});
		chatAddMsg("system", `Connected to moltis gateway v${hello.server.version} at ${ts}`);
		if ((S.sandboxInfo as Record<string, unknown> | null)?.image_building) {
			chatAddMsg("system", "Building sandbox image (installing packages)\u2026");
		}
		// Subscribe to all needed events (v4 protocol).
		// Await so that events are not lost to a race between subscribe and
		// the first broadcast after connect.
		S.setSubscribed(false);
		await subscribeEvents(
			Object.keys(eventHandlers).concat([
				"tick",
				"shutdown",
				"auth.credentials_changed",
				"exec.approval.requested",
				"exec.approval.resolved",
				"device.pair.requested",
				"device.pair.resolved",
				"node.pair.requested",
				"node.pair.resolved",
				"node.invoke.request",
				"session",
				"update.available",
				"hooks.status",
				"push.subscriptions",
				"channel",
				"metrics.update",
				"skills.install.progress",
				"mcp.status",
			]),
		);
		S.setSubscribed(true);
		// Keep initial hydration authoritative via app bootstrap/gon.
		// On reconnect, force a fresh snapshot in case realtime events were missed.
		if (isReconnect) {
			fetchModels();
			fetchSessions();
			fetchProjects();
			prefetchChannels();
		}
		sendRpc("logs.status", {}).then((res) => {
			if (res?.ok) {
				const p = (res.payload || {}) as Record<string, unknown>;
				S.setUnseenErrors((p.unseen_errors as number) || 0);
				S.setUnseenWarns((p.unseen_warns as number) || 0);
				if (currentPage === "/logs") clearLogsAlert();
				else updateLogsAlert();
			}
		});
		if (currentPage === "/chats" || currentPrefix === "/chats") mount(currentPage || "");
	},
	onHandshakeFailed: (frame) => {
		setStatus("", "handshake failed");
		const reason = frame.error?.message || "unknown error";
		chatAddMsg("error", `Handshake failed: ${reason}`);
	},
	onDisconnected: (wasConnected: boolean) => {
		if (wasConnected) {
			setStatus("", "disconnected \u2014 reconnecting\u2026");
		}
		// Reset active session's stream state
		const activeS = sessionStore.activeSession.value;
		if (activeS) activeS.resetStreamState();
		S.setStreamEl(null);
		S.setStreamText("");
	},
};

export function connect(): void {
	setStatus("connecting", "connecting...");
	connectWs(connectOpts);
}

function setStatus(state: string, text: string): void {
	const dot = S.$("statusDot");
	const sText = S.$("statusText");
	if (dot) dot.className = `status-dot ${state}`;
	if (sText) {
		sText.textContent = text;
		sText.classList.toggle("status-text-live", state === "connected");
	}
	const sendBtn = S.$<HTMLButtonElement>("sendBtn");
	if (sendBtn) sendBtn.disabled = state !== "connected";
}

document.addEventListener("visibilitychange", () => {
	if (!(document.hidden || S.connected)) {
		forceReconnect(connectOpts);
	}
});
