// ── Voice input module ───────────────────────────────────────
// Handles microphone recording and speech-to-text transcription.

import { chatAddMsg, smartScrollToBottom } from "./chat-ui";
import * as gon from "./gon";
import { renderAudioPlayer, renderMarkdown, sendRpc, warmAudioPlayback } from "./helpers";
import { t } from "./i18n";
import { bumpSessionCount, seedSessionPreviewFromUserText, setSessionReplying } from "./sessions";
import * as S from "./state";

let micBtn: HTMLButtonElement | null = null;
let mediaRecorder: MediaRecorder | null = null;
let audioChunks: Blob[] = [];
let sttConfigured = false;
let isRecording = false;
let isStarting = false;
let transcribingEl: HTMLElement | null = null;

/** Check if voice feature is enabled. */
function isVoiceEnabled(): boolean {
	return gon.get("voice_enabled") === true;
}

/** Check if STT is available and enable/disable mic button. */
async function checkSttStatus(): Promise<void> {
	// If voice feature is disabled, always hide the button
	if (!isVoiceEnabled()) {
		sttConfigured = false;
		updateMicButton();
		return;
	}
	const res = await sendRpc<{ configured?: boolean }>("stt.status", {});
	if (res?.ok && res.payload) {
		sttConfigured = res.payload.configured === true;
	} else {
		sttConfigured = false;
	}
	updateMicButton();
}

/** Update mic button visibility based on STT configuration. */
function updateMicButton(): void {
	if (!micBtn) return;
	// Hide button when voice feature is disabled or STT is not configured
	micBtn.style.display = sttConfigured && isVoiceEnabled() ? "" : "none";
	// Disable only when not connected (button is only visible when STT configured)
	micBtn.disabled = !S.connected;
	micBtn.title = isStarting ? t("chat:micStarting") : isRecording ? t("chat:micStopAndSend") : t("chat:micTooltip");
}

/** Pause all currently playing audio elements on the page. */
function stopAllAudio(): void {
	for (const audio of document.querySelectorAll("audio")) {
		if (!audio.paused) {
			audio.pause();
			console.debug("[voice] paused playing audio");
		}
	}
}

/** Start recording audio from the microphone. */
async function startRecording(): Promise<void> {
	if (isRecording || isStarting || !sttConfigured) return;

	// Stop any playing audio so the mic doesn't pick up speaker output.
	stopAllAudio();

	isStarting = true;
	micBtn?.classList.add("starting");
	micBtn?.setAttribute("aria-busy", "true");
	micBtn!.title = t("chat:micStarting");

	try {
		const stream = await navigator.mediaDevices.getUserMedia({ audio: true });
		audioChunks = [];
		let recordingUiShown = false;

		function showRecordingUi(): void {
			if (recordingUiShown || !micBtn) return;
			recordingUiShown = true;
			isStarting = false;
			micBtn.classList.remove("starting");
			micBtn.removeAttribute("aria-busy");
			micBtn.classList.add("recording");
			micBtn.setAttribute("aria-pressed", "true");
			micBtn.title = t("chat:micStopAndSend");
		}

		// Use webm/opus if available, fall back to audio/webm
		const mimeType = MediaRecorder.isTypeSupported("audio/webm;codecs=opus") ? "audio/webm;codecs=opus" : "audio/webm";

		mediaRecorder = new MediaRecorder(stream, { mimeType });

		mediaRecorder.ondataavailable = (e: BlobEvent): void => {
			if (e.data.size > 0) {
				audioChunks.push(e.data);
				showRecordingUi();
			}
		};

		// Recorder start means stop is now valid; visual indicator waits for actual audio data.
		mediaRecorder.onstart = (): void => {
			isRecording = true;
		};

		const audioTrack = stream.getAudioTracks()[0];
		if (audioTrack && !audioTrack.muted) {
			setTimeout(showRecordingUi, 150);
		} else if (audioTrack) {
			audioTrack.addEventListener("unmute", showRecordingUi, { once: true });
		}

		mediaRecorder.onstop = async (): Promise<void> => {
			// Stop all tracks to release the microphone
			for (const track of stream.getTracks()) {
				track.stop();
			}
			await transcribeAudio();
		};

		mediaRecorder.start(250);
	} catch (err) {
		isStarting = false;
		isRecording = false;
		if (micBtn) {
			micBtn.classList.remove("starting");
			micBtn.removeAttribute("aria-busy");
			micBtn.setAttribute("aria-pressed", "false");
			micBtn.title = t("chat:micTooltip");
		}
		console.error("Failed to start recording:", err);
		// Show user-friendly error
		if ((err as DOMException).name === "NotAllowedError") {
			alert(t("settings:voice.micDenied"));
		} else if ((err as DOMException).name === "NotFoundError") {
			alert(t("settings:voice.noMicFound"));
		}
	}
}

/** Stop recording and trigger transcription. */
function stopRecording(): void {
	if (!(isRecording && mediaRecorder)) return;

	isStarting = false;
	isRecording = false;
	micBtn?.classList.remove("starting");
	micBtn?.removeAttribute("aria-busy");
	micBtn?.classList.remove("recording");
	micBtn?.setAttribute("aria-pressed", "false");
	micBtn?.classList.add("transcribing");
	micBtn!.title = t("chat:voiceTranscribing");

	// Stop the recorder, which triggers onstop -> transcribeAudio
	mediaRecorder.stop();
}

/** Cancel recording without sending -- discards audio chunks. */
function cancelRecording(): void {
	if (!(isRecording && mediaRecorder)) return;

	console.debug("[voice] recording cancelled via Escape");

	// Prevent onstop from transcribing by clearing chunks first.
	audioChunks = [];

	isStarting = false;
	isRecording = false;
	micBtn?.classList.remove("starting", "recording");
	micBtn?.removeAttribute("aria-busy");
	micBtn?.setAttribute("aria-pressed", "false");
	micBtn!.title = t("chat:micTooltip");

	// Stop the recorder -- onstop will see empty chunks and bail out.
	mediaRecorder.stop();
}

/** Create transcribing indicator element. */
function createTranscribingIndicator(message: string, isError: boolean): HTMLElement {
	const el = document.createElement("div");
	el.className = "msg voice-transcribing";

	const spinner = document.createElement("span");
	spinner.className = "voice-transcribing-spinner";

	const text = document.createElement("span");
	text.className = "voice-transcribing-text";
	if (isError) text.classList.add("text-[var(--error)]");
	text.textContent = message;

	if (!isError) el.appendChild(spinner);
	el.appendChild(text);
	return el;
}

/** Update transcribing element with a message. */
function updateTranscribingMessage(message: string, isError: boolean): void {
	if (!transcribingEl) return;
	transcribingEl.textContent = "";
	const text = document.createElement("span");
	text.className = "voice-transcribing-text";
	text.classList.add(isError ? "text-[var(--error)]" : "text-[var(--muted)]");
	text.textContent = message;
	transcribingEl.appendChild(text);
}

/** Show a temporary message then remove the transcribing element. */
function showTemporaryMessage(message: string, isError: boolean, delayMs: number): void {
	updateTranscribingMessage(message, isError);
	setTimeout(() => {
		if (transcribingEl) {
			transcribingEl.remove();
			transcribingEl = null;
		}
	}, delayMs);
}

/** Remove transcribing indicator and reset mic button state. */
function cleanupTranscribingState(): void {
	isStarting = false;
	micBtn?.classList.remove("starting");
	micBtn?.removeAttribute("aria-busy");
	micBtn?.classList.remove("transcribing");
	micBtn!.title = t("chat:micTooltip");
	if (transcribingEl) {
		transcribingEl.remove();
		transcribingEl = null;
	}
}

/** Send transcribed text as a chat message. */
function sendTranscribedMessage(text: string, audioFilename: string | null): void {
	// Unlock audio playback while we still have user-gesture context.
	warmAudioPlayback();

	// Add user message to chat (like sendChat does), including the recorded
	// audio player when we have a saved filename from the upload endpoint.
	if (audioFilename) {
		const userEl = chatAddMsg("user", "", true);
		if (userEl) {
			const audioSrc = `/api/sessions/${encodeURIComponent(S.activeSessionKey)}/media/${encodeURIComponent(audioFilename)}`;
			renderAudioPlayer(userEl, audioSrc);
			if (text) {
				const textWrap = document.createElement("div");
				textWrap.className = "mt-2";
				// Safe: renderMarkdown escapes untrusted content before formatting tags.
				textWrap.textContent = "";
				const rendered = renderMarkdown(text);
				const fragment = document.createRange().createContextualFragment(rendered);
				textWrap.appendChild(fragment);
				userEl.appendChild(textWrap);
			}
		}
	} else {
		chatAddMsg("user", renderMarkdown(text), true);
	}

	// Send the message
	const chatParams: { text: string; _input_medium: string; _audio_filename?: string; model?: string } = {
		text: text,
		_input_medium: "voice",
	};
	if (audioFilename) {
		chatParams._audio_filename = audioFilename;
	}
	const selectedModel = S.selectedModelId;
	if (selectedModel) {
		chatParams.model = selectedModel;
	}
	bumpSessionCount(S.activeSessionKey, 1);
	seedSessionPreviewFromUserText(S.activeSessionKey, text);
	setSessionReplying(S.activeSessionKey, true);
	sendRpc("chat.send", chatParams).then((sendRes) => {
		if (sendRes && !sendRes.ok && sendRes.error) {
			chatAddMsg("error", sendRes.error?.message || "Request failed");
		}
	});
}

/** Send recorded audio to STT service for transcription via upload endpoint. */
async function transcribeAudio(): Promise<void> {
	if (audioChunks.length === 0) {
		cleanupTranscribingState();
		return;
	}

	// Show transcribing indicator in chat immediately
	if (S.chatMsgBox) {
		transcribingEl = createTranscribingIndicator(t("chat:voiceTranscribingMessage"), false);
		(S.chatMsgBox as HTMLElement).appendChild(transcribingEl);
		smartScrollToBottom();
	}

	try {
		const blob = new Blob(audioChunks, { type: "audio/webm" });
		audioChunks = [];

		const resp = await fetch(`/api/sessions/${encodeURIComponent(S.activeSessionKey)}/upload?transcribe=true`, {
			method: "POST",
			headers: { "Content-Type": blob.type || "audio/webm" },
			body: blob,
		});
		interface TranscriptionUploadResponse {
			ok?: boolean;
			transcription?: { text?: string };
			filename?: string;
			transcriptionError?: string;
			error?: string;
		}

		const res: TranscriptionUploadResponse = await resp.json();

		micBtn?.classList.remove("transcribing");
		micBtn!.title = t("chat:micTooltip");

		if (res.ok && res.transcription?.text) {
			const text = String(res.transcription.text).trim();
			const audioFilename = typeof res.filename === "string" ? res.filename.trim() : "";
			if (text) {
				cleanupTranscribingState();
				sendTranscribedMessage(text, audioFilename || null);
			} else {
				showTemporaryMessage("No speech detected", false, 2000);
			}
		} else if (res.transcriptionError) {
			console.error("Transcription failed:", res.transcriptionError);
			showTemporaryMessage(`Transcription failed: ${res.transcriptionError}`, true, 4000);
		} else if (!res.ok) {
			console.error("Upload failed:", res.error);
			showTemporaryMessage(`Upload failed: ${res.error || "Unknown error"}`, true, 4000);
		}
	} catch (err) {
		console.error("Transcription error:", err);
		micBtn?.classList.remove("transcribing");
		micBtn!.title = t("chat:micTooltip");
		showTemporaryMessage("Transcription error", true, 4000);
	}
}

/** Handle click on mic button - toggle recording. */
function onMicClick(e: Event): void {
	e.preventDefault();
	if (isRecording) {
		stopRecording();
	} else {
		startRecording();
	}
}

/** Initialize voice input with the mic button element. */
export function initVoiceInput(btn: HTMLButtonElement | null): void {
	if (!btn) return;

	micBtn = btn;

	// Check STT status on init
	checkSttStatus();

	// Click to toggle recording (start on first click, stop on second)
	micBtn.addEventListener("click", onMicClick);

	// Keyboard accessibility: Space/Enter to toggle
	micBtn.addEventListener("keydown", (e: KeyboardEvent): void => {
		if (e.key === " " || e.key === "Enter") {
			e.preventDefault();
			onMicClick(e);
		}
	});

	// Escape cancels recording without sending.
	document.addEventListener("keydown", (e: KeyboardEvent): void => {
		if (e.key === "Escape" && isRecording) {
			e.preventDefault();
			cancelRecording();
		}
	});

	// Re-check STT status when voice config changes
	window.addEventListener("voice-config-changed", checkSttStatus);
}

/** Teardown voice input module. */
export function teardownVoiceInput(): void {
	if (isRecording && mediaRecorder) {
		mediaRecorder.stop();
	}
	window.removeEventListener("voice-config-changed", checkSttStatus);
	micBtn = null;
	mediaRecorder = null;
	audioChunks = [];
	isRecording = false;
}

/** Re-check STT status (can be called externally). */
export function refreshVoiceStatus(): void {
	checkSttStatus();
}
