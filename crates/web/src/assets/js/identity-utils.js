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

function isMissingMethodError(res) {
	var message = res?.error?.message;
	if (typeof message !== "string") return false;
	var lower = message.toLowerCase();
	return lower.includes("method") && (lower.includes("not found") || lower.includes("unknown"));
}

/**
 * Update agent identity fields.
 * @param {object} fields - any subset of { name, emoji, theme, soul, user_name, user_timezone }
 * @param {{ agentId?: string }} [options] - optional explicit agent target
 * @returns {Promise} RPC response
 */
export function updateIdentity(fields, options = {}) {
	var agentId = options.agentId;
	if (!agentId) {
		return sendRpc("agent.identity.update", fields);
	}
	var params = { ...fields, agent_id: agentId };
	return sendRpc("agents.identity.update", params).then((res) => {
		if (res?.ok || !isMissingMethodError(res)) return res;
		return sendRpc("agent.identity.update", fields);
	});
}
