// ── WebAuthn type bridge helpers ──────────────────────────────
//
// The WebAuthn server sends JSON with base64-encoded strings for fields
// that the browser Credential Management API expects as ArrayBuffers
// (challenge, user.id, credential ids). These helpers convert the server
// JSON to the browser API format, centralizing the one unavoidable type
// assertion needed to bridge Record<string, unknown> and the specific
// PublicKeyCredential*Options types.

/**
 * Decode a base64url-encoded string to an ArrayBuffer.
 * Handles both standard base64 and base64url (RFC 4648 §5) encoding.
 */
export function base64ToArrayBuffer(b64: string): ArrayBuffer {
	let str = b64.replace(/-/g, "+").replace(/_/g, "/");
	while (str.length % 4) str += "=";
	const bin = atob(str);
	const buf = new Uint8Array(bin.length);
	for (let i = 0; i < bin.length; i++) buf[i] = bin.charCodeAt(i);
	return buf.buffer as ArrayBuffer;
}

/**
 * Convert server-provided creation options JSON to the browser
 * PublicKeyCredentialCreationOptions format.
 *
 * Mutates the publicKey object in-place, converting base64 strings
 * to ArrayBuffers for challenge, user.id, and excludeCredentials[].id.
 */
export function prepareCreationOptions(serverPk: Record<string, unknown>): PublicKeyCredentialCreationOptions {
	serverPk.challenge = base64ToArrayBuffer(serverPk.challenge as string);
	const user = serverPk.user as Record<string, unknown>;
	user.id = base64ToArrayBuffer(user.id as string);
	if (serverPk.excludeCredentials) {
		for (const c of serverPk.excludeCredentials as Array<Record<string, unknown>>) {
			c.id = base64ToArrayBuffer(c.id as string);
		}
	}
	// Bridge assertion: server JSON fields have been converted to ArrayBuffer
	// above, satisfying the PublicKeyCredentialCreationOptions contract.
	return serverPk as unknown as PublicKeyCredentialCreationOptions;
}

/**
 * Convert server-provided request options JSON to the browser
 * PublicKeyCredentialRequestOptions format.
 *
 * Mutates the publicKey object in-place, converting base64 strings
 * to ArrayBuffers for challenge and allowCredentials[].id.
 */
export function prepareRequestOptions(serverPk: Record<string, unknown>): PublicKeyCredentialRequestOptions {
	serverPk.challenge = base64ToArrayBuffer(serverPk.challenge as string);
	if (serverPk.allowCredentials) {
		for (const c of serverPk.allowCredentials as Array<Record<string, unknown>>) {
			c.id = base64ToArrayBuffer(c.id as string);
		}
	}
	// Bridge assertion: server JSON fields have been converted to ArrayBuffer
	// above, satisfying the PublicKeyCredentialRequestOptions contract.
	return serverPk as unknown as PublicKeyCredentialRequestOptions;
}
