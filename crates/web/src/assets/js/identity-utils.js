// ── Shared identity RPC wrappers and validation ───────────────
//
// Used by page-settings.js and onboarding-view.js.

import { sendRpc } from "./helpers.js";

/**
 * Validate identity fields before submission.
 * @param {string} name - agent name
 * @param {string} userName - user's name
 * @returns {{ valid: true } | { valid: false, error: string }}
 */
export function validateIdentityFields(name, userName) {
	if (!(name.trim() || userName.trim())) {
		return { valid: false, error: "Agent name and your name are required." };
	}
	if (!name.trim()) {
		return { valid: false, error: "Agent name is required." };
	}
	if (!userName.trim()) {
		return { valid: false, error: "Your name is required." };
	}
	return { valid: true };
}

/**
 * Update agent identity fields.
 * @param {object} fields - any subset of { name, emoji, theme, soul, user_name, user_timezone }
 * @returns {Promise} RPC response
 */
export function updateIdentity(fields) {
	return sendRpc("agent.identity.update", fields);
}
