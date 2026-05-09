// ── Chat send logic ──────────────────────────────────────────

import { chatAddMsg, chatAddMsgWithImages, setComposerStopButton } from "../../chat-ui";
import { highlightCodeBlocks } from "../../code-highlight";
import { renderMarkdown, sendRpc, warmAudioPlayback } from "../../helpers";
import { clearPendingImages, getPendingImages, hasPendingImages } from "../../media-drop";
import { setSessionModel } from "../../models";
import {
	bumpSessionCount,
	cacheOutgoingUserMessage,
	seedSessionPreviewFromUserText,
	setSessionActiveRunId,
	setSessionReplying,
} from "../../sessions";
import * as S from "../../state";
import { modelStore } from "../../stores/model-store";
import type { RpcResponse } from "../../types/rpc";
import { handleSlashCommand, parseSlashCommand, shouldHandleSlashLocally, slashHideMenu } from "./slash-commands";

// ── Types ────────────────────────────────────────────────────

export interface ChatSendParams {
	text?: string;
	content?: ChatContentPart[];
	_seq: number;
	model?: string;
}

export type ChatContentPart = { type: "text"; text: string } | { type: "image_url"; image_url: { url: string } };

export interface ChatSendPayload {
	runId?: string;
	queued?: boolean;
}

// ── Auto-resize ─────────────────────────────────────────────

function chatAutoResize(): void {
	if (!S.chatInput) return;
	S.chatInput.style.height = "auto";
	S.chatInput.style.height = `${Math.min(S.chatInput.scrollHeight, 120)}px`;
}

// ── Slash command integration ───────────────────────────────

export function tryHandleLocalSlashCommand(text: string, hasImages: boolean): boolean {
	if (text.charAt(0) !== "/" || hasImages) return false;
	const slash = parseSlashCommand(text);
	if (!(slash && shouldHandleSlashLocally(slash.name, slash.args))) return false;
	(S.chatInput as HTMLTextAreaElement).value = "";
	chatAutoResize();
	slashHideMenu();
	handleSlashCommand(slash.name, slash.args);
	return true;
}

// ── History navigation ──────────────────────────────────────

export function handleHistoryUp(): void {
	if (S.chatHistory.length === 0) return;
	if (S.chatHistoryIdx === -1) {
		S.setChatHistoryDraft((S.chatInput as HTMLTextAreaElement).value);
		S.setChatHistoryIdx(S.chatHistory.length - 1);
	} else if (S.chatHistoryIdx > 0) S.setChatHistoryIdx(S.chatHistoryIdx - 1);
	(S.chatInput as HTMLTextAreaElement).value = S.chatHistory[S.chatHistoryIdx];
	chatAutoResize();
}

export function handleHistoryDown(): void {
	if (S.chatHistoryIdx === -1) return;
	if (S.chatHistoryIdx < S.chatHistory.length - 1) {
		S.setChatHistoryIdx(S.chatHistoryIdx + 1);
		(S.chatInput as HTMLTextAreaElement).value = S.chatHistory[S.chatHistoryIdx];
	} else {
		S.setChatHistoryIdx(-1);
		(S.chatInput as HTMLTextAreaElement).value = S.chatHistoryDraft;
	}
	chatAutoResize();
}

// ── Send helpers ────────────────────────────────────────────

export function rememberChatHistory(text: string): void {
	if (!text) return;
	S.chatHistory.push(text);
	if (S.chatHistory.length > 200) S.setChatHistory(S.chatHistory.slice(-200));
	localStorage.setItem("moltis-chat-history", JSON.stringify(S.chatHistory));
}

export function resetComposerAfterSend(): void {
	S.setChatHistoryIdx(-1);
	S.setChatHistoryDraft("");
	(S.chatInput as HTMLTextAreaElement).value = "";
	chatAutoResize();
	if (window.innerWidth < 768) S.chatInput?.blur();
}

export function normalizeOutgoingText(text: string, hasImages: boolean): string {
	if (!(S.commandModeEnabled && text && !hasImages)) return text;
	const parsed = parseSlashCommand(text);
	if (parsed && parsed.name === "sh") return text;
	return `/sh ${text}`;
}

export function applySelectedModelToChatParams(chatParams: ChatSendParams): void {
	const effectiveId = modelStore.effectiveModelId.value;
	if (!effectiveId) return;
	chatParams.model = effectiveId;
	setSessionModel(S.activeSessionKey, effectiveId);
}

export function handleChatSendRpcResponse(res: RpcResponse<ChatSendPayload>, userEl: HTMLElement | null): void {
	if (res.ok && res.payload?.runId) setSessionActiveRunId(S.activeSessionKey, res.payload.runId);
	if (res.payload?.queued) {
		markMessageQueued(userEl, S.activeSessionKey);
		return;
	}
	if (!res.ok && res.error) {
		setComposerStopButton(false);
		chatAddMsg("error", res.error.message || "Request failed");
	}
}

export function buildChatMessage(
	text: string,
	seq: number,
	displayText?: string,
): { params: ChatSendParams; el: HTMLElement | null } {
	const userText = displayText !== undefined ? displayText : text;
	const images = hasPendingImages() ? getPendingImages() : [];
	if (images.length > 0) {
		const content: ChatContentPart[] = [];
		if (text) content.push({ type: "text", text });
		for (const img of images)
			content.push({ type: "image_url", image_url: { url: (img as { dataUrl: string }).dataUrl } });
		const params: ChatSendParams = { content, _seq: seq };
		const el = chatAddMsgWithImages("user", userText ? renderMarkdown(userText) : "", images);
		clearPendingImages();
		return { params, el };
	}
	return { params: { text, _seq: seq }, el: chatAddMsg("user", renderMarkdown(userText), true) };
}

function markMessageQueued(el: HTMLElement | null, sessionKey: string): void {
	if (!el) return;
	const tray = document.getElementById("queuedMessages");
	if (!tray) return;
	console.debug("[queued] marking user message as queued, moving to tray", { sessionKey });
	el.classList.add("queued");
	const badge = document.createElement("div");
	badge.className = "queued-badge";
	const label = document.createElement("span");
	label.className = "queued-label";
	label.textContent = "Queued";
	const btn = document.createElement("button");
	btn.className = "queued-cancel";
	btn.title = "Cancel all queued";
	btn.textContent = "\u2715";
	btn.addEventListener("click", (e: MouseEvent) => {
		e.stopPropagation();
		sendRpc("chat.cancel_queued", { sessionKey });
	});
	badge.appendChild(label);
	badge.appendChild(btn);
	el.appendChild(badge);
	tray.appendChild(el);
	tray.classList.remove("hidden");
}

// ── Main sendChat function ──────────────────────────────────
// Exposed so ChatPage and slash-commands can call it.

let maybeRefreshFullContextFn: (() => void) | null = null;

/** Called by ChatPage to register the refresh callback. */
export function setMaybeRefreshFullContextFn(fn: () => void): void {
	maybeRefreshFullContextFn = fn;
}

export function sendChat(): void {
	const text = (S.chatInput as HTMLTextAreaElement).value.trim();
	const hasImages = hasPendingImages();
	if (!((text || hasImages) && S.connected)) return;
	warmAudioPlayback();
	if (tryHandleLocalSlashCommand(text, hasImages)) return;
	rememberChatHistory(text);
	resetComposerAfterSend();
	const outgoingText = normalizeOutgoingText(text, hasImages);
	S.setChatSeq(S.chatSeq + 1);
	const msg = buildChatMessage(outgoingText, S.chatSeq, text);
	const chatParams = msg.params;
	const userEl = msg.el;
	if (userEl) highlightCodeBlocks(userEl);
	applySelectedModelToChatParams(chatParams);
	bumpSessionCount(S.activeSessionKey, 1);
	cacheOutgoingUserMessage(S.activeSessionKey, chatParams);
	seedSessionPreviewFromUserText(S.activeSessionKey, text || outgoingText);
	setSessionReplying(S.activeSessionKey, true);
	setComposerStopButton(true, S.activeSessionKey);
	sendRpc<ChatSendPayload>("chat.send", chatParams)
		.then((res) => handleChatSendRpcResponse(res, userEl))
		.catch(() => {
			setComposerStopButton(false);
			setSessionReplying(S.activeSessionKey, false);
			chatAddMsg("error", "Request failed");
		});
	maybeRefreshFullContextFn?.();
}

export { chatAutoResize };
