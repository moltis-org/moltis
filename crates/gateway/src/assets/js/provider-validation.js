import { sendRpc } from "./helpers.js";

function firstProbeFailure(payload) {
	var results = Array.isArray(payload?.results) ? payload.results : [];
	var failed = results.find((r) => r?.status === "error" || r?.status === "unsupported");
	if (!failed) return null;
	if (typeof failed.error === "string" && failed.error.trim()) {
		return failed.error.trim();
	}
	return null;
}

export async function validateProviderConnection(providerName) {
	var res = await sendRpc("models.detect_supported", {
		provider: providerName,
		reason: "provider_credentials_validation",
	});

	if (!res?.ok) {
		return {
			ok: false,
			message: res?.error?.message || "Failed to validate provider credentials.",
		};
	}

	var payload = res.payload || {};
	var total = payload.total || 0;
	var supported = payload.supported || 0;
	var unsupported = payload.unsupported || 0;
	var errors = payload.errors || 0;

	if (supported > 0) {
		return {
			ok: true,
			message: null,
		};
	}

	// No probe targets usually means no model is configured yet.
	if (total === 0) {
		return {
			ok: true,
			message: null,
		};
	}

	var reason = firstProbeFailure(payload);
	if (!reason) {
		reason = `0/${total} models responded successfully (unsupported: ${unsupported}, errors: ${errors}).`;
	}

	return {
		ok: false,
		message: `Credentials were saved, but validation failed: ${reason}`,
	};
}
