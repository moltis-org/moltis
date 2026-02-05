// ── Voice input module ───────────────────────────────────────
// Handles microphone recording and speech-to-text transcription.

import * as gon from "./gon.js";
import { sendRpc } from "./helpers.js";
import * as S from "./state.js";

var micBtn = null;
var mediaRecorder = null;
var audioChunks = [];
var sttConfigured = false;
var isRecording = false;

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
	micBtn.title = "Record voice (hold to record)";
}

/** Start recording audio from the microphone. */
async function startRecording() {
	if (isRecording || !sttConfigured) return;

	try {
		var stream = await navigator.mediaDevices.getUserMedia({ audio: true });
		audioChunks = [];

		// Use webm/opus if available, fall back to audio/webm
		var mimeType = MediaRecorder.isTypeSupported("audio/webm;codecs=opus") ? "audio/webm;codecs=opus" : "audio/webm";

		mediaRecorder = new MediaRecorder(stream, { mimeType });

		mediaRecorder.ondataavailable = (e) => {
			if (e.data.size > 0) {
				audioChunks.push(e.data);
			}
		};

		mediaRecorder.onstop = async () => {
			// Stop all tracks to release the microphone
			for (var track of stream.getTracks()) {
				track.stop();
			}
			await transcribeAudio();
		};

		mediaRecorder.start();
		isRecording = true;
		micBtn.classList.add("recording");
		micBtn.setAttribute("aria-pressed", "true");
	} catch (err) {
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

	isRecording = false;
	micBtn.classList.remove("recording");
	micBtn.setAttribute("aria-pressed", "false");
	micBtn.classList.add("transcribing");

	// Stop the recorder, which triggers onstop -> transcribeAudio
	mediaRecorder.stop();
}

/** Send recorded audio to STT service for transcription. */
async function transcribeAudio() {
	if (audioChunks.length === 0) {
		micBtn.classList.remove("transcribing");
		return;
	}

	try {
		var blob = new Blob(audioChunks, { type: "audio/webm" });
		audioChunks = [];

		// Convert to base64
		var buffer = await blob.arrayBuffer();
		var base64 = btoa(String.fromCharCode(...new Uint8Array(buffer)));

		var res = await sendRpc("stt.transcribe", {
			audio: base64,
			format: "webm",
		});

		micBtn.classList.remove("transcribing");

		if (res?.ok && res.payload?.text) {
			// Insert transcribed text into chat input
			var text = res.payload.text.trim();
			if (text && S.chatInput) {
				var current = S.chatInput.value;
				var newText = current ? `${current} ${text}` : text;
				S.chatInput.value = newText;
				S.chatInput.focus();
				// Trigger resize
				S.chatInput.dispatchEvent(new Event("input", { bubbles: true }));
			}
		} else if (res?.error) {
			console.error("Transcription failed:", res.error.message);
		}
	} catch (err) {
		console.error("Transcription error:", err);
		micBtn.classList.remove("transcribing");
	}
}

/** Handle mouse/touch start on mic button. */
function onMicDown(e) {
	e.preventDefault();
	startRecording();
}

/** Handle mouse/touch end on mic button. */
function onMicUp(e) {
	e.preventDefault();
	stopRecording();
}

/** Initialize voice input with the mic button element. */
export function initVoiceInput(btn) {
	if (!btn) return;

	micBtn = btn;

	// Check STT status on init
	checkSttStatus();

	// Set up press-and-hold recording
	micBtn.addEventListener("mousedown", onMicDown);
	micBtn.addEventListener("mouseup", onMicUp);
	micBtn.addEventListener("mouseleave", onMicUp);
	micBtn.addEventListener("touchstart", onMicDown, { passive: false });
	micBtn.addEventListener("touchend", onMicUp, { passive: false });
	micBtn.addEventListener("touchcancel", onMicUp, { passive: false });

	// Keyboard accessibility: Space/Enter to toggle
	micBtn.addEventListener("keydown", (e) => {
		if (e.key === " " || e.key === "Enter") {
			e.preventDefault();
			if (isRecording) {
				stopRecording();
			} else {
				startRecording();
			}
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
