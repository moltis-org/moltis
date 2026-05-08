// ── Shared identity RPC wrappers and validation ───────────────
//
// Used by page-settings.js and onboarding-view.js.

import { sendRpc } from "./helpers";

interface ValidationSuccess {
	valid: true;
}

interface ValidationFailure {
	valid: false;
	error: string;
}

export type ValidationResult = ValidationSuccess | ValidationFailure;

export interface IdentityFields {
	name?: string;
	emoji?: string;
	theme?: string;
	soul?: string;
	user_name?: string;
	user_timezone?: string;
}

import type { RpcResponse } from "./types/rpc";

interface UpdateIdentityOptions {
	agentId?: string;
}

/**
 * Validate identity fields before submission.
 */
export function validateIdentityFields(name: string, userName: string): ValidationResult {
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

function isMissingMethodError(res: RpcResponse | null | undefined): boolean {
	const message = res?.error?.message;
	if (typeof message !== "string") return false;
	const lower = message.toLowerCase();
	return lower.includes("method") && (lower.includes("not found") || lower.includes("unknown"));
}

/**
 * Update agent identity fields.
 */
export function updateIdentity(fields: IdentityFields, options: UpdateIdentityOptions = {}): Promise<RpcResponse> {
	const agentId = options.agentId;
	if (!agentId) {
		return sendRpc("agent.identity.update", fields);
	}
	const params = { ...fields, agent_id: agentId };
	return sendRpc("agents.identity.update", params).then((res) => {
		if (res?.ok || !isMissingMethodError(res as RpcResponse)) return res as RpcResponse;
		return sendRpc("agent.identity.update", fields) as unknown as Promise<RpcResponse>;
	}) as unknown as Promise<RpcResponse>;
}
