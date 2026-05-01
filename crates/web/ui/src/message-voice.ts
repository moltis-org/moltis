import { renderAudioPlayer, sendRpc } from "./helpers";

let cachedTtsEnabled: boolean | null = null;
let pendingStatus: Promise<boolean> | null = null;

async function isTtsEnabled(): Promise<boolean> {
	if (cachedTtsEnabled !== null) return cachedTtsEnabled;
	if (!pendingStatus) {
		pendingStatus = sendRpc("tts.status", {})
			.then((res) => {
				cachedTtsEnabled = !!(res?.ok && (res.payload as Record<string, unknown> | undefined)?.enabled === true);
				return cachedTtsEnabled;
			})
			.catch(() => {
				cachedTtsEnabled = false;
				return false;
			})
			.finally(() => {
				pendingStatus = null;
			});
	}
	return pendingStatus;
}

function buildSessionMediaUrl(sessionKey: string | undefined, audioPath: string | undefined): string | null {
	if (!(sessionKey && audioPath)) return null;
	const filename = String(audioPath).split("/").pop();
	if (!filename) return null;
	return `/api/sessions/${encodeURIComponent(sessionKey)}/media/${encodeURIComponent(filename)}`;
}

function upsertVoiceWarning(messageEl: HTMLElement | null, warningText: string | null): void {
	if (!messageEl) return;
	let warningEl = messageEl.querySelector(".msg-voice-warning") as HTMLElement | null;
	if (!warningText) {
		if (warningEl) warningEl.remove();
		return;
	}
	if (!warningEl) {
		warningEl = document.createElement("div");
		warningEl.className = "voice-error-result msg-voice-warning";
		messageEl.appendChild(warningEl);
	}
	warningEl.textContent = warningText;
}

function ensureVoicePlayerSlot(messageEl: HTMLElement | null): HTMLElement | null {
	if (!messageEl) return null;
	let slot = messageEl.querySelector(".msg-voice-player-slot") as HTMLElement | null;
	if (slot) return slot;
	slot = document.createElement("div");
	slot.className = "msg-voice-player-slot";
	messageEl.insertBefore(slot, messageEl.firstChild);
	return slot;
}

export function renderPersistedAudio(
	messageEl: HTMLElement,
	sessionKey: string | undefined,
	audioPath: string | undefined,
	autoplay: boolean,
): boolean {
	const src = buildSessionMediaUrl(sessionKey, audioPath);
	if (!src) return false;
	const slot = ensureVoicePlayerSlot(messageEl);
	if (!slot) return false;
	slot.textContent = "";
	renderAudioPlayer(slot, src, autoplay === true);
	return true;
}

interface AttachMessageVoiceControlOptions {
	messageEl?: HTMLElement | null;
	footerEl?: HTMLElement | null;
	sessionKey?: string;
	text?: string;
	runId?: string;
	messageIndex?: number;
	audioPath?: string;
	audioWarning?: string;
	forceAction?: boolean;
	autoplayOnGenerate?: boolean;
}

export async function attachMessageVoiceControl(options: AttachMessageVoiceControlOptions): Promise<void> {
	const messageEl = options?.messageEl;
	const footerEl = options?.footerEl;
	if (!(messageEl && footerEl)) return;

	const sessionKey = options?.sessionKey;
	const text = String(options?.text || "").trim();
	const runId = options?.runId;
	const messageIndex = options?.messageIndex;
	const audioPath = options?.audioPath;
	const audioWarning = options?.audioWarning;
	const forceAction = options?.forceAction === true;
	const autoplayOnGenerate = options?.autoplayOnGenerate === true;

	upsertVoiceWarning(messageEl, audioWarning || null);
	if (!text || audioPath) return;

	const showAction = forceAction || (await isTtsEnabled());
	if (!showAction) return;

	let actionBtn = footerEl.querySelector(".msg-voice-action") as HTMLButtonElement | null;
	if (!actionBtn) {
		actionBtn = document.createElement("button");
		actionBtn.type = "button";
		actionBtn.className = "msg-voice-action";
		actionBtn.textContent = "Voice it";
		footerEl.appendChild(actionBtn);
	}

	actionBtn.onclick = async (): Promise<void> => {
		if (!sessionKey) {
			upsertVoiceWarning(messageEl, "Cannot generate voice: missing session key.");
			return;
		}

		const params: Record<string, unknown> = { key: sessionKey };
		if (runId) params.runId = runId;
		if (Number.isInteger(messageIndex) && (messageIndex as number) >= 0) {
			params.messageIndex = messageIndex;
		}
		if (!(params.runId || Number.isInteger(params.messageIndex))) {
			upsertVoiceWarning(messageEl, "Cannot generate voice for this message.");
			return;
		}

		actionBtn!.disabled = true;
		actionBtn!.textContent = "Voicing...";
		const result = (await sendRpc("sessions.voice.generate", params)) as unknown as Record<string, unknown>;
		if (!(result?.ok && (result.payload as Record<string, unknown> | undefined)?.audio)) {
			actionBtn!.disabled = false;
			actionBtn!.textContent = "Retry voice";
			const errorText =
				((result?.error as Record<string, unknown> | undefined)?.message as string) || "Voice generation failed.";
			upsertVoiceWarning(messageEl, errorText);
			return;
		}

		if (
			!renderPersistedAudio(
				messageEl,
				sessionKey,
				(result.payload as Record<string, unknown>).audio as string,
				autoplayOnGenerate,
			)
		) {
			actionBtn!.disabled = false;
			actionBtn!.textContent = "Retry voice";
			upsertVoiceWarning(messageEl, "Voice audio generated but could not be rendered.");
			return;
		}

		upsertVoiceWarning(messageEl, null);
		actionBtn?.remove();
	};
}
