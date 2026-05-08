// ── Shared helpers used across ws/ sub-modules ───────────────

import { renderMarkdown, sendRpc } from "../helpers";
import { setSessionActiveRunId } from "../sessions";
import * as S from "../state";
import { sessionStore } from "../stores/session-store";

// ── Chat empty-state management ───────────────────────────────

export function clearChatEmptyState(): void {
	if (!S.chatMsgBox) return;
	const welcome = S.chatMsgBox.querySelector("#welcomeCard");
	if (welcome) welcome.remove();
	const noProviders = S.chatMsgBox.querySelector("#noProvidersCard");
	if (noProviders) noProviders.remove();
	S.chatMsgBox.classList.remove("chat-messages-empty");
}

// ── Thinking UI helpers ───────────────────────────────────────

export function makeThinkingDots(): Element {
	const tpl = S.$<HTMLTemplateElement>("tpl-thinking-dots")!;
	return (tpl.content.cloneNode(true) as DocumentFragment).firstElementChild!;
}

export function makeThinkingStopBtn(sessionKey: string): HTMLButtonElement {
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

// ── Session helpers ───────────────────────────────────────────

export function updateSessionRunId(sessionKey: string, runId: string | undefined): void {
	if (!runId) return;
	setSessionActiveRunId(sessionKey, runId);
}

export function updateSessionHistoryIndex(sessionKey: string, messageIndex: number | undefined): void {
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

export function moveFirstQueuedToChat(): void {
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

// ── Markdown rendering ────────────────────────────────────────

/**
 * Safe wrapper: renderMarkdown uses the `marked` library which HTML-escapes
 * all input by default. No raw user content reaches innerHTML.
 */
export function setSafeMarkdownHtml(el: HTMLElement, text: string): void {
	const rendered = renderMarkdown(text);
	el.textContent = "";
	const wrapper = document.createElement("span");
	wrapper.insertAdjacentHTML("afterbegin", rendered);
	while (wrapper.firstChild) el.appendChild(wrapper.firstChild);
}

export function hasNonWhitespaceContent(text: string | null | undefined): boolean {
	return String(text || "").trim().length > 0;
}

// ── Reasoning dedup ───────────────────────────────────────────

/** Check whether a reasoning disclosure with the given text already exists in
 * the chat box (from a previous preserveThinkingAsDisclosure call). */
export function isReasoningAlreadyShown(text: string | null | undefined): boolean {
	if (!(S.chatMsgBox && text)) return false;
	const normalized = text.trim();
	for (const el of S.chatMsgBox.querySelectorAll(".msg-reasoning-body")) {
		if (el.textContent?.trim() === normalized) return true;
	}
	return false;
}
