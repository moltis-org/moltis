// ── Shared voice RPC wrappers and helpers ─────────────────────
//
// Used by page-settings.js and onboarding-view.js.

import { sendRpc } from "./helpers";

/**
 * Counterpart IDs between TTS and STT for providers that share an API key.
 * E.g. "elevenlabs" (TTS) <-> "elevenlabs-stt" (STT).
 */
export const VOICE_COUNTERPART_IDS: Record<string, string> = {
	elevenlabs: "elevenlabs-stt",
	"elevenlabs-stt": "elevenlabs",
	"google-tts": "google",
	google: "google-tts",
};

/**
 * Fetch all voice providers (TTS + STT).
 * Resolves with the RPC response; payload has `{ tts: [], stt: [] }`.
 */
export function fetchVoiceProviders(): Promise<unknown> {
	return sendRpc("voice.providers.all", {});
}

/**
 * Toggle a voice provider on or off.
 */
export function toggleVoiceProvider(providerId: string, enabled: boolean, type: string): Promise<unknown> {
	return sendRpc("voice.provider.toggle", { provider: providerId, enabled, type });
}

interface SaveVoiceKeyOptions {
	voice?: string;
	model?: string;
	languageCode?: string;
	baseUrl?: string;
}

/**
 * Save an API key (and optional settings) for a voice provider.
 */
interface VoiceKeyPayload {
	provider: string;
	api_key: string;
	voice?: string;
	voiceId?: string;
	model?: string;
	languageCode?: string;
	baseUrl?: string;
}

export function saveVoiceKey(providerId: string, apiKey: string, opts?: SaveVoiceKeyOptions): Promise<unknown> {
	const payload: VoiceKeyPayload = { provider: providerId, api_key: apiKey };
	if (opts?.voice) {
		payload.voice = opts.voice;
		payload.voiceId = opts.voice;
	}
	if (opts?.model) payload.model = opts.model;
	if (opts?.languageCode) payload.languageCode = opts.languageCode;
	if (typeof opts?.baseUrl === "string") payload.baseUrl = opts.baseUrl;
	return sendRpc("voice.config.save_key", payload);
}

interface SaveVoiceSettingsOptions {
	voice?: string;
	model?: string;
	languageCode?: string;
	baseUrl?: string;
}

/**
 * Save non-secret voice provider settings.
 */
interface VoiceSettingsPayload {
	provider: string;
	voice?: string;
	voiceId?: string;
	model?: string;
	languageCode?: string;
	baseUrl?: string;
}

export function saveVoiceSettings(providerId: string, opts?: SaveVoiceSettingsOptions): Promise<unknown> {
	const payload: VoiceSettingsPayload = { provider: providerId };
	if (opts?.voice) {
		payload.voice = opts.voice;
		payload.voiceId = opts.voice;
	}
	if (opts?.model) payload.model = opts.model;
	if (opts?.languageCode) payload.languageCode = opts.languageCode;
	if (typeof opts?.baseUrl === "string") payload.baseUrl = opts.baseUrl;
	return sendRpc("voice.config.save_settings", payload);
}

/**
 * Convert text to speech via a given provider.
 */
export function testTts(text: string, providerId: string): Promise<unknown> {
	return sendRpc("tts.convert", { text, provider: providerId });
}

/**
 * Upload an audio blob for STT transcription.
 * Returns raw fetch Response.
 */
export function transcribeAudio(sessionKey: string, providerId: string, audioBlob: Blob): Promise<Response> {
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
export function decodeBase64Safe(input: string | null | undefined): Uint8Array {
	if (!input) return new Uint8Array();
	let normalized = String(input).replace(/\s+/g, "").replace(/-/g, "+").replace(/_/g, "/");
	while (normalized.length % 4) normalized += "=";
	let binary = "";
	try {
		binary = atob(normalized);
	} catch (_err) {
		throw new Error("Invalid base64 audio payload");
	}
	const bytes = new Uint8Array(binary.length);
	for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
	return bytes;
}
