// ── Tool call utilities ───────────────────────────────────────

import type { ChannelFooterInfo } from "../chat-ui";
import {
	appendChannelFooter,
	appendReasoningDisclosure,
	chatAddMsg,
	removeThinking,
	smartScrollToBottom,
	stripChannelPrefix,
} from "../chat-ui";
import {
	formatAssistantTokenUsage,
	formatTokenSpeed,
	renderAudioPlayer,
	renderDocument,
	renderMapLinks,
	renderMapPointGroups,
	renderMarkdown,
	renderScreenshot,
	tokenSpeedTone,
	toolCallSummary,
} from "../helpers";
import { attachMessageVoiceControl } from "../message-voice";
import { navigate } from "../router";
import * as S from "../state";
import { sessionStore } from "../stores/session-store";
import type { ChatPayload, ToolCallPayload, ToolResult } from "../types/ws-events";
import { clearChatEmptyState, hasNonWhitespaceContent, isReasoningAlreadyShown, setSafeMarkdownHtml } from "./shared";

// ── Pending tool call end tracking ────────────────────────────

export const pendingToolCallEnds: Map<string, ToolCallPayload> = new Map();

export function toolCallLogicalId(payload: ToolCallPayload | null | undefined): string {
	if (!payload) return "";
	if (payload.runId) return `${payload.runId}:${payload.toolCallId}`;
	return String(payload.toolCallId || "");
}

export function toolCallCardId(payload: ToolCallPayload | ChatPayload | null | undefined): string {
	if ((payload as ToolCallPayload)?.runId) {
		return `tool-${(payload as ToolCallPayload).runId}-${(payload as ToolCallPayload).toolCallId}`;
	}
	return `tool-${(payload as ToolCallPayload)?.toolCallId}`;
}

export function toolCallEventKey(
	eventSession: string,
	payload: ToolCallPayload | ChatPayload | null | undefined,
): string {
	return `${eventSession}:${toolCallLogicalId(payload as ToolCallPayload)}`;
}

export function clearPendingToolCallEndsForSession(sessionKey: string): void {
	const prefix = `${sessionKey}:`;
	for (const key of pendingToolCallEnds.keys()) {
		if (key.startsWith(prefix)) {
			pendingToolCallEnds.delete(key);
		}
	}
}

// ── Tool result rendering ─────────────────────────────────────

export function appendToolResult(toolCard: HTMLElement, result: ToolResult, eventSession: string): void {
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

// ── Tool card completion ──────────────────────────────────────

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

export function completeToolCard(toolCard: HTMLElement, p: ChatPayload, eventSession: string): void {
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

export function clearStaleRunningToolCards(): void {
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

// ── Tool call start (with thinking text extraction) ───────────

/** Extract thinking text from the indicator before it is removed. Returns the
 * trimmed text or null if the indicator has no thinking content. */
function extractThinkingText(): string | null {
	const indicator = document.getElementById("thinkingIndicator");
	if (!indicator) return null;
	const textEl = indicator.querySelector(".thinking-text");
	const text = textEl?.textContent?.trim();
	return text || null;
}

export function handleToolCallStartDom(p: ChatPayload, eventSession: string): void {
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
	smartScrollToBottom();
}

// ── Channel user message rendering ────────────────────────────

export function renderChannelUserMessage(p: ChatPayload, _eventSession: string): void {
	// Compare against the per-session history index, not the global one,
	// to avoid skipping events when viewing a different session.
	const chanSession = sessionStore.getByKey(p.sessionKey || S.activeSessionKey);
	const chanLastIdx = chanSession ? chanSession.lastHistoryIndex.value : S.lastHistoryIndex;
	if (p.messageIndex !== undefined && p.messageIndex <= chanLastIdx) return;

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
				// Safe: renderMarkdown calls esc() first -- all user input is
				// HTML-escaped before formatting tags are applied.
				setSafeMarkdownHtml(textWrap, cleanText);
				el.appendChild(textWrap);
			}
		}
	} else {
		el = chatAddMsg("user", renderMarkdown(cleanText), true);
	}
	if (el && p.channel) {
		appendChannelFooter(el, p.channel as ChannelFooterInfo);
	}
}

// ── Final message resolution ──────────────────────────────────

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

export function resolveFinalMessageEl(p: ChatPayload): HTMLElement | null {
	const finalText = String(p.text || "");
	const hasFinalText = hasNonWhitespaceContent(finalText);
	const isEcho = hasFinalText && isPureToolOutputEcho(finalText, S.lastToolOutput);
	if (!isEcho) {
		if (hasFinalText && S.streamEl) {
			setSafeMarkdownHtml(S.streamEl, finalText);
			return S.streamEl;
		}
		if (hasFinalText) return chatAddMsg("assistant", renderMarkdown(finalText), true);
		// No text (silent reply) -- remove any leftover stream element.
		if (S.streamEl) S.streamEl.remove();
		return null;
	}
	if (S.streamEl) S.streamEl.remove();
	return null;
}

// ── Final footer ──────────────────────────────────────────────

export function appendFinalFooter(msgEl: HTMLElement | null, p: ChatPayload, eventSession: string): void {
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

// ── Aborted partial rendering ─────────────────────────────────

export function renderAbortedPartialInDom(
	eventSession: string,
	p: ChatPayload,
	partialState: {
		partial: ChatPayload["partialMessage"] | null;
		partialText: string;
		partialReasoning: string;
		hasVisiblePartial: boolean;
	},
): void {
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
	smartScrollToBottom();
}
