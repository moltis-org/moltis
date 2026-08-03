export function randomId(): string {
	const webCrypto = globalThis.crypto;
	if (typeof webCrypto.randomUUID === "function") return webCrypto.randomUUID();

	const bytes = webCrypto.getRandomValues(new Uint8Array(16));
	return Array.from(bytes, (byte) => byte.toString(16).padStart(2, "0")).join("");
}
