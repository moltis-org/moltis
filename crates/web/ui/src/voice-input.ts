// ── Voice input module ───────────────────────────────────────
// Handles microphone recording and speech-to-text transcription.
// Supports three input modes:
//   1. Toggle (default): click mic to start, click again to stop & send
//   2. Push-to-talk (PTT): hold a hotkey to record, release to send
//   3. VAD (continuous): click waveform button to enter hands-free mode;
//      auto-detects speech via energy-based VAD, auto-sends on silence,
//      auto-re-listens after TTS playback finishes.

import { chatAddMsg, smartScrollToBottom } from "./chat-ui";
import * as gon from "./gon";
import { renderAudioPlayer, renderMarkdown, sendRpc, warmAudioPlayback } from "./helpers";
import { t } from "./i18n";
import { bumpSessionCount, seedSessionPreviewFromUserText, setSessionReplying } from "./sessions";
import * as S from "./state";
import { sessionStore } from "./stores/session-store";

// ── Shared state ─────────────────────────────────────────────
let micBtn: HTMLButtonElement | null = null;
let vadBtn: HTMLButtonElement | null = null;
let mediaRecorder: MediaRecorder | null = null;
let audioChunks: Blob[] = [];
let sttConfigured = false;
let isRecording = false;
let isStarting = false;
let recordingCancelled = false;
let transcribingEl: HTMLElement | null = null;

// ── PTT state ────────────────────────────────────────────────
let pttKey = localStorage.getItem("moltis_ptt_key") || "F13";
let pttActive = false;

// ── Tab coordination (prevent dual-tab recording) ────────────
const voiceLockChannel = typeof BroadcastChannel !== "undefined" ? new BroadcastChannel("moltis_voice_lock") : null;
let voiceLockedByOtherTab = false;
if (voiceLockChannel) {
	voiceLockChannel.onmessage = (e: MessageEvent): void => {
		if (e.data?.type === "voice_lock") {
			voiceLockedByOtherTab = true;
			console.debug("[voice] another tab claimed voice lock");
		} else if (e.data?.type === "voice_unlock") {
			voiceLockedByOtherTab = false;
			console.debug("[voice] another tab released voice lock");
		}
	};
}
function claimVoiceLock(): void {
	voiceLockedByOtherTab = false;
	voiceLockChannel?.postMessage({ type: "voice_lock" });
}
function releaseVoiceLock(): void {
	voiceLockChannel?.postMessage({ type: "voice_unlock" });
}

// ── VAD state ────────────────────────────────────────────────
let vadActive = false;
let vadStream: MediaStream | null = null;
let vadAudioCtx: AudioContext | null = null;
let vadAnalyser: AnalyserNode | null = null;
let vadDataArray: Uint8Array<ArrayBuffer> | null = null;
let vadRafId: number | null = null;
let vadSpeechDetected = false;
let vadSilenceStart = 0;
let vadMutedForTts = false;
let vadSensitivity = parseInt(localStorage.getItem("moltis_vad_sensitivity") || "50", 10);
let vadSpeechThreshold = sensitivityToThreshold(vadSensitivity);

/** Map sensitivity percentage (0-100) to RMS threshold.
 *  0% = least sensitive (threshold 0.08), 100% = most sensitive (threshold 0.005). */
function sensitivityToThreshold(pct: number): number {
	const clamped = Math.max(0, Math.min(100, pct));
	return 0.08 * (0.005 / 0.08) ** (clamped / 100);
}

const VAD_SILENCE_DURATION = 2500;
const VAD_DEBOUNCE_SPEECH = 250;
let vadSpeechStart = 0;
let vadRecordingStart = 0;
let vadMediaRecorder: MediaRecorder | null = null;
let vadTranscribing = false;
let vadReacquiring = false;
let vadStarting = false;
let vadSourceNode: MediaStreamAudioSourceNode | null = null;

// VAD monitor loop mutable state (avoids static properties on function)
let vadMonitorMuteStart = 0;

function isSttEnabled(): boolean {
	return gon.get("stt_enabled") !== false;
}

/** Check if STT is available and enable/disable buttons. */
async function checkSttStatus(): Promise<void> {
	if (!isSttEnabled()) {
		sttConfigured = false;
		if (vadActive) stopVad();
		updateMicButton();
		updateVadButton();
		return;
	}
	const res = await sendRpc<{ configured?: boolean }>("stt.status", {});
	if (res?.ok && res.payload) {
		sttConfigured = res.payload.configured === true;
	} else {
		sttConfigured = false;
	}
	if (!sttConfigured && vadActive) stopVad();
	updateMicButton();
	updateVadButton();
}

// ── Mic button (toggle mode) ─────────────────────────────────

function updateMicButton(): void {
	if (!micBtn) return;
	micBtn.style.display = sttConfigured && isSttEnabled() ? "" : "none";
	micBtn.disabled = !S.connected;
	micBtn.title = isStarting ? t("chat:micStarting") : isRecording ? t("chat:micStopAndSend") : t("chat:micTooltip");
}

// ── VAD button ───────────────────────────────────────────────

function updateVadButton(): void {
	if (!vadBtn) return;
	vadBtn.style.display = sttConfigured && isSttEnabled() ? "" : "none";
	vadBtn.disabled = !S.connected;
	vadBtn.title = vadActive ? t("chat:vadStopTooltip") : t("chat:vadTooltip");
}

// ── Audio helpers ────────────────────────────────────────────

function stopAllAudio(): void {
	for (const audio of document.querySelectorAll("audio")) {
		if (!audio.paused) {
			audio.pause();
			console.debug("[voice] paused playing audio");
		}
	}
}

function getRMS(analyser: AnalyserNode, dataArray: Uint8Array<ArrayBuffer>): number {
	analyser.getByteTimeDomainData(dataArray);
	let sum = 0;
	for (const sample of dataArray) {
		const val = (sample - 128) / 128;
		sum += val * val;
	}
	return Math.sqrt(sum / dataArray.length);
}

// ── Recording (shared by toggle + PTT + VAD) ─────────────────

interface StartRecordingOpts {
	fromVad?: boolean;
	stream?: MediaStream | null;
}

async function startRecording(opts?: StartRecordingOpts): Promise<void> {
	if (isRecording || isStarting || !sttConfigured) return;

	const fromVad = opts?.fromVad === true;
	let stream = opts?.stream ?? null;

	if (!fromVad) stopAllAudio();

	isStarting = true;
	if (micBtn && !fromVad) {
		micBtn.classList.add("starting");
		micBtn.setAttribute("aria-busy", "true");
		micBtn.title = t("chat:micStarting");
	}

	try {
		if (!stream) {
			stream = await navigator.mediaDevices.getUserMedia({
				audio: { echoCancellation: true, noiseSuppression: true, autoGainControl: true },
			});
		}
		// If recording was cancelled while getUserMedia was in-flight (e.g. quick PTT tap), bail out
		if (recordingCancelled) {
			isStarting = false;
			recordingCancelled = false;
			if (!fromVad) {
				for (const track of stream.getTracks()) track.stop();
			}
			if (micBtn) {
				micBtn.classList.remove("starting");
				micBtn.removeAttribute("aria-busy");
				micBtn.title = t("chat:micTooltip");
			}
			return;
		}
		audioChunks = [];
		let recordingUiShown = false;

		function showRecordingUi(): void {
			if (recordingUiShown) return;
			recordingUiShown = true;
			isStarting = false;
			if (fromVad) {
				vadBtn?.classList.add("vad-speech");
			} else if (micBtn) {
				micBtn.classList.remove("starting");
				micBtn.removeAttribute("aria-busy");
				micBtn.classList.add("recording");
				micBtn.setAttribute("aria-pressed", "true");
				micBtn.title = t("chat:micStopAndSend");
			}
		}

		const mimeType = MediaRecorder.isTypeSupported("audio/webm;codecs=opus") ? "audio/webm;codecs=opus" : "audio/webm";

		mediaRecorder = new MediaRecorder(stream, { mimeType });

		mediaRecorder.ondataavailable = (e: BlobEvent): void => {
			if (e.data.size > 0) {
				audioChunks.push(e.data);
				showRecordingUi();
			}
		};

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
			if (!fromVad) {
				for (const track of stream!.getTracks()) {
					track.stop();
				}
			}
			if (fromVad) vadBtn?.classList.remove("vad-speech");
			await transcribeAudio();
		};

		mediaRecorder.start(250);
	} catch (err) {
		isStarting = false;
		isRecording = false;
		if (micBtn && !fromVad) {
			micBtn.classList.remove("starting");
			micBtn.removeAttribute("aria-busy");
			micBtn.setAttribute("aria-pressed", "false");
			micBtn.title = t("chat:micTooltip");
		}
		console.error("Failed to start recording:", err);
		if ((err as DOMException).name === "NotAllowedError") {
			alert(t("settings:voice.micDenied"));
		} else if ((err as DOMException).name === "NotFoundError") {
			alert(t("settings:voice.noMicFound"));
		}
	}
}

function stopRecording(): void {
	if (!(isRecording && mediaRecorder)) return;

	isStarting = false;
	isRecording = false;
	if (micBtn) {
		micBtn.classList.remove("starting");
		micBtn.removeAttribute("aria-busy");
		micBtn.classList.remove("recording");
		micBtn.setAttribute("aria-pressed", "false");
		micBtn.classList.add("transcribing");
		micBtn.title = t("chat:voiceTranscribing");
	}
	mediaRecorder.stop();
}

function cancelRecording(): void {
	if (!(isRecording && mediaRecorder)) return;

	console.debug("[voice] recording cancelled via Escape");
	recordingCancelled = true;
	audioChunks = [];
	isStarting = false;
	isRecording = false;
	if (micBtn) {
		micBtn.classList.remove("starting", "recording");
		micBtn.removeAttribute("aria-busy");
		micBtn.setAttribute("aria-pressed", "false");
		micBtn.title = t("chat:micTooltip");
	}
	vadBtn?.classList.remove("vad-speech");
	mediaRecorder.stop();
}

// ── Transcription UI helpers ─────────────────────────────────

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

function updateTranscribingMessage(message: string, isError: boolean): void {
	if (!transcribingEl) return;
	transcribingEl.textContent = "";
	const text = document.createElement("span");
	text.className = "voice-transcribing-text";
	text.classList.add(isError ? "text-[var(--error)]" : "text-[var(--muted)]");
	text.textContent = message;
	transcribingEl.appendChild(text);
}

function showTemporaryMessage(message: string, isError: boolean, delayMs: number): void {
	updateTranscribingMessage(message, isError);
	setTimeout(() => {
		if (transcribingEl) {
			transcribingEl.remove();
			transcribingEl = null;
		}
	}, delayMs);
}

function cleanupTranscribingState(): void {
	isStarting = false;
	micBtn?.classList.remove("starting");
	micBtn?.removeAttribute("aria-busy");
	micBtn?.classList.remove("transcribing");
	if (micBtn) micBtn.title = t("chat:micTooltip");
	if (transcribingEl) {
		transcribingEl.remove();
		transcribingEl = null;
	}
}

// ── Send transcribed message ─────────────────────────────────

function sendTranscribedMessage(text: string, audioFilename: string | null): void {
	warmAudioPlayback();

	if (audioFilename) {
		const userEl = chatAddMsg("user", "", true);
		if (userEl) {
			const audioSrc = `/api/sessions/${encodeURIComponent(S.activeSessionKey)}/media/${encodeURIComponent(audioFilename)}`;
			renderAudioPlayer(userEl, audioSrc);
			if (text) {
				const textWrap = document.createElement("div");
				textWrap.className = "mt-2";
				const rendered = renderMarkdown(text);
				const fragment = document.createRange().createContextualFragment(rendered);
				textWrap.appendChild(fragment);
				userEl.appendChild(textWrap);
			}
		}
	} else {
		chatAddMsg("user", renderMarkdown(text), true);
	}

	const chatParams: { text: string; _input_medium: string; _audio_filename?: string; model?: string } = {
		text,
		_input_medium: "voice",
	};
	if (audioFilename) chatParams._audio_filename = audioFilename;
	const selectedModel = S.selectedModelId;
	if (selectedModel) chatParams.model = selectedModel;

	bumpSessionCount(S.activeSessionKey, 1);
	seedSessionPreviewFromUserText(S.activeSessionKey, text);
	setSessionReplying(S.activeSessionKey, true);
	sendRpc("chat.send", chatParams).then((sendRes) => {
		if (sendRes && !sendRes.ok && sendRes.error) {
			chatAddMsg("error", sendRes.error?.message || "Request failed");
		}
	});
}

// ── Transcription ────────────────────────────────────────────

interface TranscriptionUploadResponse {
	ok?: boolean;
	transcription?: { text?: string };
	filename?: string;
	transcriptionError?: string;
	error?: string;
}

async function transcribeAudio(): Promise<void> {
	if (recordingCancelled || audioChunks.length === 0) {
		recordingCancelled = false;
		cleanupTranscribingState();
		return;
	}
	recordingCancelled = false;

	if (S.chatMsgBox) {
		transcribingEl = createTranscribingIndicator(t("chat:voiceTranscribingMessage"), false);
		(S.chatMsgBox as HTMLElement).appendChild(transcribingEl);
		smartScrollToBottom();
	}

	try {
		const blob = new Blob(audioChunks, { type: "audio/webm" });
		audioChunks = [];

		// Skip tiny blobs that are just WebM headers with no real audio
		if (blob.size < 2000) {
			console.debug("[voice] skipping tiny blob:", blob.size, "bytes");
			cleanupTranscribingState();
			return;
		}

		// Validate EBML header (WebM magic bytes: 1A 45 DF A3)
		const headerBytes = new Uint8Array(await blob.slice(0, 4).arrayBuffer());
		if (headerBytes[0] !== 0x1a || headerBytes[1] !== 0x45 || headerBytes[2] !== 0xdf || headerBytes[3] !== 0xa3) {
			console.warn("[voice] corrupt WebM blob (bad EBML header), discarding. size:", blob.size);
			cleanupTranscribingState();
			return;
		}

		const abortCtrl = new AbortController();
		const fetchTimeout = setTimeout(() => abortCtrl.abort(), 15000);
		let res: TranscriptionUploadResponse;
		try {
			const resp = await fetch(`/api/sessions/${encodeURIComponent(S.activeSessionKey)}/upload?transcribe=true`, {
				method: "POST",
				headers: { "Content-Type": blob.type || "audio/webm" },
				body: blob,
				signal: abortCtrl.signal,
			});
			res = await resp.json();
		} finally {
			clearTimeout(fetchTimeout);
		}

		micBtn?.classList.remove("transcribing");
		if (micBtn) micBtn.title = t("chat:micTooltip");

		if (res.ok && res.transcription?.text) {
			const text = String(res.transcription.text).trim();
			const audioFilename = typeof res.filename === "string" ? res.filename.trim() : "";
			if (text) {
				cleanupTranscribingState();
				sendTranscribedMessage(text, audioFilename || null);
			} else {
				showTemporaryMessage(t("chat:voiceNoSpeech"), false, 2000);
			}
		} else if (res.transcriptionError) {
			console.error("Transcription failed:", res.transcriptionError);
			showTemporaryMessage(t("chat:voiceTranscriptionFailed", { error: res.transcriptionError }), true, 4000);
		} else if (!res.ok) {
			console.error("Upload failed:", res.error);
			showTemporaryMessage(t("chat:voiceUploadFailed", { error: res.error || t("chat:unknownError") }), true, 4000);
		}
	} catch (err) {
		console.error("Transcription error:", err);
		micBtn?.classList.remove("transcribing");
		if (micBtn) micBtn.title = t("chat:micTooltip");
		showTemporaryMessage(t("chat:voiceTranscriptionError"), true, 4000);
	}
}

// ── Toggle mode (mic button click) ───────────────────────────

function onMicClick(e: Event): void {
	e.preventDefault();
	if (vadActive) return;
	if (isRecording) {
		releaseVoiceLock();
		stopRecording();
	} else {
		if (voiceLockedByOtherTab) return;
		claimVoiceLock();
		startRecording();
	}
}

// ── PTT (push-to-talk via hotkey) ────────────────────────────

function onPttKeyDown(e: KeyboardEvent): void {
	if (e.key !== pttKey) return;
	if (vadActive || pttActive || isRecording) return;
	const isFunctionKey = /^F\d{1,2}$/.test(e.key);
	if (!isFunctionKey) {
		const tag = document.activeElement?.tagName;
		if (tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT") return;
	}
	e.preventDefault();
	if (voiceLockedByOtherTab) return;
	pttActive = true;
	claimVoiceLock();
	console.debug("[voice] PTT start:", pttKey);
	stopAllAudio();
	startRecording();
}

function onPttKeyUp(e: KeyboardEvent): void {
	if (e.key !== pttKey) return;
	if (!pttActive) return;
	e.preventDefault();
	pttActive = false;
	releaseVoiceLock();
	if (isStarting) {
		// getUserMedia still in-flight; cancel so it doesn't start an orphaned recording
		recordingCancelled = true;
	}
	console.debug("[voice] PTT release — sending");
	stopRecording();
}

// ── VAD (voice activity detection) ───────────────────────────

async function startVad(): Promise<void> {
	if (vadActive || vadStarting || isRecording || isStarting) return;
	vadStarting = true;

	console.debug("[voice] VAD starting");
	try {
		vadStream = await navigator.mediaDevices.getUserMedia({
			audio: { echoCancellation: true, noiseSuppression: true, autoGainControl: true },
		});
	} catch (err) {
		vadStarting = false;
		console.error("[voice] VAD mic access failed:", err);
		if ((err as DOMException).name === "NotAllowedError") {
			alert(t("settings:voice.micDenied"));
		}
		return;
	}

	vadStarting = false;
	vadActive = true;
	vadSpeechDetected = false;
	vadSilenceStart = 0;
	vadSpeechStart = 0;
	vadMutedForTts = false;
	claimVoiceLock();

	if (vadBtn) {
		vadBtn.classList.add("vad-active");
		vadBtn.title = t("chat:vadStopTooltip");
	}

	vadAudioCtx = new AudioContext();
	vadSourceNode = vadAudioCtx.createMediaStreamSource(vadStream);
	vadAnalyser = vadAudioCtx.createAnalyser();
	vadAnalyser.fftSize = 512;
	vadAnalyser.smoothingTimeConstant = 0.3;
	vadSourceNode.connect(vadAnalyser);
	vadDataArray = new Uint8Array(vadAnalyser.fftSize);

	vadStartContinuousRecorder();
	vadMonitorLoop();

	document.addEventListener("play", onTtsPlay, true);
	document.addEventListener("ended", onTtsEnded, true);
	document.addEventListener("pause", onTtsPause, true);
}

function vadStartContinuousRecorder(): void {
	if (!(vadActive && vadStream)) return;
	if (vadTranscribing) return;
	if (vadMediaRecorder && vadMediaRecorder.state === "recording") return;
	audioChunks = [];
	const mimeType = MediaRecorder.isTypeSupported("audio/webm;codecs=opus") ? "audio/webm;codecs=opus" : "audio/webm";
	vadMediaRecorder = new MediaRecorder(vadStream, { mimeType });
	vadMediaRecorder.ondataavailable = (e: BlobEvent): void => {
		if (e.data.size > 0) audioChunks.push(e.data);
	};
	vadMediaRecorder.onstop = async (): Promise<void> => {
		vadBtn?.classList.remove("vad-speech");
		if (audioChunks.length > 0 && vadSpeechDetected) {
			vadSpeechDetected = false;
			vadTranscribing = true;
			try {
				await transcribeAudio();
			} finally {
				vadTranscribing = false;
			}
		} else {
			audioChunks = [];
			vadSpeechDetected = false;
		}
		if (vadActive && !vadMutedForTts) {
			vadStartContinuousRecorder();
		}
	};
	vadMediaRecorder.start(250);
	console.debug("[voice] VAD continuous recorder started");
}

async function vadReacquireStream(): Promise<void> {
	try {
		const newStream = await navigator.mediaDevices.getUserMedia({
			audio: { echoCancellation: true, noiseSuppression: true, autoGainControl: true },
		});
		if (vadStream) {
			for (const track of vadStream.getTracks()) track.stop();
		}
		vadStream = newStream;
		if (vadAudioCtx && vadAnalyser) {
			if (vadSourceNode) {
				vadSourceNode.disconnect();
			}
			vadSourceNode = vadAudioCtx.createMediaStreamSource(newStream);
			vadSourceNode.connect(vadAnalyser);
		}
		if (vadMediaRecorder && vadMediaRecorder.state === "recording") {
			vadMediaRecorder.stop();
		} else {
			vadStartContinuousRecorder();
		}
		console.debug("[voice] VAD: stream reacquired successfully");
	} catch (err) {
		console.error("[voice] VAD: failed to reacquire stream:", err);
		stopVad();
	}
}

function stopVad(): void {
	if (!vadActive) return;
	console.debug("[voice] VAD stopping");

	vadActive = false;
	vadSpeechDetected = false;
	vadTranscribing = false;

	if (vadMediaRecorder && vadMediaRecorder.state !== "inactive") {
		audioChunks = [];
		vadMediaRecorder.stop();
	}
	vadMediaRecorder = null;

	if (isRecording && mediaRecorder) {
		audioChunks = [];
		isRecording = false;
		mediaRecorder.stop();
	}

	if (vadRafId) {
		cancelAnimationFrame(vadRafId);
		vadRafId = null;
	}

	if (vadSourceNode) {
		vadSourceNode.disconnect();
		vadSourceNode = null;
	}
	if (vadAudioCtx) {
		vadAudioCtx.close().catch(() => {});
		vadAudioCtx = null;
		vadAnalyser = null;
		vadDataArray = null;
	}

	if (vadStream) {
		for (const track of vadStream.getTracks()) track.stop();
		vadStream = null;
	}

	if (vadBtn) {
		vadBtn.classList.remove("vad-active", "vad-speech", "vad-listening");
		vadBtn.title = t("chat:vadTooltip");
	}

	releaseVoiceLock();

	document.removeEventListener("play", onTtsPlay, true);
	document.removeEventListener("ended", onTtsEnded, true);
	document.removeEventListener("pause", onTtsPause, true);
}

function vadMonitorLoop(): void {
	if (!(vadActive && vadAnalyser && vadDataArray)) return;

	// Health check: resume AudioContext if browser suspended it
	if (vadAudioCtx && vadAudioCtx.state === "suspended") {
		console.debug("[voice] VAD: AudioContext suspended, resuming");
		vadAudioCtx.resume().catch(() => {});
	}

	// Health check: if the mic stream track ended, reacquire it (guarded to prevent concurrent calls)
	if (vadStream) {
		const track = vadStream.getAudioTracks()[0];
		if (!track || track.readyState !== "live") {
			if (!vadReacquiring) {
				vadReacquiring = true;
				console.warn("[voice] VAD: mic track died, reacquiring");
				vadReacquireStream().finally(() => {
					vadReacquiring = false;
				});
			}
			vadRafId = requestAnimationFrame(vadMonitorLoop);
			return;
		}
	}

	// Skip monitoring while TTS is playing or while transcribing
	if (vadMutedForTts || micBtn?.classList.contains("transcribing")) {
		if (vadMutedForTts && !vadMonitorMuteStart) {
			vadMonitorMuteStart = Date.now();
		} else if (vadMutedForTts && Date.now() - vadMonitorMuteStart > 10000) {
			console.debug("[voice] VAD: TTS mute timeout, force-resuming");
			vadMutedForTts = false;
			vadMonitorMuteStart = 0;
			vadSpeechDetected = false;
			vadStartContinuousRecorder();
			vadBtn?.classList.add("vad-listening");
		}
		vadRafId = requestAnimationFrame(vadMonitorLoop);
		return;
	}
	vadMonitorMuteStart = 0;

	// Skip if the session is still replying (waiting for AI response)
	const activeSession = sessionStore.getByKey(S.activeSessionKey);
	if (activeSession?.replying.value) {
		vadRafId = requestAnimationFrame(vadMonitorLoop);
		return;
	}

	// Show listening state when recorder is running
	if (
		vadMediaRecorder &&
		vadMediaRecorder.state === "recording" &&
		vadBtn &&
		!vadBtn.classList.contains("vad-listening") &&
		!vadBtn.classList.contains("vad-speech")
	) {
		vadBtn.classList.add("vad-listening");
	}

	// Restart recorder if it died
	if (!vadTranscribing && (!vadMediaRecorder || vadMediaRecorder.state === "inactive")) {
		vadStartContinuousRecorder();
	}

	const rms = getRMS(vadAnalyser, vadDataArray);
	const now = Date.now();

	if (rms > vadSpeechThreshold) {
		vadSilenceStart = 0;

		// Safety valve: auto-stop after 30s of continuous speech
		if (vadSpeechDetected && vadRecordingStart && now - vadRecordingStart > 30000) {
			console.debug("[voice] VAD: max duration reached, auto-sending");
			vadSilenceStart = 0;
			vadRecordingStart = 0;
			vadBtn?.classList.remove("vad-speech", "vad-listening");
			if (vadMediaRecorder && vadMediaRecorder.state === "recording") {
				vadMediaRecorder.stop();
			}
			vadRafId = requestAnimationFrame(vadMonitorLoop);
			return;
		}

		if (!vadSpeechDetected) {
			if (!vadSpeechStart) {
				vadSpeechStart = now;
			} else if (now - vadSpeechStart >= VAD_DEBOUNCE_SPEECH) {
				vadSpeechDetected = true;
				vadSpeechStart = 0;
				vadRecordingStart = now;
				console.debug("[voice] VAD: speech detected (recorder already running)");
				stopAllAudio();
				vadBtn?.classList.add("vad-speech");
			}
		}
	} else {
		vadSpeechStart = 0;

		if (vadSpeechDetected) {
			if (!vadSilenceStart) {
				vadSilenceStart = now;
			} else if (now - vadSilenceStart >= VAD_SILENCE_DURATION) {
				console.debug("[voice] VAD: silence detected, stopping & sending");
				vadRecordingStart = 0;
				vadSilenceStart = 0;
				vadBtn?.classList.remove("vad-speech", "vad-listening");
				if (vadMediaRecorder && vadMediaRecorder.state === "recording") {
					vadMediaRecorder.stop();
				} else {
					vadSpeechDetected = false;
					audioChunks = [];
				}
			}
		}
	}

	vadRafId = requestAnimationFrame(vadMonitorLoop);
}

// ── TTS mute/unmute for VAD ──────────────────────────────────

function isAnyAudioPlaying(): boolean {
	return Array.from(document.querySelectorAll("audio")).some((a) => !(a.paused || a.ended));
}

function onTtsPlay(e: Event): void {
	if (!vadActive) return;
	if ((e.target as HTMLElement)?.tagName !== "AUDIO") return;
	console.debug("[voice] VAD: TTS playing, muting VAD + stopping recorder");
	vadMutedForTts = true;
	vadBtn?.classList.remove("vad-listening", "vad-speech");
	if (vadMediaRecorder && vadMediaRecorder.state === "recording") {
		vadSpeechDetected = false;
		audioChunks = [];
		const mr = vadMediaRecorder;
		vadMediaRecorder = null;
		mr.stop();
	}
}

function resumeVadAfterTts(): void {
	vadSpeechDetected = false;
	vadSilenceStart = 0;
	vadSpeechStart = 0;
	setTimeout(() => {
		if (!vadActive) return;
		if (isAnyAudioPlaying()) {
			console.debug("[voice] VAD: another audio still playing, staying muted");
			return;
		}
		vadMutedForTts = false;
		if (!vadTranscribing) {
			vadStartContinuousRecorder();
			vadBtn?.classList.add("vad-listening");
		}
	}, 400);
}

function onTtsEnded(e: Event): void {
	if (!vadActive) return;
	if ((e.target as HTMLElement)?.tagName !== "AUDIO") return;
	console.debug("[voice] VAD: TTS ended, resuming VAD after delay");
	resumeVadAfterTts();
}

function onTtsPause(e: Event): void {
	if (!vadActive) return;
	if ((e.target as HTMLElement)?.tagName !== "AUDIO") return;
	const audio = e.target as HTMLAudioElement;
	if (audio.ended || (audio.duration && audio.currentTime >= audio.duration - 0.1)) {
		console.debug("[voice] VAD: TTS paused at end, treating as ended");
		resumeVadAfterTts();
	} else if (!isAnyAudioPlaying()) {
		vadMutedForTts = false;
	}
}

// ── VAD button click ─────────────────────────────────────────

function onVadClick(e: Event): void {
	e.preventDefault();
	if (vadActive) {
		stopVad();
	} else {
		startVad();
	}
}

function onMicKeydown(e: KeyboardEvent): void {
	if (e.key === " " || e.key === "Enter") {
		e.preventDefault();
		onMicClick(e);
	}
}

function onEscapeKeydown(e: KeyboardEvent): void {
	if (e.key === "Escape" && isRecording) {
		e.preventDefault();
		cancelRecording();
		if (vadActive) stopVad();
	}
}

// ── Init / teardown ──────────────────────────────────────────

/** Initialize voice input with the mic button element. */
export function initVoiceInput(btn: HTMLButtonElement | null): void {
	if (!btn) return;

	micBtn = btn;
	checkSttStatus();

	micBtn.addEventListener("click", onMicClick);

	micBtn.addEventListener("keydown", onMicKeydown);
	document.addEventListener("keydown", onEscapeKeydown);

	// PTT: global key handlers
	document.addEventListener("keydown", onPttKeyDown);
	document.addEventListener("keyup", onPttKeyUp);

	window.addEventListener("voice-config-changed", checkSttStatus);
}

/** Initialize VAD (conversation mode) button. */
export function initVadButton(btn: HTMLButtonElement | null): void {
	if (!btn) return;
	vadBtn = btn;
	updateVadButton();
	vadBtn.addEventListener("click", onVadClick);
}

/** Teardown voice input module. */
export function teardownVoiceInput(): void {
	if (vadActive) stopVad();
	if (isRecording && mediaRecorder) {
		mediaRecorder.stop();
	}
	document.removeEventListener("keydown", onEscapeKeydown);
	document.removeEventListener("keydown", onPttKeyDown);
	document.removeEventListener("keyup", onPttKeyUp);
	micBtn?.removeEventListener("keydown", onMicKeydown);
	window.removeEventListener("voice-config-changed", checkSttStatus);
	releaseVoiceLock();
	micBtn = null;
	vadBtn = null;
	mediaRecorder = null;
	vadMediaRecorder = null;
	vadTranscribing = false;
	audioChunks = [];
	isRecording = false;
}

/** Re-check STT status (can be called externally). */
export function refreshVoiceStatus(): void {
	checkSttStatus();
}

/** Update PTT key at runtime. */
export function setPttKey(key: string): void {
	pttKey = key;
	localStorage.setItem("moltis_ptt_key", key);
	console.debug("[voice] PTT key set to:", key);
}

/** Get current PTT key. */
export function getPttKey(): string {
	return pttKey;
}

/** Check if VAD is currently active. */
export function isVadModeActive(): boolean {
	return vadActive;
}

/** Update VAD sensitivity at runtime (0-100). */
export function setVadSensitivity(pct: number): void {
	vadSensitivity = Math.max(0, Math.min(100, pct));
	vadSpeechThreshold = sensitivityToThreshold(vadSensitivity);
	localStorage.setItem("moltis_vad_sensitivity", String(vadSensitivity));
	console.debug("[voice] VAD sensitivity set to:", vadSensitivity, "threshold:", vadSpeechThreshold.toFixed(4));
}

/** Get current VAD sensitivity (0-100). */
export function getVadSensitivity(): number {
	return vadSensitivity;
}
