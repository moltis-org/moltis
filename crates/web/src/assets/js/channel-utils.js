// ── Shared channel RPC wrappers and validation ────────────────
//
// Used by page-channels.js and onboarding-view.js.

import { sendRpc } from "./helpers.js";

/**
 * Validate required channel fields before submission.
 * @param {string} accountId - bot username
 * @param {string} token - bot token
 * @returns {{ valid: true } | { valid: false, error: string }}
 */
export function validateChannelFields(accountId, token) {
	if (!accountId.trim()) {
		return { valid: false, error: "Bot username is required." };
	}
	if (!token.trim()) {
		return { valid: false, error: "Bot token is required." };
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
