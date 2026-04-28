// ── Chat UI ─────────────────────────────────────────────────

import { formatTokens, parseErrorMessage, sendRpc, updateCountdown } from "./helpers";
import * as S from "./state";

interface ErrorCardData {
	icon?: string;
	title: string;
	detail?: string;
	provider?: string;
	resetsAt?: number | null;
}

interface ImageAttachment {
	dataUrl: string;
	name: string;
}

function clearChatEmptyState(): void {
	if (!S.chatMsgBox) return;
	const welcome = S.chatMsgBox.querySelector("#welcomeCard");
	if (welcome) welcome.remove();
	const noProviders = S.chatMsgBox.querySelector("#noProvidersCard");
	if (noProviders) noProviders.remove();
	S.chatMsgBox.classList.remove("chat-messages-empty");
}

// Scroll chat to bottom and keep it pinned until layout settles.
// Uses a ResizeObserver to catch any late layout shifts (sidebar re-render,
// font loading, async style recalc) and re-scrolls until stable.
export function scrollChatToBottom(): void {
	if (!S.chatMsgBox) return;
	S.chatMsgBox.scrollTop = S.chatMsgBox.scrollHeight;
	const box = S.chatMsgBox;
	const observer = new ResizeObserver(() => {
		box.scrollTop = box.scrollHeight;
	});
	observer.observe(box);
	setTimeout(() => {
		observer.disconnect();
	}, 500);
	hideNewContentIndicator();
}

/** Returns true when the chat scroll position is within `threshold` px of the bottom. */
export function isChatAtBottom(threshold = 60): boolean {
	if (!S.chatMsgBox) return true;
	const { scrollTop, scrollHeight, clientHeight } = S.chatMsgBox;
	return scrollHeight - scrollTop - clientHeight < threshold;
}

/** Scroll to bottom only if the user is already near the bottom (smart auto-scroll). */
export function smartScrollToBottom(): void {
	if (S.autoScrollMode === "always") {
		scrollChatToBottom();
		return;
	}
	if (isChatAtBottom()) {
		scrollChatToBottom();
	} else {
		showNewContentIndicator();
	}
}

/** Show the "new content" floating indicator on the chat area. */
export function showNewContentIndicator(): void {
	if (!S.chatMsgBox) return;
	let indicator = S.chatMsgBox.querySelector(".new-content-indicator") as HTMLButtonElement | null;
	if (!indicator) {
		indicator = document.createElement("button") as HTMLButtonElement;
		indicator.className = "new-content-indicator";
		indicator.type = "button";
		indicator.textContent = "↓ New messages";
		indicator.addEventListener("click", () => {
			scrollChatToBottom();
		});
		S.chatMsgBox.appendChild(indicator);
	}
}

/** Hide the "new content" floating indicator. */
export function hideNewContentIndicator(): void {
	if (!S.chatMsgBox) return;
	const indicator = S.chatMsgBox.querySelector(".new-content-indicator");
	if (indicator) indicator.remove();
}

export function chatAddMsg(cls: string, content: string, isHtml?: boolean): HTMLDivElement | null {
	if (!S.chatMsgBox) return null;
	clearChatEmptyState();
	const el = document.createElement("div");
	el.className = `msg ${cls}`;
	if (cls === "system") {
		el.classList.add("system-notice");
	}
	if (isHtml) {
		// Safe: content is produced by renderMarkdown which escapes via esc() first,
		// then only adds our own formatting tags (pre, code, strong).
		el.innerHTML = content; // eslint-disable-line no-unsanitized/property
	} else {
		el.textContent = content;
	}
	S.chatMsgBox.appendChild(el);
	if (!S.chatBatchLoading) {
		if (cls === "user") scrollChatToBottom();
		else smartScrollToBottom();
	}
	return el;
}

/**
 * Add a user message with image thumbnails below the text.
 */
export function chatAddMsgWithImages(
	cls: string,
	htmlContent: string,
	images: ImageAttachment[],
): HTMLDivElement | null {
	if (!S.chatMsgBox) return null;
	clearChatEmptyState();
	const el = document.createElement("div");
	el.className = `msg ${cls}`;
	if (htmlContent) {
		const textDiv = document.createElement("div");
		// Safe: htmlContent is produced by renderMarkdown which escapes user
		// input via esc() first, then only adds our own formatting tags.
		// This is the same pattern used in chatAddMsg above.
		textDiv.innerHTML = htmlContent; // eslint-disable-line no-unsanitized/property
		el.appendChild(textDiv);
	}
	if (images && images.length > 0) {
		const thumbRow = document.createElement("div");
		thumbRow.className = "msg-image-row";
		for (const img of images) {
			const thumb = document.createElement("img");
			thumb.className = "msg-image-thumb";
			thumb.src = img.dataUrl;
			thumb.alt = img.name;
			thumbRow.appendChild(thumb);
		}
		el.appendChild(thumbRow);
	}
	S.chatMsgBox.appendChild(el);
	if (!S.chatBatchLoading) {
		if (cls === "user") scrollChatToBottom();
		else smartScrollToBottom();
	}
	return el;
}

export function stripChannelPrefix(text: string): string {
	return text.replace(/^\[Telegram(?:\s+from\s+[^\]]+)?\]\s*/, "");
}

export interface ChannelFooterInfo {
	channel_type?: string;
	username?: string;
	sender_name?: string;
	message_kind?: string;
}

export function appendChannelFooter(el: HTMLElement, channel: ChannelFooterInfo): void {
	const ft = document.createElement("div");
	ft.className = "msg-channel-footer";
	let label = channel.channel_type || "channel";
	const who = channel.username ? `@${channel.username}` : channel.sender_name;
	if (who) label += ` \u00b7 ${who}`;
	if (channel.message_kind === "voice") {
		const icon = document.createElement("span");
		icon.className = "voice-icon";
		icon.setAttribute("aria-hidden", "true");
		ft.appendChild(icon);
	}

	const text = document.createElement("span");
	text.textContent = `via ${label}`;
	ft.appendChild(text);
	el.appendChild(ft);
}

export function removeThinking(): void {
	const el = document.getElementById("thinkingIndicator");
	if (el) el.remove();
}

export function appendReasoningDisclosure(
	messageEl: HTMLElement | null,
	reasoningText: string | null | undefined,
): HTMLDetailsElement | null {
	if (!messageEl) return null;
	const normalized = String(reasoningText || "").trim();
	if (!normalized) return null;
	const existing = messageEl.querySelector(".msg-reasoning");
	if (existing) existing.remove();
	const details = document.createElement("details");
	details.className = "msg-reasoning";
	const summary = document.createElement("summary");
	summary.className = "msg-reasoning-summary";
	summary.textContent = "Reasoning";
	details.appendChild(summary);
	const body = document.createElement("div");
	body.className = "msg-reasoning-body";
	body.textContent = normalized;
	details.appendChild(body);
	messageEl.appendChild(details);
	return details;
}

export function chatAddErrorCard(err: ErrorCardData): void {
	if (!S.chatMsgBox) return;
	clearChatEmptyState();
	const el = document.createElement("div");
	el.className = "msg error-card";

	const icon = document.createElement("div");
	icon.className = "error-icon";
	icon.textContent = err.icon || "\u26A0\uFE0F";
	el.appendChild(icon);

	const body = document.createElement("div");
	body.className = "error-body";

	const title = document.createElement("div");
	title.className = "error-title";
	title.textContent = err.title;
	body.appendChild(title);

	if (err.detail) {
		const detail = document.createElement("div");
		detail.className = "error-detail";
		detail.textContent = err.detail;
		body.appendChild(detail);
	}

	if (err.provider) {
		const prov = document.createElement("div");
		prov.className = "error-detail";
		prov.textContent = `Provider: ${err.provider}`;
		prov.style.marginTop = "4px";
		prov.style.opacity = "0.6";
		body.appendChild(prov);
	}

	if (err.resetsAt) {
		const countdown = document.createElement("div");
		countdown.className = "error-countdown";
		el.appendChild(body);
		el.appendChild(countdown);
		updateCountdown(countdown, err.resetsAt);
		const timer = setInterval(() => {
			if (updateCountdown(countdown, err.resetsAt!)) clearInterval(timer);
		}, 1000);
	} else {
		el.appendChild(body);
	}

	S.chatMsgBox.appendChild(el);
	smartScrollToBottom();
}

export function chatAddErrorMsg(message: string): void {
	chatAddErrorCard(parseErrorMessage(message));
}

export function renderApprovalCard(requestId: string, command: string): void {
	if (!S.chatMsgBox) return;
	clearChatEmptyState();
	const tpl = S.$<HTMLTemplateElement>("tpl-approval-card")!;
	const frag = tpl.content.cloneNode(true) as DocumentFragment;
	const card = frag.firstElementChild as HTMLElement;
	card.id = `approval-${requestId}`;

	(card.querySelector(".approval-cmd") as HTMLElement).textContent = command;

	const allowBtn = card.querySelector(".approval-allow") as HTMLButtonElement;
	const denyBtn = card.querySelector(".approval-deny") as HTMLButtonElement;
	allowBtn.onclick = () => {
		resolveApproval(requestId, "approved", command, card);
	};
	denyBtn.onclick = () => {
		resolveApproval(requestId, "denied", null, card);
	};

	const countdown = card.querySelector(".approval-countdown") as HTMLElement;
	let remaining = 120;
	const timer = setInterval(() => {
		remaining--;
		countdown.textContent = `${remaining}s`;
		if (remaining <= 0) {
			clearInterval(timer);
			card.classList.add("approval-expired");
			allowBtn.disabled = true;
			denyBtn.disabled = true;
			countdown.textContent = "expired";
		}
	}, 1000);
	countdown.textContent = `${remaining}s`;

	S.chatMsgBox.appendChild(card);
	smartScrollToBottom();
}

export function resolveApproval(requestId: string, decision: string, command: string | null, card: HTMLElement): void {
	const params: Record<string, string> = { requestId, decision };
	if (command) params.command = command;
	sendRpc("exec.approval.resolve", params).then(() => {
		card.classList.add("approval-resolved");
		card.querySelectorAll<HTMLButtonElement>(".approval-btn").forEach((b) => {
			b.disabled = true;
		});
		const status = document.createElement("div");
		status.className = "approval-status";
		status.textContent = decision === "approved" ? "Allowed" : "Denied";
		card.appendChild(status);
	});
}

export function highlightAndScroll(msgEls: (HTMLElement | null)[], messageIndex: number, query: string): void {
	let target: HTMLElement | null = null;
	if (messageIndex >= 0 && messageIndex < msgEls.length && msgEls[messageIndex]) {
		target = msgEls[messageIndex];
	}
	const lowerQ = query.toLowerCase();
	if (!target || (target.textContent || "").toLowerCase().indexOf(lowerQ) === -1) {
		for (const candidate of msgEls) {
			if (candidate && (candidate.textContent || "").toLowerCase().indexOf(lowerQ) !== -1) {
				target = candidate;
				break;
			}
		}
	}
	if (!target) return;
	msgEls.forEach((el) => {
		if (el) highlightTermInElement(el, query);
	});
	target.scrollIntoView({ behavior: "smooth", block: "center" });
	target.classList.add("search-highlight-msg");
	setTimeout(() => {
		if (!S.chatMsgBox) return;
		S.chatMsgBox.querySelectorAll("mark.search-term-highlight").forEach((m) => {
			const parent = m.parentNode!;
			parent.replaceChild(document.createTextNode(m.textContent || ""), m);
			parent.normalize();
		});
		S.chatMsgBox.querySelectorAll(".search-highlight-msg").forEach((el) => {
			el.classList.remove("search-highlight-msg");
		});
	}, 5000);
}

export function highlightTermInElement(el: HTMLElement, query: string): void {
	const walker = document.createTreeWalker(el, NodeFilter.SHOW_TEXT, null);
	const nodes: Text[] = [];
	while (walker.nextNode()) nodes.push(walker.currentNode as Text);
	const lowerQ = query.toLowerCase();
	nodes.forEach((textNode) => {
		const text = textNode.nodeValue || "";
		const lowerText = text.toLowerCase();
		let idx = lowerText.indexOf(lowerQ);
		if (idx === -1) return;
		const frag = document.createDocumentFragment();
		let pos = 0;
		while (idx !== -1) {
			if (idx > pos) frag.appendChild(document.createTextNode(text.substring(pos, idx)));
			const mark = document.createElement("mark");
			mark.className = "search-term-highlight";
			mark.textContent = text.substring(idx, idx + query.length);
			frag.appendChild(mark);
			pos = idx + query.length;
			idx = lowerText.indexOf(lowerQ, pos);
		}
		if (pos < text.length) frag.appendChild(document.createTextNode(text.substring(pos)));
		textNode.parentNode?.replaceChild(frag, textNode);
	});
}

export function chatAutoResize(): void {
	if (!S.chatInput) return;
	S.chatInput.style.height = "auto";
	S.chatInput.style.height = `${Math.min(S.chatInput.scrollHeight, 120)}px`;
}

export function updateCommandInputUI(): void {
	if (!S.chatInput) return;
	const row = S.chatInput.closest(".chat-input-row");
	if (row) {
		row.classList.toggle("command-mode", S.commandModeEnabled);
	}
	const prompt = S.$("chatCommandPrompt");
	if (prompt) {
		prompt.textContent = S.sessionExecPromptSymbol || "$";
		prompt.classList.toggle("chat-command-prompt-hidden", !S.commandModeEnabled);
		prompt.setAttribute("aria-hidden", S.commandModeEnabled ? "false" : "true");
	}
	if (S.commandModeEnabled) {
		(S.chatInput as HTMLTextAreaElement).placeholder = "Run shell command\u2026";
		S.chatInput.setAttribute("aria-label", "Command input");
	} else {
		(S.chatInput as HTMLTextAreaElement).placeholder = "Type a message...";
		S.chatInput.setAttribute("aria-label", "Chat input");
	}
	updateTokenBar();
}

export function updateTokenBar(): void {
	const bar = S.$("tokenBar");
	if (!bar) return;
	const total = S.sessionTokens.input + S.sessionTokens.output;
	let text =
		formatTokens(S.sessionTokens.input) +
		" in / " +
		formatTokens(S.sessionTokens.output) +
		" out \u00b7 " +
		formatTokens(total) +
		" tokens";
	if (S.sessionContextWindow > 0) {
		const currentInput = S.sessionCurrentInputTokens || 0;
		const pct = Math.max(0, 100 - Math.round((currentInput / S.sessionContextWindow) * 100));
		text += ` \u00b7 Context left before auto-compact: ${pct}%`;
	}
	if (!S.sessionToolsEnabled) {
		text += " \u00b7 Tools: disabled";
	}
	const execModeLabel = S.sessionExecMode === "sandbox" ? "sandboxed" : "host";
	const promptSymbol = S.sessionExecPromptSymbol || "$";
	text += ` \u00b7 Execute: ${execModeLabel} (${promptSymbol})`;
	if (S.commandModeEnabled) {
		text += " \u00b7 /sh mode";
	}
	bar.textContent = text;
}
