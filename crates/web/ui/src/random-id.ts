let fallbackCounter = 0;

export function randomId(): string {
	const webCrypto = globalThis.crypto;
	try {
		if (typeof webCrypto?.randomUUID === "function") return webCrypto.randomUUID();
		if (typeof webCrypto?.getRandomValues === "function") {
			const bytes = webCrypto.getRandomValues(new Uint8Array(16));
			return Array.from(bytes, (byte) => byte.toString(16).padStart(2, "0")).join("");
		}
	} catch {
		// Fall back to a page-local unique ID if Web Crypto is unavailable at runtime.
	}

	fallbackCounter += 1;
	return `${Date.now().toString(36)}-${fallbackCounter.toString(36)}`;
}
