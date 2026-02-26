import { renderAudioPlayer, sendRpc } from "./helpers.js";

var cachedTtsEnabled = null;
var pendingStatus = null;

async function isTtsEnabled() {
	if (cachedTtsEnabled !== null) return cachedTtsEnabled;
	if (!pendingStatus) {
		pendingStatus = sendRpc("tts.status", {})
			.then((res) => {
				cachedTtsEnabled = !!(res?.ok && res.payload?.enabled === true);
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

function buildSessionMediaUrl(sessionKey, audioPath) {
	if (!(sessionKey && audioPath)) return null;
	var filename = String(audioPath).split("/").pop();
	if (!filename) return null;
	return `/api/sessions/${encodeURIComponent(sessionKey)}/media/${encodeURIComponent(filename)}`;
}

function upsertVoiceWarning(messageEl, warningText) {
	if (!messageEl) return;
	var warningEl = messageEl.querySelector(".msg-voice-warning");
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

function ensureVoicePlayerSlot(messageEl) {
	if (!messageEl) return null;
	var slot = messageEl.querySelector(".msg-voice-player-slot");
	if (slot) return slot;
	slot = document.createElement("div");
	slot.className = "msg-voice-player-slot";
	messageEl.insertBefore(slot, messageEl.firstChild);
	return slot;
}

function renderPersistedAudio(messageEl, sessionKey, audioPath, autoplay) {
	var src = buildSessionMediaUrl(sessionKey, audioPath);
	if (!src) return false;
	var slot = ensureVoicePlayerSlot(messageEl);
	if (!slot) return false;
	slot.textContent = "";
	renderAudioPlayer(slot, src, autoplay === true);
	return true;
}

export async function attachMessageVoiceControl(options) {
	var messageEl = options?.messageEl;
	var footerEl = options?.footerEl;
	if (!(messageEl && footerEl)) return;

	var sessionKey = options?.sessionKey;
	var text = String(options?.text || "").trim();
	var runId = options?.runId;
	var messageIndex = options?.messageIndex;
	var audioPath = options?.audioPath;
	var audioWarning = options?.audioWarning;
	var forceAction = options?.forceAction === true;
	var autoplayOnGenerate = options?.autoplayOnGenerate === true;

	upsertVoiceWarning(messageEl, audioWarning || null);
	if (!text || audioPath) return;

	var showAction = forceAction || (await isTtsEnabled());
	if (!showAction) return;

	var actionBtn = footerEl.querySelector(".msg-voice-action");
	if (!actionBtn) {
		actionBtn = document.createElement("button");
		actionBtn.type = "button";
		actionBtn.className = "msg-voice-action";
		actionBtn.textContent = "Voice it";
		footerEl.appendChild(actionBtn);
	}

	actionBtn.onclick = async () => {
		if (!sessionKey) {
			upsertVoiceWarning(messageEl, "Cannot generate voice: missing session key.");
			return;
		}

		var params = { key: sessionKey };
		if (runId) params.runId = runId;
		if (Number.isInteger(messageIndex) && messageIndex >= 0) {
			params.messageIndex = messageIndex;
		}
		if (!(params.runId || Number.isInteger(params.messageIndex))) {
			upsertVoiceWarning(messageEl, "Cannot generate voice for this message.");
			return;
		}

		actionBtn.disabled = true;
		actionBtn.textContent = "Voicing...";
		var result = await sendRpc("sessions.voice.generate", params);
		if (!(result?.ok && result.payload?.audio)) {
			actionBtn.disabled = false;
			actionBtn.textContent = "Retry voice";
			var errorText = result?.error?.message || "Voice generation failed.";
			upsertVoiceWarning(messageEl, errorText);
			return;
		}

		if (!renderPersistedAudio(messageEl, sessionKey, result.payload.audio, autoplayOnGenerate)) {
			actionBtn.disabled = false;
			actionBtn.textContent = "Retry voice";
			upsertVoiceWarning(messageEl, "Voice audio generated but could not be rendered.");
			return;
		}

		upsertVoiceWarning(messageEl, null);
		actionBtn.remove();
	};
}
