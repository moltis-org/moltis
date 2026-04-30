// ── Message action bar (copy, voice, retry, fork) ────────────
//
// Appended below each finalized assistant message footer.
// The retry button opens a popover with "Try again", "Add details",
// and "More concise" options.
// Icons use CSS mask-image classes (icon-*) backed by SVG files on disk.

import { sendRpc } from "./helpers";
import { renderPersistedAudio } from "./message-voice";
import { showToast } from "./ui";

// ── Icon helper ──────────────────────────────────────────────

function iconSpan(iconClass: string): HTMLSpanElement {
	const span = document.createElement("span");
	span.className = `icon icon-sm ${iconClass}`;
	span.setAttribute("aria-hidden", "true");
	return span;
}

// ── Popover dismiss ──────────────────────────────────────────

let activePopover: HTMLElement | null = null;

function dismissActivePopover(): void {
	if (activePopover) {
		activePopover.remove();
		activePopover = null;
	}
}

function onDocClick(e: MouseEvent): void {
	if (activePopover && !activePopover.contains(e.target as Node)) {
		dismissActivePopover();
	}
}

// ── Core: build the action bar ───────────────────────────────

export interface MessageActionContext {
	messageEl: HTMLElement;
	sessionKey: string;
	messageIndex?: number;
	text?: string;
	runId?: string;
	hasAudio?: boolean;
	audioWarning?: string;
}

export function appendMessageActions(ctx: MessageActionContext): void {
	const { messageEl, sessionKey } = ctx;

	// Surface server-side audio warnings inline on the message.
	if (ctx.audioWarning) {
		let warningEl = messageEl.querySelector(".msg-voice-warning") as HTMLElement | null;
		if (!warningEl) {
			warningEl = document.createElement("div");
			warningEl.className = "voice-error-result msg-voice-warning";
			messageEl.appendChild(warningEl);
		}
		warningEl.textContent = ctx.audioWarning;
	}

	const bar = document.createElement("div");
	bar.className = "msg-action-bar";

	// ── Copy button ──────────────────────────────────────────
	const copyBtn = actionButton("icon-copy", "Copy");
	copyBtn.addEventListener("click", () => {
		const text = extractPlainText(messageEl);
		if (navigator.clipboard?.writeText) {
			navigator.clipboard.writeText(text).then(
				() => {
					copyBtn.replaceChildren(iconSpan("icon-checkmark"));
					copyBtn.title = "Copied";
					setTimeout(() => {
						copyBtn.replaceChildren(iconSpan("icon-copy"));
						copyBtn.title = "Copy";
					}, 1500);
				},
				() => {
					showToast("Failed to copy to clipboard", "error");
				},
			);
		}
	});
	bar.appendChild(copyBtn);

	// ── Retry button (with popover) ──────────────────────────
	const retryBtn = actionButton("icon-retry", "Retry");
	retryBtn.addEventListener("click", (e) => {
		e.stopPropagation();
		if (activePopover && activePopover.parentElement === bar) {
			dismissActivePopover();
			return;
		}
		dismissActivePopover();
		const popover = buildRetryPopover(sessionKey);
		bar.appendChild(popover);
		activePopover = popover;
		requestAnimationFrame(() => {
			document.addEventListener("click", onDocClick, { once: true });
		});
	});
	bar.appendChild(retryBtn);

	// ── Voice button ─────────────────────────────────────────
	if (ctx.text && !ctx.hasAudio) {
		const voiceBtn = actionButton("icon-microphone", "Voice it");
		voiceBtn.addEventListener("click", async () => {
			const params: Record<string, unknown> = { key: sessionKey };
			if (ctx.runId) params.runId = ctx.runId;
			if (Number.isInteger(ctx.messageIndex) && (ctx.messageIndex as number) >= 0) {
				params.messageIndex = ctx.messageIndex;
			}
			if (!(params.runId || Number.isInteger(params.messageIndex))) {
				showToast("Cannot generate voice for this message", "error");
				return;
			}
			voiceBtn.classList.add("msg-action-btn-active");
			voiceBtn.title = "Generating voice...";
			const result = await sendRpc("sessions.voice.generate", params);
			voiceBtn.classList.remove("msg-action-btn-active");
			const payload = result?.payload as Record<string, unknown> | undefined;
			if (result?.ok && payload?.audio) {
				renderPersistedAudio(messageEl, sessionKey, payload.audio as string, true);
				voiceBtn.replaceChildren(iconSpan("icon-checkmark"));
				voiceBtn.title = "Voice generated";
			} else {
				voiceBtn.title = "Voice it";
				const errorMsg = (result?.error as Record<string, unknown> | undefined)?.message as string | undefined;
				showToast(errorMsg || "Voice generation failed", "error");
			}
		});
		bar.appendChild(voiceBtn);
	}

	// ── Fork button ──────────────────────────────────────────
	const forkBtn = actionButton("icon-git-fork", "Fork into new session");
	forkBtn.addEventListener("click", () => {
		sendRpc("sessions.fork", {
			key: sessionKey,
			forkPoint: ctx.messageIndex,
		}).then((res) => {
			if (res.ok) {
				showToast("Forked into new session", "success");
			} else {
				showToast(res.error?.message || "Fork failed", "error");
			}
		});
	});
	bar.appendChild(forkBtn);

	messageEl.appendChild(bar);
}

// ── Button factory ───────────────────────────────────────────

function actionButton(iconClass: string, title: string): HTMLButtonElement {
	const btn = document.createElement("button");
	btn.type = "button";
	btn.className = "msg-action-btn";
	btn.title = title;
	btn.appendChild(iconSpan(iconClass));
	return btn;
}

// ── Retry popover ────────────────────────────────────────────

function buildRetryPopover(sessionKey: string): HTMLElement {
	const pop = document.createElement("div");
	pop.className = "msg-action-popover";

	const items: Array<{ iconClass: string; label: string; action: () => void }> = [
		{
			iconClass: "icon-retry",
			label: "Try again",
			action: () => retrySend(sessionKey, "Please try again with a different response."),
		},
		{
			iconClass: "icon-list-plus",
			label: "Add details",
			action: () => retrySend(sessionKey, "Please provide more details and expand on your answer."),
		},
		{
			iconClass: "icon-list-minus",
			label: "More concise",
			action: () => retrySend(sessionKey, "Please be more concise and brief in your response."),
		},
	];

	for (const item of items) {
		const row = document.createElement("button");
		row.type = "button";
		row.className = "msg-action-popover-item";
		row.appendChild(iconSpan(item.iconClass));
		const span = document.createElement("span");
		span.textContent = item.label;
		row.appendChild(span);
		row.addEventListener("click", (e) => {
			e.stopPropagation();
			dismissActivePopover();
			item.action();
		});
		pop.appendChild(row);
	}

	return pop;
}

// ── Retry action ─────────────────────────────────────────────

function retrySend(sessionKey: string, text: string): void {
	sendRpc("chat.send", { text, _session_key: sessionKey }).then((res) => {
		if (!res.ok) {
			showToast(res.error?.message || "Retry failed", "error");
		}
	});
}

// ── Text extraction ──────────────────────────────────────────

function extractPlainText(messageEl: HTMLElement): string {
	const clone = messageEl.cloneNode(true) as HTMLElement;
	for (const sel of [
		".msg-model-footer",
		".msg-action-bar",
		".msg-reasoning",
		".msg-voice-player-slot",
		".msg-voice-warning",
	]) {
		for (const el of clone.querySelectorAll(sel)) {
			el.remove();
		}
	}
	return (clone.textContent || "").trim();
}
