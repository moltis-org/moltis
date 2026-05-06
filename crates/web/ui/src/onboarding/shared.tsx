// ── Shared helpers for onboarding sub-modules ────────────────

import type { VNode } from "preact";
import { eventListeners } from "../events";
import { t } from "../i18n";
import { connectWs, subscribeEvents } from "../ws-connect";

// ── WebSocket bootstrap ─────────────────────────────────────

let wsStarted = false;
export function ensureWsConnected(): void {
	if (wsStarted) return;
	wsStarted = true;
	connectWs({
		backoff: { factor: 2, max: 10000 },
		onConnected: () => {
			subscribeEvents(["channel"]);
		},
		onFrame: (frame: { type: string; event?: string; payload?: Record<string, unknown> }) => {
			if (frame.type !== "event") return;
			const listeners = eventListeners[frame.event || ""] || [];
			listeners.forEach((h) => {
				h(frame.payload || {});
			});
		},
	});
}

// ── Shared components ───────────────────────────────────────

export function ErrorPanel({ message }: { message: string }): VNode {
	return (
		<div role="alert" className="alert-error-text whitespace-pre-line">
			<span className="text-[var(--error)] font-medium">{t("onboarding:errorPrefix")}</span> {message}
		</div>
	);
}

// ── Utility helpers ─────────────────────────────────────────

export function preferredChatPath(): string {
	const key = localStorage.getItem("moltis-session") || "main";
	return `/chats/${key.replace(/:/g, "/")}`;
}

export function detectBrowserTimezone(): string {
	try {
		const timezone = Intl.DateTimeFormat().resolvedOptions().timeZone;
		return typeof timezone === "string" ? timezone.trim() : "";
	} catch {
		return "";
	}
}

export function bufferToBase64(buf: ArrayBuffer): string {
	const bytes = new Uint8Array(buf);
	let str = "";
	for (const b of bytes) str += String.fromCharCode(b);
	return btoa(str).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}
