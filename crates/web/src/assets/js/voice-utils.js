// ── Shared voice RPC wrappers and helpers ─────────────────────
//
// Used by page-settings.js and onboarding-view.js.

import { sendRpc } from "./helpers.js";

/**
 * Counterpart IDs between TTS and STT for providers that share an API key.
 * E.g. "elevenlabs" (TTS) ↔ "elevenlabs-stt" (STT).
 */
export var VOICE_COUNTERPART_IDS = {
	elevenlabs: "elevenlabs-stt",
	"elevenlabs-stt": "elevenlabs",
	"google-tts": "google",
	google: "google-tts",
};

/**
 * Fetch all voice providers (TTS + STT).
 * Resolves with the RPC response; payload has `{ tts: [], stt: [] }`.
 */
export function fetchVoiceProviders() {
	return sendRpc("voice.providers.all", {});
}

/**
 * Toggle a voice provider on or off.
 * @param {string} providerId - e.g. "elevenlabs"
 * @param {boolean} enabled
 * @param {string} type - "tts" | "stt"
 */
export function toggleVoiceProvider(providerId, enabled, type) {
	return sendRpc("voice.provider.toggle", { provider: providerId, enabled, type });
}

/**
 * Save an API key (and optional settings) for a voice provider.
 * @param {string} providerId
 * @param {string} apiKey
 * @param {object} [opts] - Optional TTS settings: voice, model, languageCode
 */
export function saveVoiceKey(providerId, apiKey, opts) {
	var payload = { provider: providerId, api_key: apiKey };
	if (opts?.voice) {
		payload.voice = opts.voice;
		payload.voiceId = opts.voice;
	}
	if (opts?.model) payload.model = opts.model;
	if (opts?.languageCode) payload.languageCode = opts.languageCode;
	return sendRpc("voice.config.save_key", payload);
}

/**
 * Convert text to speech via a given provider.
 * @param {string} text
 * @param {string} providerId
 */
export function testTts(text, providerId) {
	return sendRpc("tts.convert", { text, provider: providerId });
}

/**
 * Upload an audio blob for STT transcription.
 * @param {string} sessionKey - active session key from state
 * @param {string} providerId
 * @param {Blob} audioBlob
 * @returns {Promise<Response>} raw fetch Response
 */
export function transcribeAudio(sessionKey, providerId, audioBlob) {
	return fetch(
		`/api/sessions/${encodeURIComponent(sessionKey)}/upload?transcribe=true&provider=${encodeURIComponent(providerId)}`,
		{
			method: "POST",
			headers: { "Content-Type": audioBlob.type || "audio/webm" },
			body: audioBlob,
		},
	);
}

/**
 * Decode a base64 (or base64url) string to a Uint8Array, tolerating
 * whitespace, URL-safe characters, and missing padding.
 */
export function decodeBase64Safe(input) {
	if (!input) return new Uint8Array();
	var normalized = String(input).replace(/\s+/g, "").replace(/-/g, "+").replace(/_/g, "/");
	while (normalized.length % 4) normalized += "=";
	var binary = "";
	try {
		binary = atob(normalized);
	} catch (_err) {
		throw new Error("Invalid base64 audio payload");
	}
	var bytes = new Uint8Array(binary.length);
	for (var i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
	return bytes;
}
