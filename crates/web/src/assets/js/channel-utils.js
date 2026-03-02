// ── Shared channel RPC wrappers and validation ────────────────
//
// Used by page-channels.js and onboarding-view.js.

import { sendRpc } from "./helpers.js";

/**
 * Validate required channel fields before submission.
 * @param {string} type - channel type
 * @param {string} accountId - account identifier
 * @param {string} credential - primary credential (token or app password)
 * @returns {{ valid: true } | { valid: false, error: string }}
 */
export function validateChannelFields(type, accountId, credential) {
	if (!accountId.trim()) {
		return { valid: false, error: "Account ID is required." };
	}
	if (!credential.trim()) {
		return {
			valid: false,
			error: type === "msteams" ? "App password is required." : "Bot token is required.",
		};
	}
	return { valid: true };
}

/**
 * Add a new channel (e.g. Telegram bot).
 * @param {string} type - channel type, e.g. "telegram"
 * @param {string} accountId - bot username / account identifier
 * @param {object} config - channel-specific config (token, dm_policy, etc.)
 */
export function addChannel(type, accountId, config) {
	return sendRpc("channels.add", { type, account_id: accountId, config });
}

/**
 * Fetch the current status of all configured channels.
 * Resolves with the RPC response; payload has `{ channels: [] }`.
 */
export function fetchChannelStatus() {
	return sendRpc("channels.status", {});
}

/**
 * Default base URL for Teams webhook endpoints (current page origin).
 */
export function defaultTeamsBaseUrl() {
	if (typeof window === "undefined") return "";
	return window.location?.origin || "";
}

/**
 * Normalise a user-provided base URL into `protocol://host`.
 */
export function normalizeBaseUrlForWebhook(baseUrl) {
	var raw = (baseUrl || "").trim();
	if (!raw) raw = defaultTeamsBaseUrl();
	if (!raw) return "";
	if (!/^https?:\/\//i.test(raw)) raw = `https://${raw}`;
	try {
		var parsed = new URL(raw);
		return `${parsed.protocol}//${parsed.host}`;
	} catch (_e) {
		return "";
	}
}

/**
 * Generate a random 48-hex-char webhook secret.
 */
export function generateWebhookSecretHex() {
	if (typeof window !== "undefined" && window.crypto?.getRandomValues) {
		var bytes = new Uint8Array(24);
		window.crypto.getRandomValues(bytes);
		return Array.from(bytes, (b) => b.toString(16).padStart(2, "0")).join("");
	}
	var value = "";
	while (value.length < 48) {
		value += Math.floor(Math.random() * 16).toString(16);
	}
	return value.slice(0, 48);
}

/**
 * Build the full Teams messaging endpoint URL.
 */
export function buildTeamsEndpoint(baseUrl, accountId, webhookSecret) {
	var normalizedBase = normalizeBaseUrlForWebhook(baseUrl);
	var account = (accountId || "").trim();
	var secret = (webhookSecret || "").trim();
	if (!(normalizedBase && account && secret)) return "";
	return `${normalizedBase}/api/channels/msteams/${encodeURIComponent(account)}/webhook?secret=${encodeURIComponent(secret)}`;
}
