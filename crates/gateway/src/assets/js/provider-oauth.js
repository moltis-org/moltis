import { sendRpc } from "./helpers.js";

function normalizeOAuthStartResponse(res) {
	var payload = res?.payload;

	if (res?.ok && payload?.alreadyAuthenticated) {
		return {
			status: "already",
		};
	}

	if (res?.ok && payload?.authUrl) {
		return {
			status: "browser",
			authUrl: payload.authUrl,
		};
	}

	if (res?.ok && payload?.deviceFlow) {
		var verificationUrl = payload.verificationUriComplete || payload.verificationUri;
		if (!(verificationUrl && payload.userCode)) {
			return {
				status: "error",
				error: "OAuth device flow response is missing verification data.",
			};
		}
		return {
			status: "device",
			verificationUrl,
			userCode: payload.userCode,
		};
	}

	return {
		status: "error",
		error: res?.error?.message || "Failed to start OAuth",
	};
}

export function startProviderOAuth(providerName) {
	return sendRpc("providers.oauth.start", {
		provider: providerName,
		redirectUri: `${window.location.origin}/auth/callback`,
	}).then(normalizeOAuthStartResponse);
}
