// ── Voice input module ───────────────────────────────────────
// Handles microphone recording and speech-to-text transcription.

import { chatAddMsg } from "./chat-ui.js";
import * as gon from "./gon.js";
import { renderMarkdown, sendRpc, warmAudioPlayback } from "./helpers.js";
import { bumpSessionCount, setSessionReplying } from "./sessions.js";
import * as S from "./state.js";

var micBtn = null;
var mediaRecorder = null;
var audioChunks = [];
var sttConfigured = false;
var isRecording = false;
var isStarting = false;
var transcribingEl = null;

/** Check if voice feature is enabled. */
function isVoiceEnabled() {
	return gon.get("voice_enabled") === true;
}

/** Check if STT is available and enable/disable mic button. */
async function checkSttStatus() {
	// If voice feature is disabled, always hide the button
	if (!isVoiceEnabled()) {
		sttConfigured = false;
		updateMicButton();
		return;
	}
	var res = await sendRpc("stt.status", {});
	if (res?.ok && res.payload) {
		sttConfigured = res.payload.configured === true;
	} else {
		sttConfigured = false;
	}
	updateMicButton();
}

/** Update mic button visibility based on STT configuration. */
function updateMicButton() {
	if (!micBtn) return;
	// Hide button when voice feature is disabled or STT is not configured
	micBtn.style.display = sttConfigured && isVoiceEnabled() ? "" : "none";
	// Disable only when not connected (button is only visible when STT configured)
	micBtn.disabled = !S.connected;
	micBtn.title = isStarting
		? "Starting microphone..."
		: isRecording
			? "Click to stop and send"
			: "Click to start recording";
}

/** Pause all currently playing audio elements on the page. */
function stopAllAudio() {
	for (var audio of document.querySelectorAll("audio")) {
		if (!audio.paused) {
			audio.pause();
			console.debug("[voice] paused playing audio");
		}
	}
}

/** Start recording audio from the microphone. */
async function startRecording() {
	if (isRecording || isStarting || !sttConfigured) return;

	// Stop any playing audio so the mic doesn't pick up speaker output.
	stopAllAudio();

	isStarting = true;
	micBtn.classList.add("starting");
	micBtn.setAttribute("aria-busy", "true");
	micBtn.title = "Starting microphone...";

	try {
		var stream = await navigator.mediaDevices.getUserMedia({ audio: true });
		audioChunks = [];
		var recordingUiShown = false;

		function showRecordingUi() {
			if (recordingUiShown || !micBtn) return;
			recordingUiShown = true;
			isStarting = false;
			micBtn.classList.remove("starting");
			micBtn.removeAttribute("aria-busy");
			micBtn.classList.add("recording");
			micBtn.setAttribute("aria-pressed", "true");
			micBtn.title = "Click to stop and send";
		}

		// Use webm/opus if available, fall back to audio/webm
		var mimeType = MediaRecorder.isTypeSupported("audio/webm;codecs=opus") ? "audio/webm;codecs=opus" : "audio/webm";

		mediaRecorder = new MediaRecorder(stream, { mimeType });

		mediaRecorder.ondataavailable = (e) => {
			if (e.data.size > 0) {
				audioChunks.push(e.data);
				showRecordingUi();
			}
		};

		// Recorder start means stop is now valid; visual indicator waits for actual audio data.
		mediaRecorder.onstart = () => {
			isRecording = true;
		};

		var audioTrack = stream.getAudioTracks()[0];
		if (audioTrack && !audioTrack.muted) {
			setTimeout(showRecordingUi, 150);
		} else if (audioTrack) {
			audioTrack.addEventListener("unmute", showRecordingUi, { once: true });
		}

		mediaRecorder.onstop = async () => {
			// Stop all tracks to release the microphone
			for (var track of stream.getTracks()) {
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
			micBtn.title = "Click to start recording";
		}
		console.error("Failed to start recording:", err);
		// Show user-friendly error
		if (err.name === "NotAllowedError") {
			alert("Microphone permission denied. Please allow microphone access in your browser settings.");
		} else if (err.name === "NotFoundError") {
			alert("No microphone found. Please connect a microphone and try again.");
		}
	}
}

/** Stop recording and trigger transcription. */
function stopRecording() {
	if (!(isRecording && mediaRecorder)) return;

	isStarting = false;
	isRecording = false;
	micBtn.classList.remove("starting");
	micBtn.removeAttribute("aria-busy");
	micBtn.classList.remove("recording");
	micBtn.setAttribute("aria-pressed", "false");
	micBtn.classList.add("transcribing");
	micBtn.title = "Transcribing...";

	// Stop the recorder, which triggers onstop -> transcribeAudio
	mediaRecorder.stop();
}

/** Cancel recording without sending — discards audio chunks. */
function cancelRecording() {
	if (!(isRecording && mediaRecorder)) return;

	console.debug("[voice] recording cancelled via Escape");

	// Prevent onstop from transcribing by clearing chunks first.
	audioChunks = [];

	isStarting = false;
	isRecording = false;
	micBtn.classList.remove("starting", "recording");
	micBtn.removeAttribute("aria-busy");
	micBtn.setAttribute("aria-pressed", "false");
	micBtn.title = "Click to start recording";

	// Stop the recorder — onstop will see empty chunks and bail out.
	mediaRecorder.stop();
}

/** Create transcribing indicator element. */
function createTranscribingIndicator(message, isError) {
	var el = document.createElement("div");
	el.className = "msg voice-transcribing";

	var spinner = document.createElement("span");
	spinner.className = "voice-transcribing-spinner";

	var text = document.createElement("span");
	text.className = "voice-transcribing-text";
	if (isError) text.classList.add("text-[var(--error)]");
	text.textContent = message;

	if (!isError) el.appendChild(spinner);
	el.appendChild(text);
	return el;
}

/** Update transcribing element with a message. */
function updateTranscribingMessage(message, isError) {
	if (!transcribingEl) return;
	transcribingEl.textContent = "";
	var text = document.createElement("span");
	text.className = "voice-transcribing-text";
	text.classList.add(isError ? "text-[var(--error)]" : "text-[var(--muted)]");
	text.textContent = message;
	transcribingEl.appendChild(text);
}

/** Show a temporary message then remove the transcribing element. */
function showTemporaryMessage(message, isError, delayMs) {
	updateTranscribingMessage(message, isError);
	setTimeout(() => {
		if (transcribingEl) {
			transcribingEl.remove();
			transcribingEl = null;
		}
	}, delayMs);
}

/** Remove transcribing indicator and reset mic button state. */
function cleanupTranscribingState() {
	isStarting = false;
	micBtn.classList.remove("starting");
	micBtn.removeAttribute("aria-busy");
	micBtn.classList.remove("transcribing");
	micBtn.title = "Click to start recording";
	if (transcribingEl) {
		transcribingEl.remove();
		transcribingEl = null;
	}
}

/** Send transcribed text as a chat message. */
function sendTranscribedMessage(text) {
	// Unlock audio playback while we still have user-gesture context.
	warmAudioPlayback();

	// Add user message to chat (like sendChat does)
	chatAddMsg("user", renderMarkdown(text), true);

	// Send the message
	var chatParams = { text: text, _input_medium: "voice" };
	var selectedModel = S.selectedModelId;
	if (selectedModel) {
		chatParams.model = selectedModel;
	}
	bumpSessionCount(S.activeSessionKey, 1);
	setSessionReplying(S.activeSessionKey, true);
	sendRpc("chat.send", chatParams).then((sendRes) => {
		if (sendRes && !sendRes.ok && sendRes.error) {
			chatAddMsg("error", sendRes.error.message || "Request failed");
		}
	});
}

/** Send recorded audio to STT service for transcription via upload endpoint. */
async function transcribeAudio() {
	if (audioChunks.length === 0) {
		cleanupTranscribingState();
		return;
	}

	// Show transcribing indicator in chat immediately
	if (S.chatMsgBox) {
		transcribingEl = createTranscribingIndicator("Transcribing voice...", false);
		S.chatMsgBox.appendChild(transcribingEl);
		S.chatMsgBox.scrollTop = S.chatMsgBox.scrollHeight;
	}

	try {
		var blob = new Blob(audioChunks, { type: "audio/webm" });
		audioChunks = [];

		var resp = await fetch(`/api/sessions/${encodeURIComponent(S.activeSessionKey)}/upload?transcribe=true`, {
			method: "POST",
			headers: { "Content-Type": blob.type || "audio/webm" },
			body: blob,
		});
		var res = await resp.json();

		micBtn.classList.remove("transcribing");
		micBtn.title = "Click to start recording";

		if (res.ok && res.transcription?.text) {
			var text = res.transcription.text.trim();
			if (text) {
				cleanupTranscribingState();
				sendTranscribedMessage(text);
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
		micBtn.classList.remove("transcribing");
		micBtn.title = "Click to start recording";
		showTemporaryMessage("Transcription error", true, 4000);
	}
}

/** Handle click on mic button - toggle recording. */
function onMicClick(e) {
	e.preventDefault();
	if (isRecording) {
		stopRecording();
	} else {
		startRecording();
	}
}

/** Initialize voice input with the mic button element. */
export function initVoiceInput(btn) {
	if (!btn) return;

	micBtn = btn;

	// Check STT status on init
	checkSttStatus();

	// Click to toggle recording (start on first click, stop on second)
	micBtn.addEventListener("click", onMicClick);

	// Keyboard accessibility: Space/Enter to toggle
	micBtn.addEventListener("keydown", (e) => {
		if (e.key === " " || e.key === "Enter") {
			e.preventDefault();
			onMicClick(e);
		}
	});

	// Escape cancels recording without sending.
	document.addEventListener("keydown", (e) => {
		if (e.key === "Escape" && isRecording) {
			e.preventDefault();
			cancelRecording();
		}
	});

	// Re-check STT status when voice config changes
	window.addEventListener("voice-config-changed", checkSttStatus);
}

/** Teardown voice input module. */
export function teardownVoiceInput() {
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
export function refreshVoiceStatus() {
	checkSttStatus();
}
