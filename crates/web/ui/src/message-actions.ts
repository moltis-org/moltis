// ── Message action bar (copy, voice, retry, fork) ────────────
//
// Appended below each finalized assistant message footer.
// The retry button opens a popover with "Try again", "Add details",
// and "More concise" options.

import { sendRpc } from "./helpers";
import { showToast } from "./ui";

// ── SVG icon helpers ─────────────────────────────────────────

const NS = "http://www.w3.org/2000/svg";

function svgIcon(viewBox: string, ...paths: string[]): SVGSVGElement {
	const svg = document.createElementNS(NS, "svg");
	svg.setAttribute("viewBox", viewBox);
	svg.setAttribute("fill", "none");
	svg.setAttribute("stroke", "currentColor");
	svg.setAttribute("stroke-width", "2");
	svg.setAttribute("stroke-linecap", "round");
	svg.setAttribute("stroke-linejoin", "round");
	svg.setAttribute("aria-hidden", "true");
	svg.classList.add("msg-action-icon");
	for (const d of paths) {
		const p = document.createElementNS(NS, "path");
		p.setAttribute("d", d);
		svg.appendChild(p);
	}
	return svg;
}

function copyIcon(): SVGSVGElement {
	// Two-rect clipboard icon (Lucide "copy")
	const svg = document.createElementNS(NS, "svg");
	svg.setAttribute("viewBox", "0 0 24 24");
	svg.setAttribute("fill", "none");
	svg.setAttribute("stroke", "currentColor");
	svg.setAttribute("stroke-width", "2");
	svg.setAttribute("stroke-linecap", "round");
	svg.setAttribute("stroke-linejoin", "round");
	svg.setAttribute("aria-hidden", "true");
	svg.classList.add("msg-action-icon");
	const r1 = document.createElementNS(NS, "rect");
	r1.setAttribute("x", "9");
	r1.setAttribute("y", "9");
	r1.setAttribute("width", "13");
	r1.setAttribute("height", "13");
	r1.setAttribute("rx", "2");
	svg.appendChild(r1);
	const p = document.createElementNS(NS, "path");
	p.setAttribute("d", "M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1");
	svg.appendChild(p);
	return svg;
}

function checkIcon(): SVGSVGElement {
	return svgIcon("0 0 24 24", "M20 6 9 17l-5-5");
}

function retryIcon(): SVGSVGElement {
	return svgIcon("0 0 24 24", "M21 12a9 9 0 1 1-9-9c2.52 0 4.93 1 6.74 2.74L21 8", "M21 3v5h-5");
}

function forkIcon(): SVGSVGElement {
	// Git-branch style icon
	const svg = document.createElementNS(NS, "svg");
	svg.setAttribute("viewBox", "0 0 24 24");
	svg.setAttribute("fill", "none");
	svg.setAttribute("stroke", "currentColor");
	svg.setAttribute("stroke-width", "2");
	svg.setAttribute("stroke-linecap", "round");
	svg.setAttribute("stroke-linejoin", "round");
	svg.setAttribute("aria-hidden", "true");
	svg.classList.add("msg-action-icon");
	const l = document.createElementNS(NS, "line");
	l.setAttribute("x1", "6");
	l.setAttribute("y1", "3");
	l.setAttribute("x2", "6");
	l.setAttribute("y2", "15");
	svg.appendChild(l);
	const c1 = document.createElementNS(NS, "circle");
	c1.setAttribute("cx", "18");
	c1.setAttribute("cy", "6");
	c1.setAttribute("r", "3");
	svg.appendChild(c1);
	const c2 = document.createElementNS(NS, "circle");
	c2.setAttribute("cx", "6");
	c2.setAttribute("cy", "18");
	c2.setAttribute("r", "3");
	svg.appendChild(c2);
	const p = document.createElementNS(NS, "path");
	p.setAttribute("d", "M18 9a9 9 0 0 1-9 9");
	svg.appendChild(p);
	return svg;
}

// ── Menu item icons (for retry popover) ──────────────────────

function addDetailsIcon(): SVGSVGElement {
	// List-plus style icon
	return svgIcon("0 0 24 24", "M11 12H3", "M16 6H3", "M16 18H3", "M18 9v6", "M21 12h-6");
}

function conciseIcon(): SVGSVGElement {
	// List-minus style icon
	return svgIcon("0 0 24 24", "M11 12H3", "M16 6H3", "M16 18H3", "M21 12h-6");
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
}

export function appendMessageActions(ctx: MessageActionContext): void {
	const { messageEl, sessionKey } = ctx;

	const bar = document.createElement("div");
	bar.className = "msg-action-bar";

	// ── Copy button ──────────────────────────────────────────
	const copyBtn = actionButton(copyIcon(), "Copy");
	copyBtn.addEventListener("click", () => {
		const text = extractPlainText(messageEl);
		if (navigator.clipboard?.writeText) {
			navigator.clipboard.writeText(text).then(() => {
				// Swap icon to checkmark briefly
				copyBtn.replaceChildren(checkIcon());
				copyBtn.title = "Copied";
				setTimeout(() => {
					copyBtn.replaceChildren(copyIcon());
					copyBtn.title = "Copy";
				}, 1500);
			});
		}
	});
	bar.appendChild(copyBtn);

	// ── Retry button (with popover) ──────────────────────────
	const retryBtn = actionButton(retryIcon(), "Retry");
	retryBtn.addEventListener("click", (e) => {
		e.stopPropagation();
		if (activePopover && activePopover.parentElement === bar) {
			dismissActivePopover();
			return;
		}
		dismissActivePopover();
		const popover = buildRetryPopover(sessionKey, messageEl);
		bar.appendChild(popover);
		activePopover = popover;
		// Dismiss on next outside click
		requestAnimationFrame(() => {
			document.addEventListener("click", onDocClick, { once: true });
		});
	});
	bar.appendChild(retryBtn);

	// ── Fork button ──────────────────────────────────────────
	const forkBtn = actionButton(forkIcon(), "Fork into new session");
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

function actionButton(icon: SVGSVGElement, title: string): HTMLButtonElement {
	const btn = document.createElement("button");
	btn.type = "button";
	btn.className = "msg-action-btn";
	btn.title = title;
	btn.appendChild(icon);
	return btn;
}

// ── Retry popover ────────────────────────────────────────────

function buildRetryPopover(sessionKey: string, messageEl: HTMLElement): HTMLElement {
	const pop = document.createElement("div");
	pop.className = "msg-action-popover";

	const items: Array<{ icon: SVGSVGElement; label: string; action: () => void }> = [
		{
			icon: retryIcon(),
			label: "Try again",
			action: () => retryMessage(sessionKey, messageEl),
		},
		{
			icon: addDetailsIcon(),
			label: "Add details",
			action: () =>
				retryWithInstruction(sessionKey, messageEl, "Please provide more details and expand on your answer."),
		},
		{
			icon: conciseIcon(),
			label: "More concise",
			action: () => retryWithInstruction(sessionKey, messageEl, "Please be more concise and brief in your response."),
		},
	];

	for (const item of items) {
		const row = document.createElement("button");
		row.type = "button";
		row.className = "msg-action-popover-item";
		row.appendChild(item.icon);
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

// ── Retry actions ────────────────────────────────────────────
// Uses chat.send with follow-up instructions since there is no
// dedicated retry RPC. The agent regenerates based on the prompt.

function retryMessage(_sessionKey: string, _messageEl: HTMLElement): void {
	sendRpc("chat.send", { text: "Please try again with a different response.", _seq: Date.now() }).then((res) => {
		if (!res.ok) {
			showToast(res.error?.message || "Retry failed", "error");
		}
	});
}

function retryWithInstruction(_sessionKey: string, _messageEl: HTMLElement, instruction: string): void {
	sendRpc("chat.send", { text: instruction, _seq: Date.now() }).then((res) => {
		if (!res.ok) {
			showToast(res.error?.message || "Retry failed", "error");
		}
	});
}

// ── Text extraction ──────────────────────────────────────────

function extractPlainText(messageEl: HTMLElement): string {
	// Clone the element, remove footer/action bar/reasoning, get textContent
	const clone = messageEl.cloneNode(true) as HTMLElement;
	for (const sel of [
		".msg-model-footer",
		".msg-action-bar",
		".msg-reasoning",
		".msg-voice-player-slot",
		".msg-voice-warning",
	]) {
		const el = clone.querySelector(sel);
		if (el) el.remove();
	}
	return (clone.textContent || "").trim();
}
