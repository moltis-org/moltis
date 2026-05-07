// ── Phone settings utility functions ─────────────────────────

import { sendRpc } from "./helpers";

export interface PhoneProviderData {
	id: string;
	name: string;
	description?: string;
	type?: string;
	category?: string;
	available?: boolean;
	enabled?: boolean;
	keySource?: string;
	keyPlaceholder?: string;
	keyUrl?: string;
	keyUrlLabel?: string;
	hint?: string;
	settings?: {
		from_number?: string;
		webhook_url?: string;
		connection_id?: string;
	};
}

export interface PhoneProviders {
	providers: PhoneProviderData[];
}

export function fetchPhoneProviders(): Promise<unknown> {
	return sendRpc("phone.providers.all", {});
}

export function togglePhoneProvider(providerId: string, enabled: boolean): Promise<unknown> {
	return sendRpc("phone.provider.toggle", { provider: providerId, enabled });
}

export function savePhoneKey(
	providerId: string,
	accountSid: string,
	authToken: string,
	opts?: { from_number?: string; webhook_url?: string },
): Promise<unknown> {
	return sendRpc("phone.config.save_key", {
		provider: providerId,
		account_sid: accountSid,
		auth_token: authToken,
		from_number: opts?.from_number || "",
		webhook_url: opts?.webhook_url || "",
	});
}

export function savePhoneSettings(
	providerId: string,
	opts?: { from_number?: string; webhook_url?: string },
): Promise<unknown> {
	return sendRpc("phone.config.save_settings", {
		provider: providerId,
		from_number: opts?.from_number || "",
		webhook_url: opts?.webhook_url || "",
	});
}

export function removePhoneKey(providerId: string): Promise<unknown> {
	return sendRpc("phone.config.remove_key", { provider: providerId });
}
